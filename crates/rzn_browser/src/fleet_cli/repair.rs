//! Local workflow fixes become fleet repair candidates (backend `docs/plans/browser-fleet/`
//! spec section 7, BFL-T-0019).
//!
//! A person or an agent fixes a broken workflow by editing a copy in the user workflow
//! directory. That copy is a *fork*: it shadows the fleet catalog's version (or the bundled
//! builtin) for local runs. `rzn-browser fleet propose <id> --reason "…"` sends the fork's
//! manifest, the hash of the version it was made from, the reason and its last local run to the
//! fleet server as a repair candidate. Offline, the proposal stays queued in
//! `<app_base>/fleet_proposals.json` and the supervisor sends it after its next check-in.
//!
//! After a catalog pull (and on `rzn-browser fleet forks`) [`tidy`] moves a fork to
//! `<runtime>/workflows/trash/<ms>/` when it is the same as the version it shadows, or when the
//! catalog released it (its hash was current in a synced catalog after it was proposed). Every
//! other shadowing fork is listed with its diff and the propose command.
//!
//! Forks are matched by the manifest's `id`, the catalog key.

use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rzn_contracts::fleet::{manifest_diff, FleetRepairCandidate, FleetRepairCandidateAck};
use rzn_contracts::workflow::RunResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::fleet_catalog::{HeldCatalog, HELD_CATALOG_FILENAME};
use crate::workflow_cache::manifest_content_hash;

pub(crate) const PROPOSALS_FILENAME: &str = "fleet_proposals.json";
const MANIFEST_SCHEMA: &str = "rzn.workflow_manifest";

/// The directories a fork is looked up in.
#[derive(Debug, Clone)]
pub(crate) struct Dirs {
    pub user: PathBuf,
    pub fleet: PathBuf,
    pub builtin: PathBuf,
    pub trash: PathBuf,
}

impl Dirs {
    pub(crate) fn installed() -> Self {
        let roots = crate::workflow_catalog::workflow_roots();
        Self {
            trash: roots.runtime_dir.join("workflows").join("trash"),
            fleet: crate::workflow_catalog::fleet_workflows_dir(),
            user: roots.user_dir,
            builtin: roots.builtin_dir,
        }
    }
}

/// A user workflow and the fleet or builtin version it shadows, if any.
#[derive(Debug, Clone)]
pub(crate) struct Fork {
    pub workflow_id: String,
    pub path: PathBuf,
    pub manifest: Value,
    pub hash: String,
    /// `(source, manifest, hash)` of the shadowed version: the fleet copy, else the builtin.
    pub base: Option<(&'static str, Value, String)>,
}

impl Fork {
    pub(crate) fn diff(&self) -> Vec<String> {
        self.base
            .as_ref()
            .map(|(_, base, _)| manifest_diff(base, &self.manifest))
            .unwrap_or_default()
    }
}

/// Every manifest under `root` by its `id` (first file wins).
fn manifests_by_id(root: &Path) -> HashMap<String, (PathBuf, Value)> {
    let mut files = Vec::new();
    collect_json(root, &mut files);
    files.sort();
    let mut out = HashMap::new();
    for path in files {
        let Some(value) = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        else {
            continue;
        };
        if value["schema_version"].as_str() != Some(MANIFEST_SCHEMA) {
            continue;
        }
        if let Some(id) = value["id"].as_str() {
            out.entry(id.to_string()).or_insert((path, value));
        }
    }
    out
}

fn collect_json(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            out.push(path);
        }
    }
}

/// Every user workflow, with the version it shadows.
pub(crate) fn forks(dirs: &Dirs) -> Vec<Fork> {
    let user = manifests_by_id(&dirs.user);
    if user.is_empty() {
        return Vec::new();
    }
    let fleet = manifests_by_id(&dirs.fleet);
    let builtin = manifests_by_id(&dirs.builtin);
    let mut forks: Vec<Fork> = user
        .into_iter()
        .map(|(workflow_id, (path, manifest))| {
            let base = fleet
                .get(&workflow_id)
                .map(|(_, m)| ("fleet", m))
                .or_else(|| builtin.get(&workflow_id).map(|(_, m)| ("builtin", m)))
                .map(|(source, m)| (source, m.clone(), manifest_content_hash(m)));
            Fork {
                hash: manifest_content_hash(&manifest),
                workflow_id,
                path,
                manifest,
                base,
            }
        })
        .collect();
    forks.sort_by(|a, b| a.workflow_id.cmp(&b.workflow_id));
    forks
}

// ---------------------------------------------------------------------------
// Proposals queue
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct Proposals {
    #[serde(default)]
    pub entries: Vec<Proposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct Proposal {
    pub workflow_id: String,
    /// Content hash of the proposed manifest.
    pub fork_hash: String,
    /// Set once the server acknowledged it.
    #[serde(default)]
    pub candidate_id: Option<String>,
    /// A synced catalog named `fork_hash` as the workflow's current version.
    #[serde(default)]
    pub released: bool,
    /// Waiting to be sent; dropped once the server acknowledged it.
    #[serde(default)]
    pub pending: Option<FleetRepairCandidate>,
}

impl Proposals {
    pub(crate) fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub(crate) fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, path)
    }

    /// Queue `candidate`, replacing an unsent proposal of the same workflow.
    fn queue(&mut self, candidate: FleetRepairCandidate, fork_hash: String) {
        self.entries
            .retain(|p| !(p.workflow_id == candidate.workflow_id && p.pending.is_some()));
        self.entries.push(Proposal {
            workflow_id: candidate.workflow_id.clone(),
            fork_hash,
            candidate_id: None,
            released: false,
            pending: Some(candidate),
        });
    }

    fn status_of(&self, fork: &Fork) -> Option<String> {
        let p = self
            .entries
            .iter()
            .rev()
            .find(|p| p.workflow_id == fork.workflow_id && p.fork_hash == fork.hash)?;
        Some(match &p.candidate_id {
            Some(id) => format!("proposed as candidate {id}"),
            None => "proposal queued; sent at the next check-in".to_string(),
        })
    }
}

/// Send every queued proposal through `post`; an acknowledged one keeps only its candidate id.
/// Returns the errors of the ones still queued.
pub(crate) async fn send_pending<F, Fut>(path: &Path, post: F) -> Vec<String>
where
    F: Fn(FleetRepairCandidate) -> Fut,
    Fut: Future<Output = Result<FleetRepairCandidateAck, String>>,
{
    let mut proposals = Proposals::load(path);
    let mut errors = Vec::new();
    let mut changed = false;
    for entry in proposals.entries.iter_mut() {
        let Some(candidate) = entry.pending.clone() else {
            continue;
        };
        match post(candidate).await {
            Ok(ack) => {
                entry.candidate_id = Some(ack.candidate_id);
                entry.pending = None;
                changed = true;
            }
            Err(err) => errors.push(format!("{}: {err}", entry.workflow_id)),
        }
    }
    if changed {
        if let Err(err) = proposals.save(path) {
            errors.push(format!("save {}: {err}", path.display()));
        }
    }
    errors
}

/// Send the queued proposals with this device's fleet credentials. Called by the supervisor
/// after each check-in and by `propose`. Nothing queued or not enrolled is a no-op.
pub(crate) async fn send_queued(app_base: &Path) -> Vec<String> {
    let path = app_base.join(PROPOSALS_FILENAME);
    if !Proposals::load(&path)
        .entries
        .iter()
        .any(|p| p.pending.is_some())
    {
        return Vec::new();
    }
    let (server_url, token) = match super::device_credentials() {
        Ok(Some(credentials)) => credentials,
        Ok(None) => return vec!["this device is not enrolled in a fleet".to_string()],
        Err(err) => return vec![format!("{err:#}")],
    };
    let client = match super::fleet_http_client() {
        Ok(client) => client,
        Err(err) => return vec![format!("{err:#}")],
    };
    let url = format!("{}/v1/fleet/candidates", server_url.trim_end_matches('/'));
    send_pending(&path, |candidate| {
        let request = client.post(&url).bearer_auth(&token).json(&candidate);
        async move {
            let response = request.send().await.map_err(|e| e.to_string())?;
            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(format!("status {status}: {body}"));
            }
            response
                .json::<FleetRepairCandidateAck>()
                .await
                .map_err(|e| e.to_string())
        }
    })
    .await
}

// ---------------------------------------------------------------------------
// Propose
// ---------------------------------------------------------------------------

/// Build the candidate for the user's copy of `workflow_id`: refuse a manifest the contract
/// rejects, or an item workflow whose last successful run's output fails the item check.
pub(crate) fn build_candidate(
    fork: &Fork,
    reason: &str,
    last_run: Option<RunResult>,
) -> Result<FleetRepairCandidate> {
    if reason.trim().is_empty() {
        bail!("--reason must say what broke and what changed");
    }
    if let Err(issues) = rzn_contracts::workflow::validate_manifest_value(&fork.manifest) {
        let text: Vec<String> = issues
            .iter()
            .map(|i| format!("{}: {}", i.field, i.message))
            .collect();
        bail!(
            "{} is not a valid workflow: {}",
            fork.path.display(),
            text.join("; ")
        );
    }
    let Some(contract) = crate::workflow_contract_lint::declared_contract(&fork.manifest) else {
        bail!("declare result.contract (rzn.item/v2, rzn.items/v2 or none) before proposing");
    };
    if contract.is_item() {
        let output = last_run
            .as_ref()
            .filter(|r| r.status == rzn_contracts::workflow::RunStatus::Succeeded)
            .and_then(|r| r.output.as_ref());
        let Some(output) = output else {
            bail!(
                "{} returns {}: run it live first (rzn-browser run {}) so its output can be checked",
                fork.workflow_id,
                contract.as_str(),
                fork.path.display()
            );
        };
        if let Err(errors) = rzn_core::item_contract::check_output(contract, output) {
            bail!(
                "the last run's output fails {}: {}",
                contract.as_str(),
                errors.join("; ")
            );
        }
    }
    Ok(FleetRepairCandidate {
        workflow_id: fork.workflow_id.clone(),
        manifest: fork.manifest.clone(),
        base_hash: fork.base.as_ref().map(|(_, _, hash)| hash.clone()),
        reason: reason.trim().to_string(),
        last_run,
    })
}

/// The newest local run of `workflow_id`.
fn last_run(workflow_id: &str) -> Option<RunResult> {
    let store =
        crate::run_store::RunStore::open(rzn_core::runtime_paths::default_app_base_dir()).ok()?;
    let (_, rows) = store
        .list(crate::run_store::RunListFilter {
            limit: 1,
            workflow_id: Some(workflow_id.to_string()),
            ..Default::default()
        })
        .ok()?;
    let row = rows.into_iter().next()?;
    store
        .get(&row.run_id)
        .ok()
        .flatten()
        .map(|(_, result)| result)
}

pub(crate) async fn run_propose(workflow_id: &str, reason: &str) -> Result<()> {
    let dirs = Dirs::installed();
    let Some(fork) = forks(&dirs)
        .into_iter()
        .find(|f| f.workflow_id == workflow_id)
    else {
        bail!(
            "no workflow with id {workflow_id} in {}; keep the fixed copy there (a user fork) and propose it",
            dirs.user.display()
        );
    };
    let run = last_run(workflow_id);
    if run.is_none() {
        eprintln!("warning: {workflow_id} has no local run; the candidate carries none");
    }
    let candidate = build_candidate(&fork, reason, run)?;
    let path = rzn_core::runtime_paths::default_app_base_dir().join(PROPOSALS_FILENAME);
    let mut proposals = Proposals::load(&path);
    proposals.queue(candidate, fork.hash.clone());
    proposals
        .save(&path)
        .with_context(|| format!("save {}", path.display()))?;
    let errors = send_queued(&rzn_core::runtime_paths::default_app_base_dir()).await;
    let proposals = Proposals::load(&path);
    match proposals.status_of(&fork) {
        Some(status) if errors.is_empty() => println!("{workflow_id}: {status}"),
        _ => println!(
            "{workflow_id}: queued ({}); the supervisor sends it after its next check-in",
            errors.join("; ")
        ),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tidy after a pull
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub(crate) struct Tidy {
    /// `(workflow_id, why, where it went)`.
    pub trashed: Vec<(String, String, PathBuf)>,
    /// Shadowing forks left in place.
    pub kept: Vec<Fork>,
}

/// Mark proposals the held catalog released, move retired forks to the trash, and return the
/// shadowing forks that remain.
pub(crate) fn tidy(dirs: &Dirs, held_path: &Path, proposals_path: &Path, now_ms: i64) -> Tidy {
    let held = HeldCatalog::load(held_path);
    let mut proposals = Proposals::load(proposals_path);
    let mut changed = false;
    for p in proposals.entries.iter_mut().filter(|p| !p.released) {
        if held
            .workflows
            .iter()
            .any(|e| e.workflow_id == p.workflow_id && e.content_hash == p.fork_hash)
        {
            p.released = true;
            changed = true;
        }
    }

    let mut report = Tidy::default();
    for fork in forks(dirs) {
        let Some((source, _, base_hash)) = &fork.base else {
            continue; // the user's own workflow, shadowing nothing
        };
        let why =
            if *base_hash == fork.hash {
                format!("same as the {source} version")
            } else if proposals.entries.iter().any(|p| {
                p.released && p.workflow_id == fork.workflow_id && p.fork_hash == fork.hash
            }) {
                "released to the fleet catalog".to_string()
            } else {
                report.kept.push(fork);
                continue;
            };
        match move_to_trash(&dirs.user, &dirs.trash, &fork.path, now_ms) {
            Ok(to) => {
                proposals
                    .entries
                    .retain(|p| !(p.workflow_id == fork.workflow_id && p.fork_hash == fork.hash));
                changed = true;
                report.trashed.push((fork.workflow_id, why, to));
            }
            Err(err) => {
                tracing::warn!(workflow_id = %fork.workflow_id, "fork not moved to trash: {err}");
                report.kept.push(fork);
            }
        }
    }
    if changed {
        if let Err(err) = proposals.save(proposals_path) {
            tracing::warn!("save {}: {err}", proposals_path.display());
        }
    }
    report
}

/// Move `file` to `<trash>/<now_ms>/<its path under user>`; never deletes.
fn move_to_trash(user: &Path, trash: &Path, file: &Path, now_ms: i64) -> io::Result<PathBuf> {
    let relative = file.strip_prefix(user).map_err(io::Error::other)?;
    let to = trash.join(now_ms.to_string()).join(relative);
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(file, &to)?;
    Ok(to)
}

pub(crate) fn propose_command(workflow_id: &str) -> String {
    format!("rzn-browser fleet propose {workflow_id} --reason \"<what broke; what changed>\"")
}

/// After a catalog sync: tidy and log what remains (the supervisor's half of A4).
pub(crate) fn tidy_after_pull(dirs: &Dirs, app_base: &Path) {
    let report = tidy(
        dirs,
        &app_base.join(HELD_CATALOG_FILENAME),
        &app_base.join(PROPOSALS_FILENAME),
        now_ms(),
    );
    for (id, why, to) in &report.trashed {
        tracing::info!(workflow_id = %id, to = %to.display(), "user fork moved to trash: {why}");
    }
    for fork in &report.kept {
        tracing::warn!(
            workflow_id = %fork.workflow_id,
            changes = fork.diff().len(),
            "user fork shadows the {} version; see `rzn-browser fleet forks`, or `{}`",
            fork.base.as_ref().map_or("", |b| b.0),
            propose_command(&fork.workflow_id)
        );
    }
}

/// `rzn-browser fleet forks`: tidy, then print each remaining shadowing fork with its diff.
pub(crate) fn run_forks(json: bool) -> Result<()> {
    let app_base = rzn_core::runtime_paths::default_app_base_dir();
    let proposals_path = app_base.join(PROPOSALS_FILENAME);
    let report = tidy(
        &Dirs::installed(),
        &app_base.join(HELD_CATALOG_FILENAME),
        &proposals_path,
        now_ms(),
    );
    let proposals = Proposals::load(&proposals_path);
    if json {
        let value = serde_json::json!({
            "trashed": report.trashed.iter().map(|(id, why, to)| serde_json::json!({
                "workflow_id": id, "why": why, "moved_to": to })).collect::<Vec<_>>(),
            "forks": report.kept.iter().map(|f| serde_json::json!({
                "workflow_id": f.workflow_id,
                "path": f.path,
                "shadows": f.base.as_ref().map(|b| b.0),
                "base_hash": f.base.as_ref().map(|b| &b.2),
                "fork_hash": f.hash,
                "diff": f.diff(),
                "proposal": proposals.status_of(f),
                "propose": propose_command(&f.workflow_id),
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    for (id, why, to) in &report.trashed {
        println!("moved {id} to {} ({why})", to.display());
    }
    if report.kept.is_empty() {
        println!("no user workflow shadows a fleet or builtin version");
    }
    for fork in &report.kept {
        let diff = fork.diff();
        println!(
            "\n{} ({}) shadows the {} version; {} change(s):",
            fork.workflow_id,
            fork.path.display(),
            fork.base.as_ref().map_or("", |b| b.0),
            diff.len()
        );
        for line in diff.iter().take(40) {
            println!("  {line}");
        }
        if diff.len() > 40 {
            println!("  … {} more (--json for all)", diff.len() - 40);
        }
        match proposals.status_of(fork) {
            Some(status) => println!("  {status}"),
            None => println!("  propose: {}", propose_command(&fork.workflow_id)),
        }
    }
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rzn_contracts::fleet::FleetCatalogEntry;
    use serde_json::json;
    use std::sync::Mutex;

    fn temp(tag: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("rzn-fleet-repair-{tag}-{n}"))
    }

    fn dirs(root: &Path) -> Dirs {
        Dirs {
            user: root.join("user"),
            fleet: root.join("fleet"),
            builtin: root.join("builtin"),
            trash: root.join("trash"),
        }
    }

    fn smoke(id: &str, version: &str) -> Value {
        let mut m: Value = serde_json::from_str(include_str!(
            "../../../../workflows/_smoke/fleet-tier1.json"
        ))
        .unwrap();
        m["id"] = json!(id);
        m["version"] = json!(version);
        m
    }

    fn write(path: &Path, value: &Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn hold(path: &Path, id: &str, hash: &str) {
        let held = HeldCatalog {
            catalog_hash: Some("c".into()),
            workflows: vec![FleetCatalogEntry {
                workflow_id: id.into(),
                content_hash: hash.into(),
                status: "active".into(),
                paused_reason: None,
                system: "s".into(),
            }],
        };
        fs::write(path, serde_json::to_vec(&held).unwrap()).unwrap();
    }

    /// A4: after a pull, a fork equal to the version it shadows, or one the catalog released, is
    /// moved to the trash (not deleted); any other shadowing fork is listed with its diff.
    #[test]
    fn tidy_trashes_equal_and_released_forks_and_lists_the_rest() {
        let root = temp("tidy");
        let d = dirs(&root);
        let (held, props) = (root.join("held.json"), root.join("proposals.json"));
        // s/same: user copy equals the builtin (file names differ, ids match).
        write(&d.builtin.join("s/s_same.json"), &smoke("s/same", "1"));
        write(&d.user.join("s/same.json"), &smoke("s/same", "1"));
        // s/fixed: user fix of the fleet version, proposed; s/other: unproposed fix.
        write(&d.fleet.join("s/fixed.json"), &smoke("s/fixed", "1"));
        write(&d.builtin.join("s/fixed.json"), &smoke("s/fixed", "0"));
        let fixed = smoke("s/fixed", "2");
        write(&d.user.join("s/fixed.json"), &fixed);
        write(&d.builtin.join("s/other.json"), &smoke("s/other", "1"));
        write(&d.user.join("s/other.json"), &smoke("s/other", "9"));
        // s/mine: the user's own workflow, shadowing nothing.
        write(&d.user.join("s/mine.json"), &smoke("s/mine", "1"));

        let fixed_hash = manifest_content_hash(&fixed);
        let mut queue = Proposals::default();
        queue.entries.push(Proposal {
            workflow_id: "s/fixed".into(),
            fork_hash: fixed_hash.clone(),
            candidate_id: Some("cand-1".into()),
            released: false,
            pending: None,
        });
        queue.save(&props).unwrap();

        // Before the release: s/same goes, s/fixed and s/other stay with their diffs.
        hold(&held, "s/fixed", "old");
        let first = tidy(&d, &held, &props, 7);
        assert_eq!(first.trashed.len(), 1);
        assert_eq!(first.trashed[0].0, "s/same");
        assert!(first.trashed[0].1.contains("builtin"));
        assert!(root.join("trash/7/s/same.json").is_file());
        assert!(!d.user.join("s/same.json").exists());
        let kept: Vec<&str> = first.kept.iter().map(|f| f.workflow_id.as_str()).collect();
        assert_eq!(kept, ["s/fixed", "s/other"]);
        assert_eq!(first.kept[0].base.as_ref().unwrap().0, "fleet");
        assert_eq!(first.kept[0].diff(), ["~ version: \"1\" -> \"2\""]);

        // The catalog released the fork; a later release moved on before this device synced
        // the equal file, yet the fork still goes because its proposal was seen released.
        hold(&held, "s/fixed", &fixed_hash);
        let second = tidy(&d, &held, &props, 8);
        assert_eq!(second.trashed.len(), 1);
        assert_eq!(second.trashed[0].1, "released to the fleet catalog");
        assert!(root.join("trash/8/s/fixed.json").is_file());
        assert!(Proposals::load(&props).entries.is_empty());
        let kept: Vec<&str> = second.kept.iter().map(|f| f.workflow_id.as_str()).collect();
        assert_eq!(kept, ["s/other"]);
        assert!(d.user.join("s/mine.json").is_file());
        let _ = fs::remove_dir_all(root);
    }

    /// A2: a proposal sends the fork's manifest, base hash, reason and last run; offline it
    /// stays queued and the next send (the supervisor's, after a check-in) delivers it.
    #[tokio::test]
    async fn propose_queues_offline_and_sends_at_the_next_check_in() {
        let root = temp("send");
        let d = dirs(&root);
        write(&d.builtin.join("s/a.json"), &smoke("s/a", "1"));
        write(&d.user.join("s/a.json"), &smoke("s/a", "2"));
        let fork = forks(&d).into_iter().next().unwrap();
        let run: RunResult = serde_json::from_value(json!({
            "version": "rzn.run_result", "run_id": "r1", "workflow_id": "s/a",
            "status": "succeeded", "output": {"ok": true}
        }))
        .unwrap();
        assert!(build_candidate(&fork, " ", Some(run.clone())).is_err());
        let candidate = build_candidate(&fork, "selector moved; new selector", Some(run)).unwrap();
        assert_eq!(
            candidate.base_hash.as_deref(),
            Some(manifest_content_hash(&smoke("s/a", "1")).as_str())
        );
        assert_eq!(candidate.last_run.as_ref().unwrap().run_id, "r1");

        let path = root.join(PROPOSALS_FILENAME);
        let mut queue = Proposals::default();
        queue.queue(candidate, fork.hash.clone());
        queue.save(&path).unwrap();

        let errors = send_pending(&path, |_| async { Err("connection refused".to_string()) }).await;
        assert_eq!(errors, ["s/a: connection refused"]);
        assert!(Proposals::load(&path).entries[0].pending.is_some());

        let sent = Mutex::new(Vec::new());
        let errors = send_pending(&path, |c| {
            sent.lock().unwrap().push(c.clone());
            async {
                Ok(FleetRepairCandidateAck {
                    candidate_id: "cand-9".into(),
                    content_hash: "h".into(),
                    deduped: false,
                })
            }
        })
        .await;
        assert!(errors.is_empty());
        let sent = sent.into_inner().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].manifest["version"], "2");
        assert_eq!(sent[0].reason, "selector moved; new selector");
        let held = Proposals::load(&path);
        assert_eq!(held.entries[0].candidate_id.as_deref(), Some("cand-9"));
        assert!(held.entries[0].pending.is_none());
        assert_eq!(
            held.status_of(&fork).as_deref(),
            Some("proposed as candidate cand-9")
        );
        // Nothing left to send.
        let errors = send_pending(&path, |_| async { Err("unused".to_string()) }).await;
        assert!(errors.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_item_workflow_needs_a_passing_last_run() {
        let root = temp("item");
        let d = dirs(&root);
        let mut m = smoke("s/i", "1");
        m["result"]["contract"] = json!("rzn.item/v2");
        m["result"]["output_selector"] = json!({"step_id": "wait", "path": "$"});
        write(&d.user.join("s/i.json"), &m);
        let fork = forks(&d).into_iter().next().unwrap();
        let err = build_candidate(&fork, "why", None).unwrap_err().to_string();
        assert!(err.contains("run it live first"), "{err}");
        let bad: RunResult = serde_json::from_value(json!({
            "version": "rzn.run_result", "run_id": "r", "workflow_id": "s/i",
            "status": "succeeded", "output": {"not": "an item"}
        }))
        .unwrap();
        let err = build_candidate(&fork, "why", Some(bad))
            .unwrap_err()
            .to_string();
        assert!(err.contains("fails rzn.item/v2"), "{err}");
        let _ = fs::remove_dir_all(root);
    }
}
