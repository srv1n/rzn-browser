use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::workflow_catalog::workflow_roots;

pub const DEFAULT_SKILL_NAME: &str = "rzn-browser";
pub const MANIFEST_FILE: &str = "RZN_SKILL_INSTALL.json";

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SkillInstallScope {
    Global,
    Project,
}

impl SkillInstallScope {
    pub fn as_str(self) -> &'static str {
        match self {
            SkillInstallScope::Global => "global",
            SkillInstallScope::Project => "project",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub enum SkillClient {
    Codex,
    Claude,
    Gemini,
    Agent,
}

impl SkillClient {
    pub fn as_str(self) -> &'static str {
        match self {
            SkillClient::Codex => "codex",
            SkillClient::Claude => "claude",
            SkillClient::Gemini => "gemini",
            SkillClient::Agent => "agent",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SkillInstallRequest {
    pub skill: String,
    pub scope: SkillInstallScope,
    pub clients: Vec<SkillClient>,
    pub project_dir: PathBuf,
    pub source: Option<PathBuf>,
    pub repo_root: Option<PathBuf>,
    pub force: bool,
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct SkillRemoveRequest {
    pub skill: String,
    pub scope: SkillInstallScope,
    pub clients: Vec<SkillClient>,
    pub project_dir: PathBuf,
    pub force: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillInstallSummary {
    pub skill: String,
    pub scope: String,
    pub version: String,
    pub source: String,
    pub canonical_path: String,
    pub content_hash: String,
    pub links: Vec<SkillLinkSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillRemoveSummary {
    pub skill: String,
    pub scope: String,
    pub canonical_path: String,
    pub removed: Vec<String>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillPathsSummary {
    pub skill: String,
    pub scope: String,
    pub canonical_path: String,
    pub source_candidates: Vec<String>,
    pub links: Vec<SkillLinkSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillLinkSummary {
    pub client: String,
    pub path: String,
    pub target: String,
    pub exists: bool,
    pub points_to_target: bool,
}

#[derive(Debug, Clone, Serialize)]
struct SkillInstallManifest {
    skill: String,
    scope: String,
    version: String,
    installed_at_unix: u64,
    source: String,
    canonical_path: String,
    content_hash: String,
    clients: Vec<String>,
}

pub fn parse_clients(values: &[String]) -> Result<Vec<SkillClient>> {
    if values.is_empty() {
        return Ok(Vec::new());
    }

    let mut clients = BTreeSet::new();
    for raw in values {
        for part in raw.split(',') {
            let name = part.trim().to_ascii_lowercase();
            if name.is_empty() {
                continue;
            }
            if name == "all" {
                clients.extend([
                    SkillClient::Codex,
                    SkillClient::Claude,
                    SkillClient::Gemini,
                    SkillClient::Agent,
                ]);
                continue;
            }
            let client = match name.as_str() {
                "codex" => SkillClient::Codex,
                "claude" | "claude-code" | "claudecode" => SkillClient::Claude,
                "gemini" | "gemini-cli" => SkillClient::Gemini,
                "agent" | "agents" | "generic" | "agentskills" => SkillClient::Agent,
                _ => {
                    return Err(anyhow!(
                        "unknown skill client '{}'; use all, codex, claude, gemini, or agent",
                        part
                    ))
                }
            };
            clients.insert(client);
        }
    }

    Ok(clients.into_iter().collect())
}

pub fn default_clients() -> Vec<SkillClient> {
    vec![
        SkillClient::Codex,
        SkillClient::Claude,
        SkillClient::Gemini,
        SkillClient::Agent,
    ]
}

pub fn install_skill(req: SkillInstallRequest) -> Result<SkillInstallSummary> {
    let clients = if req.clients.is_empty() {
        default_clients()
    } else {
        req.clients.clone()
    };
    let source = resolve_skill_source(&req.skill, req.source.as_deref(), req.repo_root.as_deref())?;
    validate_skill_source(&source, &req.skill)?;
    let canonical = canonical_skill_dir(req.scope, &req.project_dir, &req.skill);

    replace_managed_dir(&source, &canonical, req.force)
        .with_context(|| format!("install skill '{}' into {}", req.skill, canonical.display()))?;

    let content_hash = hash_skill_dir(&canonical)?;
    let links = link_skill_to_clients(
        &req.skill,
        req.scope,
        &req.project_dir,
        &canonical,
        &clients,
        req.force,
    )?;
    let manifest = SkillInstallManifest {
        skill: req.skill.clone(),
        scope: req.scope.as_str().to_string(),
        version: req.version.clone(),
        installed_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        source: source.display().to_string(),
        canonical_path: canonical.display().to_string(),
        content_hash: content_hash.clone(),
        clients: clients
            .iter()
            .map(|client| client.as_str().to_string())
            .collect(),
    };
    write_manifest(&canonical, &manifest)?;

    Ok(SkillInstallSummary {
        skill: req.skill,
        scope: req.scope.as_str().to_string(),
        version: req.version,
        source: source.display().to_string(),
        canonical_path: canonical.display().to_string(),
        content_hash,
        links,
    })
}

pub fn update_skill(mut req: SkillInstallRequest) -> Result<SkillInstallSummary> {
    if req.clients.is_empty() {
        req.clients = installed_manifest_clients(req.scope, &req.project_dir, &req.skill)?
            .unwrap_or_else(default_clients);
    }
    req.force = true;
    install_skill(req)
}

pub fn remove_skill(req: SkillRemoveRequest) -> Result<SkillRemoveSummary> {
    let canonical = canonical_skill_dir(req.scope, &req.project_dir, &req.skill);
    let clients = if req.clients.is_empty() {
        installed_manifest_clients(req.scope, &req.project_dir, &req.skill)?
            .unwrap_or_else(default_clients)
    } else {
        req.clients.clone()
    };

    let mut removed = Vec::new();
    let mut skipped = Vec::new();
    for client in &clients {
        let link = client_skill_link_path(*client, req.scope, &req.project_dir, &req.skill)?;
        if remove_client_link(&link, &canonical, req.force)? {
            removed.push(link.display().to_string());
        } else {
            skipped.push(link.display().to_string());
        }
    }

    if canonical.exists() {
        if is_managed_skill_dir(&canonical) || req.force {
            remove_path(&canonical).with_context(|| {
                format!("remove canonical skill directory {}", canonical.display())
            })?;
            removed.push(canonical.display().to_string());
        } else {
            skipped.push(format!(
                "{} (not managed; pass --force to remove)",
                canonical.display()
            ));
        }
    }

    Ok(SkillRemoveSummary {
        skill: req.skill,
        scope: req.scope.as_str().to_string(),
        canonical_path: canonical.display().to_string(),
        removed,
        skipped,
    })
}

pub fn skill_paths(
    skill: &str,
    scope: SkillInstallScope,
    project_dir: &Path,
    clients: &[SkillClient],
    source: Option<&Path>,
    repo_root: Option<&Path>,
) -> Result<SkillPathsSummary> {
    let canonical = canonical_skill_dir(scope, project_dir, skill);
    let clients = if clients.is_empty() {
        default_clients()
    } else {
        clients.to_vec()
    };
    let source_candidates = skill_source_candidates(skill, source, repo_root)
        .into_iter()
        .map(|path| path.display().to_string())
        .collect();
    let links = clients
        .iter()
        .map(|client| {
            let path = client_skill_link_path(*client, scope, project_dir, skill)?;
            Ok(link_summary(*client, &path, &canonical))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(SkillPathsSummary {
        skill: skill.to_string(),
        scope: scope.as_str().to_string(),
        canonical_path: canonical.display().to_string(),
        source_candidates,
        links,
    })
}

fn resolve_skill_source(
    skill: &str,
    source: Option<&Path>,
    repo_root: Option<&Path>,
) -> Result<PathBuf> {
    for candidate in skill_source_candidates(skill, source, repo_root) {
        if candidate.join("SKILL.md").is_file() {
            return Ok(candidate);
        }
    }

    let rendered = skill_source_candidates(skill, source, repo_root)
        .into_iter()
        .map(|path| format!("  - {}", path.display()))
        .collect::<Vec<_>>()
        .join("\n");
    Err(anyhow!(
        "could not find bundled skill '{}'. Checked:\n{}",
        skill,
        rendered
    ))
}

fn skill_source_candidates(
    skill: &str,
    source: Option<&Path>,
    repo_root: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(source) = source {
        candidates.push(source.to_path_buf());
    }
    if let Some(repo_root) = repo_root {
        candidates.push(repo_root.join("skills").join(skill));
    }
    if let Ok(current) = std::env::current_dir() {
        candidates.push(current.join("skills").join(skill));
    }
    let roots = workflow_roots();
    candidates.push(roots.runtime_dir.join("skills").join("builtin").join(skill));
    candidates.push(roots.runtime_dir.join("skills").join(skill));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            candidates.push(
                bin_dir
                    .join("..")
                    .join("skills")
                    .join("builtin")
                    .join(skill),
            );
            candidates.push(bin_dir.join("..").join("skills").join(skill));
        }
    }

    dedupe_paths(candidates)
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for path in paths {
        let key = path.to_string_lossy().to_string();
        if seen.insert(key) {
            out.push(path);
        }
    }
    out
}

fn validate_skill_source(source: &Path, skill: &str) -> Result<()> {
    let skill_md = source.join("SKILL.md");
    let content =
        fs::read_to_string(&skill_md).with_context(|| format!("read {}", skill_md.display()))?;
    if !content.starts_with("---\n") {
        return Err(anyhow!(
            "{} is missing YAML frontmatter",
            skill_md.display()
        ));
    }
    let expected = format!("name: {}", skill);
    if !content.lines().any(|line| line.trim() == expected) {
        return Err(anyhow!(
            "{} must contain `{}` so the folder and skill name match",
            skill_md.display(),
            expected
        ));
    }
    Ok(())
}

fn canonical_skill_dir(scope: SkillInstallScope, project_dir: &Path, skill: &str) -> PathBuf {
    match scope {
        SkillInstallScope::Global => workflow_roots()
            .runtime_dir
            .join("skills")
            .join("installed")
            .join(skill),
        SkillInstallScope::Project => project_dir.join(".rzn").join("skills").join(skill),
    }
}

fn client_skill_link_path(
    client: SkillClient,
    scope: SkillInstallScope,
    project_dir: &Path,
    skill: &str,
) -> Result<PathBuf> {
    let base = match (client, scope) {
        (SkillClient::Codex, SkillInstallScope::Global) => codex_home().join("skills"),
        (SkillClient::Codex, SkillInstallScope::Project) => {
            project_dir.join(".codex").join("skills")
        }
        (SkillClient::Claude, SkillInstallScope::Global) => {
            home_dir()?.join(".claude").join("skills")
        }
        (SkillClient::Claude, SkillInstallScope::Project) => {
            project_dir.join(".claude").join("skills")
        }
        (SkillClient::Gemini, SkillInstallScope::Global) => {
            home_dir()?.join(".gemini").join("skills")
        }
        (SkillClient::Gemini, SkillInstallScope::Project) => {
            project_dir.join(".gemini").join("skills")
        }
        (SkillClient::Agent, SkillInstallScope::Global) => {
            home_dir()?.join(".agents").join("skills")
        }
        (SkillClient::Agent, SkillInstallScope::Project) => {
            project_dir.join(".agents").join("skills")
        }
    };
    Ok(base.join(skill))
}

fn codex_home() -> PathBuf {
    std::env::var("CODEX_HOME")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            home_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".codex")
        })
}

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow!("could not determine home directory"))
}

fn replace_managed_dir(source: &Path, dest: &Path, force: bool) -> Result<()> {
    if dest.exists() {
        if !force && !is_managed_skill_dir(dest) {
            return Err(anyhow!(
                "{} already exists and was not created by rzn-browser; pass --force to replace it",
                dest.display()
            ));
        }
        remove_path(dest)?;
    }
    copy_dir_recursive(source, dest)?;
    Ok(())
}

fn copy_dir_recursive(source: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest).with_context(|| format!("create {}", dest.display()))?;
    for entry in fs::read_dir(source).with_context(|| format!("read {}", source.display()))? {
        let entry = entry?;
        let src = entry.path();
        let dst = dest.join(entry.file_name());
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else if ty.is_file() {
            fs::copy(&src, &dst)
                .with_context(|| format!("copy {} -> {}", src.display(), dst.display()))?;
        } else if ty.is_symlink() {
            let target =
                fs::read_link(&src).with_context(|| format!("read symlink {}", src.display()))?;
            create_symlink(&target, &dst)
                .with_context(|| format!("copy symlink {} -> {}", src.display(), dst.display()))?;
        }
    }
    Ok(())
}

fn link_skill_to_clients(
    skill: &str,
    scope: SkillInstallScope,
    project_dir: &Path,
    canonical: &Path,
    clients: &[SkillClient],
    force: bool,
) -> Result<Vec<SkillLinkSummary>> {
    clients
        .iter()
        .map(|client| {
            let link = client_skill_link_path(*client, scope, project_dir, skill)?;
            create_client_symlink(&link, canonical, force)?;
            Ok(link_summary(*client, &link, canonical))
        })
        .collect()
}

fn create_client_symlink(link: &Path, canonical: &Path, force: bool) -> Result<()> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    if link.exists() || fs::symlink_metadata(link).is_ok() {
        let metadata = fs::symlink_metadata(link)?;
        if metadata.file_type().is_symlink() || force {
            remove_path(link)?;
        } else {
            return Err(anyhow!(
                "{} already exists and is not a symlink; pass --force to replace it",
                link.display()
            ));
        }
    }
    create_symlink(canonical, link).with_context(|| {
        format!(
            "create symlink {} -> {}",
            link.display(),
            canonical.display()
        )
    })?;
    Ok(())
}

fn remove_client_link(link: &Path, canonical: &Path, force: bool) -> Result<bool> {
    let Ok(metadata) = fs::symlink_metadata(link) else {
        return Ok(false);
    };
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(link).unwrap_or_default();
        if force || same_path_text(&target, canonical) {
            remove_path(link)?;
            return Ok(true);
        }
        return Ok(false);
    }
    if is_managed_skill_dir(link) || force {
        remove_path(link)?;
        return Ok(true);
    }
    Ok(false)
}

fn remove_path(path: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("stat {}", path.display()))?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    } else if metadata.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(())
}

fn is_managed_skill_dir(path: &Path) -> bool {
    path.join(MANIFEST_FILE).is_file()
}

fn write_manifest(path: &Path, manifest: &SkillInstallManifest) -> Result<()> {
    let body = serde_json::to_string_pretty(manifest)?;
    fs::write(path.join(MANIFEST_FILE), format!("{}\n", body))
        .with_context(|| format!("write {}", path.join(MANIFEST_FILE).display()))
}

fn installed_manifest_clients(
    scope: SkillInstallScope,
    project_dir: &Path,
    skill: &str,
) -> Result<Option<Vec<SkillClient>>> {
    let manifest_path = canonical_skill_dir(scope, project_dir, skill).join(MANIFEST_FILE);
    if !manifest_path.is_file() {
        return Ok(None);
    }
    let value: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path)
            .with_context(|| format!("read {}", manifest_path.display()))?,
    )?;
    let clients = value
        .get("clients")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(|item| item.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let parsed = parse_clients(&clients)?;
    if parsed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(parsed))
    }
}

fn hash_skill_dir(path: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_files(path, path, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (relative, absolute) in files {
        if relative == MANIFEST_FILE {
            continue;
        }
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        hasher.update(fs::read(&absolute).with_context(|| format!("read {}", absolute.display()))?);
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<(String, PathBuf)>) -> Result<()> {
    for entry in fs::read_dir(current).with_context(|| format!("read {}", current.display()))? {
        let entry = entry?;
        let path = entry.path();
        let ty = entry.file_type()?;
        if ty.is_dir() {
            collect_files(root, &path, files)?;
        } else if ty.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            files.push((relative, path));
        }
    }
    Ok(())
}

fn link_summary(client: SkillClient, link: &Path, canonical: &Path) -> SkillLinkSummary {
    let target = fs::read_link(link)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| canonical.display().to_string());
    SkillLinkSummary {
        client: client.as_str().to_string(),
        path: link.display().to_string(),
        target,
        exists: link.exists() || fs::symlink_metadata(link).is_ok(),
        points_to_target: fs::read_link(link)
            .map(|target| same_path_text(&target, canonical))
            .unwrap_or(false),
    }
}

fn same_path_text(left: &Path, right: &Path) -> bool {
    left == right || left.to_string_lossy() == right.to_string_lossy()
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clients_supports_all_and_aliases() {
        let clients = parse_clients(&["codex,claude-code".to_string(), "gemini".to_string()])
            .expect("parse clients");
        assert_eq!(
            clients,
            vec![SkillClient::Codex, SkillClient::Claude, SkillClient::Gemini]
        );

        let all = parse_clients(&["all".to_string()]).expect("parse all");
        assert_eq!(all, default_clients());
    }

    #[test]
    fn project_paths_match_client_conventions() {
        let root = PathBuf::from("/tmp/project");
        assert_eq!(
            client_skill_link_path(
                SkillClient::Claude,
                SkillInstallScope::Project,
                &root,
                "rzn-browser"
            )
            .expect("claude path"),
            PathBuf::from("/tmp/project/.claude/skills/rzn-browser")
        );
        assert_eq!(
            client_skill_link_path(
                SkillClient::Gemini,
                SkillInstallScope::Project,
                &root,
                "rzn-browser"
            )
            .expect("gemini path"),
            PathBuf::from("/tmp/project/.gemini/skills/rzn-browser")
        );
        assert_eq!(
            client_skill_link_path(
                SkillClient::Agent,
                SkillInstallScope::Project,
                &root,
                "rzn-browser"
            )
            .expect("agent path"),
            PathBuf::from("/tmp/project/.agents/skills/rzn-browser")
        );
    }
}
