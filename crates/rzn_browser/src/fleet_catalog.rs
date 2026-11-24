//! Device half of the fleet catalog (backend spec section 4) and of site pauses (section 6).
//!
//! The supervisor keeps the catalog it holds in `<app_base>/fleet_catalog.json` and sends its
//! hash on each check-in. When the server answers `catalog_changed`, [`sync`] reads
//! `GET /v1/fleet/catalog`, fetches each manifest whose hash it does not hold through the
//! workflow cache (which verifies the sha256), writes it to the fleet workflow directory local
//! runs read (`workflow_catalog::fleet_workflows_dir`), and removes a workflow the catalog no
//! longer lists or has retired. A paused workflow stays; a local run of it prints a warning.
//!
//! A site paused by a challenge is written into the local site queue
//! ([`rzn_browser::site_queue::pause_site_until`]), so a local run of that site refuses until the pause
//! ends; the server already gives this device no job for it.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rzn_contracts::fleet::{FleetCatalog, FleetCatalogEntry, FleetSitePause};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::workflow_cache::{ManifestFetcher, WorkflowCache};

pub(crate) const HELD_CATALOG_FILENAME: &str = "fleet_catalog.json";
const RETIRED: &str = "retired";
const PAUSED: &str = "paused";

/// The catalog this device holds.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct HeldCatalog {
    /// The server's hash of the catalog last synced in full; `None` after a sync that could not
    /// write every workflow, so the next check-in is told the catalog changed and retries.
    #[serde(default)]
    pub catalog_hash: Option<String>,
    #[serde(default)]
    pub workflows: Vec<FleetCatalogEntry>,
}

impl HeldCatalog {
    pub(crate) fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, path)
    }

    fn entry(&self, workflow_id: &str) -> Option<&FleetCatalogEntry> {
        self.workflows.iter().find(|e| e.workflow_id == workflow_id)
    }
}

/// What one sync did, by workflow id.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SyncReport {
    /// Fetched from the server (the cache did not hold that hash).
    pub fetched: Vec<String>,
    /// Written to the fleet workflow directory (a new version, or a file that was missing).
    pub written: Vec<String>,
    pub removed: Vec<String>,
    pub failed: Vec<String>,
}

/// Bring the fleet workflow directory to `catalog` and record it as held.
pub(crate) async fn sync(
    catalog: FleetCatalog,
    held_path: &Path,
    workflows_dir: &Path,
    cache: &WorkflowCache,
    fetcher: &dyn ManifestFetcher,
) -> SyncReport {
    let held = HeldCatalog::load(held_path);
    let mut report = SyncReport::default();
    let mut kept: Vec<FleetCatalogEntry> = Vec::new();

    for entry in &catalog.workflows {
        if entry.status == RETIRED {
            continue;
        }
        let id = entry.workflow_id.as_str();
        let Some(file) = workflow_file(workflows_dir, id) else {
            tracing::warn!(workflow_id = %id, "fleet catalog: not a workflow id; skipped");
            report.failed.push(id.to_string());
            continue;
        };
        let held_same = held
            .entry(id)
            .is_some_and(|h| h.content_hash == entry.content_hash);
        if !(held_same && file.is_file()) {
            let in_cache = cache
                .manifest_path(id, &entry.content_hash)
                .is_ok_and(|p| p.is_file());
            let written = match cache.get(id, &entry.content_hash, fetcher).await {
                Ok(manifest) => write_manifest(&file, &manifest),
                Err(err) => Err(err.to_string()),
            };
            if let Err(err) = written {
                tracing::warn!(workflow_id = %id, "fleet catalog: {err}");
                report.failed.push(id.to_string());
                // The previous version, if any, stays on disk and stays held.
                kept.extend(held.entry(id).cloned());
                continue;
            }
            if !in_cache {
                report.fetched.push(id.to_string());
            }
            report.written.push(id.to_string());
            tracing::info!(workflow_id = %id, content_hash = %entry.content_hash,
                fetched = !in_cache, "fleet catalog: wrote workflow");
        }
        kept.push(entry.clone());
    }

    let live: HashSet<&str> = catalog
        .workflows
        .iter()
        .filter(|e| e.status != RETIRED)
        .map(|e| e.workflow_id.as_str())
        .collect();
    for gone in held
        .workflows
        .iter()
        .filter(|h| !live.contains(h.workflow_id.as_str()))
    {
        if let Some(file) = workflow_file(workflows_dir, &gone.workflow_id) {
            match fs::remove_file(&file) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => {
                    tracing::warn!(workflow_id = %gone.workflow_id, "fleet catalog: remove failed: {err}");
                    report.failed.push(gone.workflow_id.clone());
                    continue;
                }
            }
            tracing::info!(workflow_id = %gone.workflow_id, "fleet catalog: removed workflow");
            report.removed.push(gone.workflow_id.clone());
        }
    }

    let next = HeldCatalog {
        catalog_hash: report
            .failed
            .is_empty()
            .then(|| catalog.catalog_hash.clone()),
        workflows: kept,
    };
    if let Err(err) = next.save(held_path) {
        tracing::warn!(
            "fleet catalog: could not save {}: {err}",
            held_path.display()
        );
    }
    report
}

/// `<dir>/<id>.json` for a catalog id of `/`-separated `[a-z0-9_-]` segments, else `None`.
fn workflow_file(dir: &Path, workflow_id: &str) -> Option<PathBuf> {
    let ok = workflow_id.split('/').all(|segment| {
        !segment.is_empty()
            && segment
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    });
    ok.then(|| {
        let mut path = dir.to_path_buf();
        for segment in workflow_id.split('/') {
            path.push(segment);
        }
        path.set_extension("json");
        path
    })
}

/// Write a manifest local runs will list, refusing one the manifest contract rejects (an
/// invalid file in the directory would break every local workflow lookup).
fn write_manifest(file: &Path, manifest: &Value) -> Result<(), String> {
    if let Err(issues) = rzn_contracts::workflow::validate_manifest_value(manifest) {
        let first = issues
            .first()
            .map(|i| format!("{}: {}", i.field, i.message))
            .unwrap_or_default();
        return Err(format!("manifest is not a valid workflow ({first})"));
    }
    let parent = file
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", file.display()))?;
    fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?;
    let tmp = file.with_extension("json.tmp");
    fs::write(&tmp, bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, file).map_err(|e| format!("rename to {}: {e}", file.display()))
}

/// Record the server's pauses for this device. A challenge pause (with an end) blocks local
/// runs of the site until it ends; the login walls (no end) are returned for the extension to
/// show until the user reports one cleared.
pub(crate) fn apply_site_pauses(app_base: &Path, pauses: &[FleetSitePause]) -> Vec<String> {
    let mut login_walls = Vec::new();
    for pause in pauses {
        match pause.until_ms {
            Some(until_ms) if until_ms > 0 => {
                match rzn_browser::site_queue::pause_site_until(
                    app_base,
                    &pause.system,
                    until_ms as u64,
                ) {
                    Ok(true) => tracing::info!(system = %pause.system, until_ms,
                        reason = %pause.reason, "fleet: site paused on this device"),
                    Ok(false) => {}
                    Err(err) => {
                        tracing::warn!(system = %pause.system, "fleet: site pause: {err:#}")
                    }
                }
            }
            _ => login_walls.push(pause.system.clone()),
        }
    }
    login_walls.sort();
    login_walls.dedup();
    login_walls
}

/// A warning when `workflow_path` is a fleet catalog workflow the catalog has paused.
pub(crate) fn paused_note(workflow_path: &Path) -> Option<String> {
    paused_note_in(
        workflow_path,
        &crate::workflow_catalog::fleet_workflows_dir(),
        &rzn_core::runtime_paths::default_app_base_dir().join(HELD_CATALOG_FILENAME),
    )
}

fn paused_note_in(workflow_path: &Path, workflows_dir: &Path, held_path: &Path) -> Option<String> {
    let relative = workflow_path.strip_prefix(workflows_dir).ok()?;
    let id = relative
        .with_extension("")
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let held = HeldCatalog::load(held_path);
    let entry = held.entry(&id).filter(|e| e.status == PAUSED)?;
    let reason = entry
        .paused_reason
        .as_deref()
        .map(|r| format!(" ({r})"))
        .unwrap_or_default();
    Some(format!(
        "warning: {id} is paused in the fleet catalog{reason}; this local run uses its last released version"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_cache::{manifest_content_hash, FetchError};
    use async_trait::async_trait;
    use rzn_contracts::fleet::FleetWorkflowFetchResponse;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Server {
        manifests: HashMap<String, Value>,
        fetched: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ManifestFetcher for Server {
        async fn fetch(
            &self,
            id: &str,
            hash: &str,
        ) -> Result<FleetWorkflowFetchResponse, FetchError> {
            self.fetched.lock().unwrap().push(id.to_string());
            let manifest = self
                .manifests
                .get(hash)
                .cloned()
                .ok_or_else(|| FetchError::new("404"))?;
            Ok(FleetWorkflowFetchResponse {
                workflow_id: id.to_string(),
                content_hash: hash.to_string(),
                manifest,
            })
        }
    }

    /// The smoke workflow under another id and version.
    fn manifest(id: &str, version: &str) -> Value {
        let mut m: Value =
            serde_json::from_str(include_str!("../../../workflows/_smoke/fleet-tier1.json"))
                .unwrap();
        m["id"] = json!(id);
        m["system"] = json!(id.split('/').next().unwrap());
        m["version"] = json!(version);
        m
    }

    fn entry(id: &str, m: &Value, status: &str) -> FleetCatalogEntry {
        FleetCatalogEntry {
            workflow_id: id.to_string(),
            content_hash: manifest_content_hash(m),
            status: status.to_string(),
            paused_reason: (status == PAUSED).then(|| "broken".to_string()),
            system: id.split('/').next().unwrap().to_string(),
        }
    }

    fn temp(tag: &str) -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("rzn-fleet-catalog-{tag}-{n}"))
    }

    #[tokio::test]
    async fn sync_fetches_only_changed_manifests_and_removes_retired_or_gone() {
        let root = temp("sync");
        let (held_path, dir) = (root.join(HELD_CATALOG_FILENAME), root.join("fleet"));
        let cache = WorkflowCache::new(root.join("cache"));
        let (a1, a2, b, c) = (
            manifest("s/a", "1"),
            manifest("s/a", "2"),
            manifest("s/b", "1"),
            manifest("s/c", "1"),
        );
        let server = Server {
            manifests: [&a1, &a2, &b, &c]
                .into_iter()
                .map(|m| (manifest_content_hash(m), m.clone()))
                .collect(),
            fetched: Mutex::new(Vec::new()),
        };

        let first = FleetCatalog {
            catalog_hash: "h1".into(),
            workflows: vec![
                entry("s/a", &a1, "active"),
                entry("s/b", &b, "active"),
                entry("s/c", &c, "active"),
            ],
        };
        let report = sync(first, &held_path, &dir, &cache, &server).await;
        assert_eq!(report.fetched, ["s/a", "s/b", "s/c"]);
        assert_eq!(
            HeldCatalog::load(&held_path).catalog_hash.as_deref(),
            Some("h1")
        );
        assert!(dir.join("s/b.json").is_file());

        // One manifest republished, one retired, one dropped: exactly one fetch.
        server.fetched.lock().unwrap().clear();
        let second = FleetCatalog {
            catalog_hash: "h2".into(),
            workflows: vec![entry("s/a", &a2, "paused"), entry("s/b", &b, "retired")],
        };
        let report = sync(second, &held_path, &dir, &cache, &server).await;
        assert_eq!(*server.fetched.lock().unwrap(), ["s/a"]);
        assert_eq!(report.written, ["s/a"]);
        assert_eq!(report.removed, ["s/b", "s/c"]);
        assert!(!dir.join("s/b.json").exists() && !dir.join("s/c.json").exists());
        let written: Value =
            serde_json::from_slice(&fs::read(dir.join("s/a.json")).unwrap()).unwrap();
        assert_eq!(written["version"], "2");

        let note = paused_note_in(&dir.join("s/a.json"), &dir, &held_path).unwrap();
        assert!(
            note.contains("s/a is paused") && note.contains("broken"),
            "{note}"
        );
        assert_eq!(
            paused_note_in(&root.join("other/s/a.json"), &dir, &held_path),
            None
        );
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_failed_fetch_leaves_the_catalog_unheld_so_the_next_check_in_retries() {
        let root = temp("fail");
        let (held_path, dir) = (root.join(HELD_CATALOG_FILENAME), root.join("fleet"));
        let cache = WorkflowCache::new(root.join("cache"));
        let a = manifest("s/a", "1");
        let server = Server {
            manifests: HashMap::new(),
            fetched: Mutex::new(Vec::new()),
        };
        let catalog = FleetCatalog {
            catalog_hash: "h".into(),
            workflows: vec![entry("s/a", &a, "active")],
        };
        let report = sync(catalog, &held_path, &dir, &cache, &server).await;
        assert_eq!(report.failed, ["s/a"]);
        assert_eq!(HeldCatalog::load(&held_path).catalog_hash, None);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn challenge_pauses_block_the_site_and_login_walls_are_returned() {
        let root = temp("pauses");
        let until = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64)
            + 60_000;
        let walls = apply_site_pauses(
            &root,
            &[
                FleetSitePause {
                    system: "s".into(),
                    until_ms: Some(until),
                    reason: "site_challenge".into(),
                },
                FleetSitePause {
                    system: "t".into(),
                    until_ms: None,
                    reason: "auth_wall".into(),
                },
            ],
        );
        assert_eq!(walls, ["t"]);
        let state: Value =
            serde_json::from_slice(&fs::read(root.join("site-queue/s.json")).unwrap()).unwrap();
        assert_eq!(state["blocked_until_ms"], until);
        let _ = fs::remove_dir_all(root);
    }
}
