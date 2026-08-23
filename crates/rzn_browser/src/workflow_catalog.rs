use anyhow::{anyhow, Context, Result};
use rzn_contracts::workflow::{validate_manifest_value, WORKFLOW_CONTRACT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

const RUNTIME_DIR_ENV: &str = "RZN_RUNTIME_DIR";
const BUILTIN_DIR_ENV: &str = "RZN_BUILTIN_WORKFLOWS_DIR";
const USER_DIR_ENV: &str = "RZN_WORKFLOWS_DIR";

#[derive(Debug, Clone)]
pub struct WorkflowRoots {
    pub runtime_dir: PathBuf,
    pub builtin_dir: PathBuf,
    pub user_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuiltinCatalogInstallSummary {
    pub builtin_dir: String,
    pub workflow_files: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct NamedWorkflowEntry {
    pub id: String,
    pub system: String,
    pub workflow: String,
    pub source: String,
    pub path: String,
    pub relative_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "is_true")]
    pub effective: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadowed_by_source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides_sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityCatalogEntry {
    pub system: String,
    pub capability_id: String,
    pub workflow: String,
    pub route: String,
    pub source: String,
    pub manifest_path: String,
    pub workflow_path: String,
    pub manifest_version: String,
    pub content_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CapabilityCatalogQuery {
    pub system_filter: Option<String>,
    pub source_filter: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogValidationLevel {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogValidationIssue {
    pub level: CatalogValidationLevel,
    pub source: String,
    pub path: String,
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogValidationReport {
    pub ok: bool,
    pub strict: bool,
    pub manifest_count: usize,
    pub capability_count: usize,
    pub error_count: usize,
    pub warning_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<CatalogValidationIssue>,
}

#[derive(Debug, Clone)]
struct CapabilityManifestRecord {
    source: String,
    path: PathBuf,
    content_hash: String,
    value: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct CapabilityManifest {
    #[serde(default)]
    schema_version: Option<String>,
    #[serde(default)]
    system: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    capability: Option<String>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    side_effects: Vec<Value>,
}

#[derive(Debug, Clone)]
struct WorkflowEntry {
    id: String,
    system: String,
    workflow: String,
    source: String,
    path: PathBuf,
    relative_path: String,
    relative_stem: String,
    name: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Clone)]
struct WorkflowIdentity {
    system: String,
    workflow: String,
}

#[derive(Debug, Clone, Default)]
pub struct WorkflowCatalogQuery {
    pub system_filter: Option<String>,
    pub source_filter: Option<String>,
    pub include_all_sources: bool,
}

pub fn workflow_roots() -> WorkflowRoots {
    let runtime_dir = std::env::var(RUNTIME_DIR_ENV)
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(default_runtime_dir);
    let user_dir = std::env::var(USER_DIR_ENV)
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime_dir.join("workflows").join("user"));
    let builtin_dir = std::env::var(BUILTIN_DIR_ENV)
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime_dir.join("workflows").join("builtin"));
    WorkflowRoots {
        runtime_dir,
        builtin_dir,
        user_dir,
    }
}

pub fn default_runtime_dir() -> PathBuf {
    if let Some(dir) = dirs::data_local_dir() {
        return dir.join(if cfg!(target_os = "linux") {
            "rzn"
        } else {
            "RZN"
        });
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(if cfg!(target_os = "linux") {
            "rzn"
        } else {
            "RZN"
        });
    }
    PathBuf::from(if cfg!(target_os = "linux") {
        "rzn"
    } else {
        "RZN"
    })
}

pub fn default_user_workflows_dir() -> PathBuf {
    workflow_roots().user_dir
}

pub fn install_builtin_catalog_from_repo_root(
    repo_root: &Path,
) -> Result<BuiltinCatalogInstallSummary> {
    let roots = workflow_roots();
    install_builtin_catalog_to_root(repo_root, &roots.builtin_dir, &roots.user_dir)
}

pub fn install_builtin_catalog_to_root(
    repo_root: &Path,
    builtin_dir: &Path,
    user_dir: &Path,
) -> Result<BuiltinCatalogInstallSummary> {
    let source_root = detect_catalog_source_root(repo_root)?;
    let workflows_src = source_root.join("workflows");
    if !workflows_src.is_dir() {
        return Err(anyhow!(
            "catalog source '{}' is missing a workflows/ directory",
            source_root.display()
        ));
    }

    fs::create_dir_all(user_dir)
        .with_context(|| format!("create user workflow dir {}", user_dir.display()))?;

    let parent = builtin_dir.parent().ok_or_else(|| {
        anyhow!(
            "builtin workflow dir has no parent: {}",
            builtin_dir.display()
        )
    })?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create builtin workflow parent {}", parent.display()))?;

    let staging_dir = unique_sibling_dir(builtin_dir, "staging");
    let backup_dir = unique_sibling_dir(builtin_dir, "backup");
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir).with_context(|| {
            format!(
                "remove stale staging workflow dir {}",
                staging_dir.display()
            )
        })?;
    }
    fs::create_dir_all(&staging_dir)
        .with_context(|| format!("create staging workflow dir {}", staging_dir.display()))?;

    let install_result =
        (|| -> Result<usize> { copy_workflow_catalog(&workflows_src, &staging_dir) })();

    let workflow_files = match install_result {
        Ok(count) => count,
        Err(err) => {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(err);
        }
    };

    if backup_dir.exists() {
        fs::remove_dir_all(&backup_dir).with_context(|| {
            format!("remove stale backup workflow dir {}", backup_dir.display())
        })?;
    }
    if builtin_dir.exists() {
        if let Err(err) = fs::rename(builtin_dir, &backup_dir).with_context(|| {
            format!(
                "move existing builtin workflow dir {} -> {}",
                builtin_dir.display(),
                backup_dir.display()
            )
        }) {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(err);
        }
    }
    if let Err(err) = fs::rename(&staging_dir, builtin_dir).with_context(|| {
        format!(
            "activate staged builtin workflow dir {} -> {}",
            staging_dir.display(),
            builtin_dir.display()
        )
    }) {
        if backup_dir.exists() && !builtin_dir.exists() {
            let _ = fs::rename(&backup_dir, builtin_dir);
        }
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(err);
    }
    let _ = fs::remove_dir_all(&backup_dir);

    Ok(BuiltinCatalogInstallSummary {
        builtin_dir: builtin_dir.to_string_lossy().to_string(),
        workflow_files,
    })
}

fn unique_sibling_dir(path: &Path, label: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("builtin");
    parent.join(format!(
        ".{}.{}.{}.{}",
        name,
        label,
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
}

pub fn detect_catalog_source_root(root: &Path) -> Result<PathBuf> {
    if root.join("workflows").is_dir() {
        return Ok(root.to_path_buf());
    }

    let mut child_dirs: Vec<PathBuf> = fs::read_dir(root)
        .with_context(|| format!("read dir {}", root.display()))?
        .filter_map(|entry| entry.ok().map(|value| value.path()))
        .filter(|path| path.is_dir())
        .collect();
    child_dirs.sort();

    if child_dirs.len() == 1 && child_dirs[0].join("workflows").is_dir() {
        return Ok(child_dirs.remove(0));
    }

    Err(anyhow!(
        "could not find a repo-style catalog root under {}",
        root.display()
    ))
}

pub fn list_named_workflows_with_query(
    query: &WorkflowCatalogQuery,
) -> Result<Vec<NamedWorkflowEntry>> {
    Ok(build_named_workflow_entries(
        collect_all_workflow_entries()?,
        query,
    ))
}

pub fn list_capabilities_with_query(
    query: &CapabilityCatalogQuery,
) -> Result<Vec<CapabilityCatalogEntry>> {
    let mut issues = Vec::new();
    let mut entries = collect_capability_entries(query, &mut issues)?;
    if let Some(error) = issues
        .iter()
        .find(|issue| matches!(issue.level, CatalogValidationLevel::Error))
    {
        return Err(anyhow!(
            "catalog manifest {} is invalid at {}: {}",
            error.path,
            error.field,
            error.message
        ));
    }
    entries.sort_by(|a, b| {
        a.system
            .cmp(&b.system)
            .then_with(|| a.capability_id.cmp(&b.capability_id))
            .then_with(|| source_rank(&a.source).cmp(&source_rank(&b.source)))
    });
    Ok(entries)
}

pub fn resolve_capability_route(
    system: &str,
    capability_id: &str,
) -> Result<CapabilityCatalogEntry> {
    let system = slugify(system);
    let capability_id = normalize_capability_id(capability_id);
    if system.is_empty() {
        return Err(anyhow!("capability routing requires explicit --system"));
    }
    if capability_id.is_empty() {
        return Err(anyhow!("capability id cannot be empty"));
    }

    let entries = list_capabilities_with_query(&CapabilityCatalogQuery {
        system_filter: Some(system.clone()),
        source_filter: None,
    })?;
    let matches = entries
        .into_iter()
        .filter(|entry| entry.system == system && entry.capability_id == capability_id)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [entry] => Ok(entry.clone()),
        [] => Err(anyhow!(
            "capability '{}' was not found for explicit system '{}'",
            capability_id,
            system
        )),
        _ => Err(anyhow!(
            "capability '{}' for system '{}' is ambiguous across catalog sources",
            capability_id,
            system
        )),
    }
}

pub fn validate_catalog_manifests(strict: bool) -> Result<CatalogValidationReport> {
    let mut issues = Vec::new();
    let records = collect_capability_manifest_records(false)?;
    if strict {
        let effective_record_paths = effective_capability_manifest_record_paths(&records);
        let effective_records = records
            .iter()
            .filter(|record| effective_record_paths.contains(&record.path))
            .cloned()
            .collect::<Vec<_>>();
        validate_manifest_contracts(&effective_records, &mut issues);
    }
    let manifest_count = records.len();
    let entries = collect_capability_entries_from_records(
        &CapabilityCatalogQuery::default(),
        &mut issues,
        records,
    );

    if strict && manifest_count == 0 {
        issues.push(CatalogValidationIssue {
            level: CatalogValidationLevel::Error,
            source: "catalog".to_string(),
            path: "-".to_string(),
            field: "manifests".to_string(),
            message: "strict catalog validation requires at least one manifest file".to_string(),
        });
    }

    push_duplicate_capability_route_issues(&entries, &mut issues);

    let error_count = issues
        .iter()
        .filter(|issue| matches!(issue.level, CatalogValidationLevel::Error))
        .count();
    let warning_count = issues
        .iter()
        .filter(|issue| matches!(issue.level, CatalogValidationLevel::Warning))
        .count();

    Ok(CatalogValidationReport {
        ok: error_count == 0,
        strict,
        manifest_count,
        capability_count: entries.len(),
        error_count,
        warning_count,
        issues,
    })
}

fn validate_manifest_contracts(
    records: &[CapabilityManifestRecord],
    issues: &mut Vec<CatalogValidationIssue>,
) {
    for record in records {
        match validate_manifest_value(&record.value) {
            Ok(_) => {}
            Err(contract_issues) => {
                for issue in contract_issues {
                    issues.push(CatalogValidationIssue {
                        level: CatalogValidationLevel::Error,
                        source: record.source.clone(),
                        path: record.path.to_string_lossy().to_string(),
                        field: if issue.field.trim().is_empty() {
                            "manifest".to_string()
                        } else {
                            issue.field
                        },
                        message: issue.message,
                    });
                }
            }
        }
    }
}

pub fn resolve_workflow_reference(reference: &str) -> Result<PathBuf> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("workflow reference cannot be empty"));
    }

    let expanded = expand_tilde(trimmed);
    if expanded.exists() {
        return Ok(expanded);
    }

    let normalized_input = normalize_pathish(trimmed);
    let normalized_no_ext = strip_json_ext(&normalized_input).to_string();
    let flat_alias = slugify(trimmed);

    let entries = collect_workflow_entries()?;
    if let Some(entry) =
        find_matching_workflow_entry(&entries, &normalized_input, &normalized_no_ext)
    {
        return Ok(entry.path.clone());
    }

    let matching_system_entries = entries
        .iter()
        .filter(|entry| entry.system == flat_alias)
        .collect::<Vec<_>>();
    if !matching_system_entries.is_empty() {
        let mut suggestions = matching_system_entries
            .iter()
            .map(|entry| {
                let mut line = format!("  - {} {}", flat_alias, entry.workflow);
                if let Some(name) = entry
                    .name
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                {
                    line.push_str(&format!(" — {}", name));
                }
                if let Some(description) = entry
                    .description
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                {
                    line.push_str(&format!(" | {}", description));
                }
                line
            })
            .collect::<Vec<_>>();
        suggestions.sort();
        return Err(anyhow!(
            "'{}' is a workflow system, not a runnable workflow.\nTry `rzn-browser workflow list {}` to inspect it, or run one of:\n{}",
            reference,
            flat_alias,
            suggestions.join("\n")
        ));
    }

    Err(anyhow!(
        "workflow '{}' was not found as a file path or installed workflow id",
        reference
    ))
}

fn find_matching_workflow_entry<'a>(
    entries: &'a [WorkflowEntry],
    normalized_input: &str,
    normalized_no_ext: &str,
) -> Option<&'a WorkflowEntry> {
    entries.iter().find(|entry| {
        normalized_input == entry.id
            || normalized_no_ext == entry.id
            || normalized_input == entry.relative_path
            || normalized_no_ext == entry.relative_stem
    })
}

pub fn compose_workflow_reference(primary: &str, secondary: Option<&str>) -> Result<String> {
    let primary = primary.trim();
    if primary.is_empty() {
        return Err(anyhow!("workflow reference cannot be empty"));
    }
    match secondary.map(str::trim).filter(|value| !value.is_empty()) {
        Some(workflow) => {
            if primary.contains('/') {
                return Err(anyhow!(
                    "workflow system '{}' already looks namespaced; do not pass a second positional workflow name",
                    primary
                ));
            }
            Ok(format!("{}/{}", slugify(primary), slugify(workflow)))
        }
        None => Ok(primary.to_string()),
    }
}

pub fn import_user_workflows(
    source: &Path,
    system: Option<&str>,
    workflow: Option<&str>,
    force: bool,
) -> Result<Vec<PathBuf>> {
    let roots = workflow_roots();
    fs::create_dir_all(&roots.user_dir)
        .with_context(|| format!("create user workflows dir {}", roots.user_dir.display()))?;

    if source.is_file() {
        let identity = infer_or_validate_identity(source, system, workflow)?;
        return import_single_file(source, &roots.user_dir, &identity, force)
            .map(|path| vec![path]);
    }

    if source.is_dir() {
        if system.is_some() || workflow.is_some() {
            return Err(anyhow!(
                "--system/--name only work when importing a single workflow file"
            ));
        }
        let mut files = Vec::new();
        collect_json_files(source, &mut files)?;
        if files.is_empty() {
            return Err(anyhow!(
                "no JSON workflows found under {}",
                source.display()
            ));
        }

        let mut imported = Vec::new();
        for file in files {
            let relative = file
                .strip_prefix(source)
                .with_context(|| format!("strip prefix {}", source.display()))?;
            validate_workflow_file(&file)?;
            let dest = roots.user_dir.join(relative);
            if dest.exists() && !force {
                return Err(anyhow!(
                    "destination already exists: {} (use --force to overwrite)",
                    dest.display()
                ));
            }
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            fs::copy(&file, &dest)
                .with_context(|| format!("copy {} -> {}", file.display(), dest.display()))?;
            imported.push(dest);
        }
        imported.sort();
        return Ok(imported);
    }

    Err(anyhow!(
        "workflow source '{}' does not exist",
        source.display()
    ))
}

fn infer_or_validate_identity(
    source: &Path,
    system: Option<&str>,
    workflow: Option<&str>,
) -> Result<WorkflowIdentity> {
    match (system, workflow) {
        (Some(system), Some(workflow)) => Ok(WorkflowIdentity {
            system: slugify(system),
            workflow: slugify(workflow),
        }),
        (Some(_), None) | (None, Some(_)) => Err(anyhow!(
            "both --system and --name are required when importing a single file"
        )),
        (None, None) => read_workflow_identity_metadata(source).ok_or_else(|| {
            anyhow!(
                "single-file imports require --system and --name unless the workflow JSON declares them"
            )
        }),
    }
}

fn import_single_file(
    source: &Path,
    user_dir: &Path,
    identity: &WorkflowIdentity,
    force: bool,
) -> Result<PathBuf> {
    validate_workflow_file(source)?;
    let dest = user_dir
        .join(&identity.system)
        .join(format!("{}.json", identity.workflow));
    if dest.exists() && !force {
        return Err(anyhow!(
            "destination already exists: {} (use --force to overwrite)",
            dest.display()
        ));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::copy(source, &dest)
        .with_context(|| format!("copy {} -> {}", source.display(), dest.display()))?;
    Ok(dest)
}

fn validate_workflow_file(path: &Path) -> Result<()> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let value: Value =
        serde_json::from_str(&content).with_context(|| format!("parse {}", path.display()))?;
    if !is_manifest_value(&value) {
        return Err(anyhow!(
            "invalid workflow {}: expected rzn.workflow_manifest",
            path.display()
        ));
    }
    validate_manifest_value(&value)
        .map(|_| ())
        .map_err(|issues| {
            anyhow!(
                "invalid workflow manifest {}: {}",
                path.display(),
                issues
                    .into_iter()
                    .map(|issue| format!("{}: {}", issue.field, issue.message))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })
}

fn copy_workflow_catalog(source_root: &Path, builtin_dir: &Path) -> Result<usize> {
    let mut files = Vec::new();
    collect_json_files(source_root, &mut files)?;

    let mut copied = 0usize;
    for file in files {
        let relative = file
            .strip_prefix(source_root)
            .with_context(|| format!("strip prefix {}", source_root.display()))?;
        let rel_path = normalize_relative_path(relative);
        if should_skip_builtin_workflow_path(&rel_path) {
            continue;
        }

        let dest = builtin_dir.join(relative);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::copy(&file, &dest)
            .with_context(|| format!("copy {} -> {}", file.display(), dest.display()))?;
        copied += 1;
    }
    Ok(copied)
}

fn should_skip_builtin_workflow_path(relative_path: &str) -> bool {
    relative_path.starts_with("tests/")
        || relative_path.starts_with("test-")
        || is_fixture_relative_path(relative_path)
}

fn is_fixture_relative_path(relative_path: &str) -> bool {
    relative_path == "fixtures" || relative_path.starts_with("fixtures/")
}

fn collect_workflow_entries() -> Result<Vec<WorkflowEntry>> {
    let mut entries = collect_all_workflow_entries()?;
    entries.sort_by(|a, b| {
        source_rank(&a.source)
            .cmp(&source_rank(&b.source))
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| a.path.cmp(&b.path))
    });
    let mut seen_ids: HashSet<String> = HashSet::new();
    entries.retain(|entry| seen_ids.insert(entry.id.clone()));
    Ok(entries)
}

fn collect_all_workflow_entries() -> Result<Vec<WorkflowEntry>> {
    let roots = workflow_roots();
    let mut entries = Vec::new();

    collect_entries_from_root("user", &roots.user_dir, &mut entries)?;
    if let Some(repo_workflows_dir) = repo_local_workflows_dir() {
        collect_entries_from_root("repo", &repo_workflows_dir, &mut entries)?;
    }
    collect_entries_from_root("builtin", &roots.builtin_dir, &mut entries)?;
    Ok(entries)
}

fn collect_capability_entries(
    query: &CapabilityCatalogQuery,
    issues: &mut Vec<CatalogValidationIssue>,
) -> Result<Vec<CapabilityCatalogEntry>> {
    let records = collect_capability_manifest_records(false)?;
    Ok(collect_capability_entries_from_records(
        query, issues, records,
    ))
}

fn collect_capability_entries_from_records(
    query: &CapabilityCatalogQuery,
    issues: &mut Vec<CatalogValidationIssue>,
    records: Vec<CapabilityManifestRecord>,
) -> Vec<CapabilityCatalogEntry> {
    let system_filter = query
        .system_filter
        .as_deref()
        .map(slugify)
        .filter(|value| !value.is_empty());
    let source_filter = query
        .source_filter
        .as_deref()
        .map(slugify)
        .filter(|value| !value.is_empty());
    let mut entries = Vec::new();

    for record in records {
        if source_filter
            .as_ref()
            .map(|source| source != &record.source)
            .unwrap_or(false)
        {
            continue;
        }

        let manifest: CapabilityManifest = match serde_json::from_value(record.value.clone()) {
            Ok(manifest) => manifest,
            Err(err) => {
                issues.push(CatalogValidationIssue {
                    level: CatalogValidationLevel::Error,
                    source: record.source.clone(),
                    path: record.path.to_string_lossy().to_string(),
                    field: "manifest".to_string(),
                    message: format!("invalid manifest shape: {}", err),
                });
                continue;
            }
        };
        let manifest_version = manifest
            .schema_version
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(WORKFLOW_CONTRACT)
            .to_string();
        let system = manifest.system.as_deref().map(slugify).unwrap_or_default();
        if system.is_empty() {
            issues.push(CatalogValidationIssue {
                level: CatalogValidationLevel::Error,
                source: record.source.clone(),
                path: record.path.to_string_lossy().to_string(),
                field: "system".to_string(),
                message: "missing explicit system".to_string(),
            });
            continue;
        }
        if system_filter
            .as_ref()
            .map(|expected| expected != &system)
            .unwrap_or(false)
        {
            continue;
        }
        let capability_id = manifest
            .capability
            .as_deref()
            .map(normalize_capability_id)
            .unwrap_or_default();
        if capability_id.is_empty() {
            issues.push(CatalogValidationIssue {
                level: CatalogValidationLevel::Error,
                source: record.source.clone(),
                path: record.path.to_string_lossy().to_string(),
                field: "capability".to_string(),
                message: "manifest must declare a capability".to_string(),
            });
            continue;
        }
        if manifest.result.is_none() {
            issues.push(CatalogValidationIssue {
                level: CatalogValidationLevel::Error,
                source: record.source.clone(),
                path: record.path.to_string_lossy().to_string(),
                field: "result".to_string(),
                message: "manifest must declare an explicit result contract".to_string(),
            });
            continue;
        }
        let workflow = manifest
            .id
            .as_deref()
            .map(slugify)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| manifest_stem(&record.path));
        entries.push(CapabilityCatalogEntry {
            system: system.clone(),
            capability_id,
            workflow: workflow.clone(),
            route: format!("{}/{}", system, workflow),
            source: record.source.clone(),
            manifest_path: record.path.to_string_lossy().to_string(),
            workflow_path: record.path.to_string_lossy().to_string(),
            manifest_version,
            content_hash: record.content_hash.clone(),
            description: manifest.description.or(manifest.summary),
            effects: manifest_side_effect_classes(&manifest.side_effects),
        });
    }

    if source_filter.is_none() {
        entries = effective_capability_entries(entries);
    }

    entries.sort_by(|a, b| {
        a.system
            .cmp(&b.system)
            .then_with(|| a.capability_id.cmp(&b.capability_id))
            .then_with(|| source_rank(&a.source).cmp(&source_rank(&b.source)))
            .then_with(|| a.manifest_path.cmp(&b.manifest_path))
    });
    entries
}

fn manifest_side_effect_classes(side_effects: &[Value]) -> Vec<String> {
    let mut classes = Vec::new();
    for effect in side_effects {
        let class = effect
            .get("class")
            .and_then(Value::as_str)
            .or_else(|| effect.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(class) = class {
            let class = class.to_string();
            if !classes.contains(&class) {
                classes.push(class);
            }
        }
    }
    classes
}

fn effective_capability_entries(
    entries: Vec<CapabilityCatalogEntry>,
) -> Vec<CapabilityCatalogEntry> {
    let mut best_rank_by_route: BTreeMap<(String, String), usize> = BTreeMap::new();
    for entry in &entries {
        let key = (entry.system.clone(), entry.capability_id.clone());
        let rank = source_rank(&entry.source);
        best_rank_by_route
            .entry(key)
            .and_modify(|best| *best = (*best).min(rank))
            .or_insert(rank);
    }

    entries
        .into_iter()
        .filter(|entry| {
            let key = (entry.system.clone(), entry.capability_id.clone());
            best_rank_by_route
                .get(&key)
                .map(|best| source_rank(&entry.source) == *best)
                .unwrap_or(true)
        })
        .collect()
}

fn push_duplicate_capability_route_issues(
    entries: &[CapabilityCatalogEntry],
    issues: &mut Vec<CatalogValidationIssue>,
) {
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for entry in entries {
        let key = (entry.system.clone(), entry.capability_id.clone());
        if !seen.insert(key.clone()) {
            issues.push(CatalogValidationIssue {
                level: CatalogValidationLevel::Error,
                source: entry.source.clone(),
                path: entry.manifest_path.clone(),
                field: "capabilities".to_string(),
                message: format!("duplicate capability route '{}:{}'", key.0, key.1),
            });
        }
    }
}

fn effective_capability_manifest_record_paths(
    records: &[CapabilityManifestRecord],
) -> BTreeSet<PathBuf> {
    let mut best_rank_by_route: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut keyed_records: Vec<(PathBuf, usize, Vec<(String, String)>)> = Vec::new();
    let mut effective_paths = BTreeSet::new();

    for record in records {
        let keys = capability_manifest_route_keys(record);
        if keys.is_empty() {
            effective_paths.insert(record.path.clone());
            continue;
        }

        let rank = source_rank(&record.source);
        for key in &keys {
            best_rank_by_route
                .entry(key.clone())
                .and_modify(|best| *best = (*best).min(rank))
                .or_insert(rank);
        }
        keyed_records.push((record.path.clone(), rank, keys));
    }

    for (path, rank, keys) in keyed_records {
        if keys.iter().any(|key| {
            best_rank_by_route
                .get(key)
                .map(|best| *best == rank)
                .unwrap_or(true)
        }) {
            effective_paths.insert(path);
        }
    }

    effective_paths
}

fn capability_manifest_route_keys(record: &CapabilityManifestRecord) -> Vec<(String, String)> {
    let Ok(manifest) = serde_json::from_value::<CapabilityManifest>(record.value.clone()) else {
        return Vec::new();
    };
    let system = manifest.system.as_deref().map(slugify).unwrap_or_default();
    if system.is_empty() {
        return Vec::new();
    }

    manifest
        .capability
        .as_deref()
        .map(normalize_capability_id)
        .filter(|capability_id| !capability_id.is_empty())
        .map(|capability_id| vec![(system, capability_id)])
        .unwrap_or_default()
}

fn collect_capability_manifest_records(
    include_fixtures: bool,
) -> Result<Vec<CapabilityManifestRecord>> {
    let roots = workflow_roots();
    let mut records = Vec::new();
    if let Some(repo_workflows_dir) = repo_local_workflows_dir() {
        collect_capability_manifest_records_from_root(
            "repo",
            &repo_workflows_dir,
            &mut records,
            include_fixtures,
        )?;
    }
    collect_capability_manifest_records_from_root(
        "user",
        &roots.user_dir,
        &mut records,
        include_fixtures,
    )?;
    collect_capability_manifest_records_from_root(
        "builtin",
        &roots.builtin_dir,
        &mut records,
        include_fixtures,
    )?;
    Ok(records)
}

fn repo_local_workflows_dir() -> Option<PathBuf> {
    let workflows_dir = std::env::current_dir().ok()?.join("workflows");
    workflows_dir.is_dir().then_some(workflows_dir)
}

fn collect_capability_manifest_records_from_root(
    source: &str,
    root: &Path,
    records: &mut Vec<CapabilityManifestRecord>,
    include_fixtures: bool,
) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    let mut files = Vec::new();
    collect_json_files(root, &mut files)?;
    for file in files {
        if !include_fixtures && is_fixture_catalog_path(root, &file) {
            continue;
        }
        let content = fs::read_to_string(&file)
            .with_context(|| format!("read manifest candidate {}", file.display()))?;
        let value: Value = match serde_json::from_str(&content) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if !is_manifest_value(&value) {
            continue;
        }
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        records.push(CapabilityManifestRecord {
            source: source.to_string(),
            path: file,
            content_hash: hex::encode(hasher.finalize()),
            value,
        });
    }
    records.sort_by(|a, b| {
        source_rank(&a.source)
            .cmp(&source_rank(&b.source))
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(())
}

fn is_fixture_catalog_path(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root)
        .ok()
        .map(normalize_relative_path)
        .map(|relative| is_fixture_relative_path(&relative))
        .unwrap_or(false)
}

fn is_manifest_value(value: &Value) -> bool {
    value.get("schema_version").and_then(Value::as_str) == Some(WORKFLOW_CONTRACT)
}

fn manifest_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(slugify)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "manifest".to_string())
}

fn build_named_workflow_entries(
    entries: Vec<WorkflowEntry>,
    query: &WorkflowCatalogQuery,
) -> Vec<NamedWorkflowEntry> {
    let system_filter = query
        .system_filter
        .as_deref()
        .map(slugify)
        .filter(|value| !value.is_empty());
    let source_filter = query
        .source_filter
        .as_deref()
        .map(slugify)
        .filter(|value| !value.is_empty());
    let mut grouped: BTreeMap<(String, String), Vec<WorkflowEntry>> = BTreeMap::new();

    for entry in entries {
        grouped
            .entry((entry.system.clone(), entry.workflow.clone()))
            .or_default()
            .push(entry);
    }

    let mut results = Vec::new();
    for ((_system, _workflow), mut group) in grouped {
        group.sort_by(|a, b| {
            source_rank(&a.source)
                .cmp(&source_rank(&b.source))
                .then_with(|| a.path.cmp(&b.path))
        });

        let winning_source = group.first().map(|entry| entry.source.clone());
        let overridden_sources: Vec<String> = group
            .iter()
            .skip(1)
            .map(|entry| entry.source.clone())
            .collect();

        for (index, entry) in group.into_iter().enumerate() {
            let effective = index == 0;
            let matches_system = system_filter
                .as_ref()
                .map(|expected| &entry.system == expected)
                .unwrap_or(true);
            let matches_source = source_filter
                .as_ref()
                .map(|expected| &entry.source == expected)
                .unwrap_or(true);
            let visible = if source_filter.is_some() {
                matches_source
            } else if query.include_all_sources {
                true
            } else {
                effective
            };

            if !matches_system || !visible {
                continue;
            }

            results.push(NamedWorkflowEntry {
                id: entry.id,
                system: entry.system,
                workflow: entry.workflow,
                source: entry.source,
                path: entry.path.to_string_lossy().to_string(),
                relative_path: entry.relative_path,
                name: entry.name,
                description: entry.description,
                effective,
                shadowed_by_source: if effective {
                    None
                } else {
                    winning_source.clone()
                },
                overrides_sources: if effective {
                    overridden_sources.clone()
                } else {
                    Vec::new()
                },
            });
        }
    }

    results.sort_by(|a, b| {
        a.system
            .cmp(&b.system)
            .then_with(|| a.workflow.cmp(&b.workflow))
            .then_with(|| source_rank(&a.source).cmp(&source_rank(&b.source)))
            .then_with(|| a.path.cmp(&b.path))
    });
    results
}

fn collect_entries_from_root(
    source: &str,
    root: &Path,
    entries: &mut Vec<WorkflowEntry>,
) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }

    let mut files = Vec::new();
    collect_json_files(root, &mut files)?;
    for file in files {
        let relative = file
            .strip_prefix(root)
            .with_context(|| format!("strip prefix {}", root.display()))?;
        let relative_path = normalize_relative_path(relative);
        if is_fixture_relative_path(&relative_path) {
            continue;
        }

        let content = fs::read_to_string(&file)
            .with_context(|| format!("read workflow manifest {}", file.display()))?;
        let value: Value = serde_json::from_str(&content)
            .with_context(|| format!("parse workflow manifest {}", file.display()))?;
        if !is_manifest_value(&value) {
            continue;
        }
        let manifest = validate_manifest_value(&value).map_err(|issues| {
            anyhow!(
                "invalid workflow manifest {}: {}",
                file.display(),
                issues
                    .into_iter()
                    .map(|issue| format!("{}: {}", issue.field, issue.message))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })?;
        let workflow = manifest
            .id
            .split_once('/')
            .map(|(_, workflow)| normalize_workflow_name(&manifest.system, workflow))
            .filter(|workflow| !workflow.is_empty())
            .unwrap_or_else(|| manifest_stem(&file));
        let id = format!("{}/{}", slugify(&manifest.system), workflow);
        let relative_stem = strip_json_ext(&relative_path).to_string();
        entries.push(WorkflowEntry {
            id,
            system: slugify(&manifest.system),
            workflow,
            source: source.to_string(),
            path: file,
            relative_path,
            relative_stem,
            name: Some(manifest.name),
            description: manifest.description.or(manifest.summary),
        });
    }
    entries.sort_by(|a, b| {
        source_rank(&a.source)
            .cmp(&source_rank(&b.source))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(())
}

fn normalize_workflow_name(system: &str, raw: &str) -> String {
    let raw_slug = slugify(raw);
    if raw_slug.is_empty() {
        return raw_slug;
    }
    let prefix = format!("{}-", system);
    raw_slug
        .strip_prefix(&prefix)
        .filter(|value| !value.is_empty())
        .unwrap_or(&raw_slug)
        .to_string()
}

fn collect_json_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut children: Vec<PathBuf> = fs::read_dir(root)
        .with_context(|| format!("read dir {}", root.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    children.sort();

    for path in children {
        if path.is_dir() {
            collect_json_files(&path, out)?;
            continue;
        }
        if path
            .extension()
            .and_then(|s| s.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("json"))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
    Ok(())
}

fn read_workflow_identity_metadata(path: &Path) -> Option<WorkflowIdentity> {
    let content = fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&content).ok()?;

    let from_id = value
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|id| id.split_once('/'))
        .map(|(system, workflow)| WorkflowIdentity {
            system: slugify(system),
            workflow: slugify(workflow),
        });
    match from_id {
        Some(identity) if !identity.system.is_empty() && !identity.workflow.is_empty() => {
            Some(identity)
        }
        _ => None,
    }
}

fn source_rank(source: &str) -> usize {
    match source {
        "repo" => 0,
        "user" => 1,
        "builtin" => 2,
        _ => 3,
    }
}

fn normalize_relative_path(path: &Path) -> String {
    let parts: Vec<String> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    parts.join("/")
}

fn normalize_pathish(input: &str) -> String {
    input
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_matches('/')
        .to_ascii_lowercase()
}

fn strip_json_ext(input: &str) -> &str {
    input
        .strip_suffix(".json")
        .or_else(|| input.strip_suffix(".JSON"))
        .unwrap_or(input)
}

fn expand_tilde(input: &str) -> PathBuf {
    if let Some(rest) = input.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(input)
}

pub fn slugify(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() || matches!(ch, '-' | '_' | '/' | '.') {
            out.push('-');
        }
    }
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_string()
}

fn normalize_capability_id(input: &str) -> String {
    input
        .trim()
        .split('.')
        .map(slugify)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

fn is_true(value: &bool) -> bool {
    *value
}

#[cfg(test)]
mod tests {
    use super::{
        build_named_workflow_entries, collect_capability_entries_from_records,
        collect_capability_manifest_records_from_root, collect_json_files,
        detect_catalog_source_root, effective_capability_entries,
        effective_capability_manifest_record_paths, find_matching_workflow_entry,
        install_builtin_catalog_to_root, is_fixture_catalog_path, is_manifest_value,
        normalize_capability_id, push_duplicate_capability_route_issues,
        resolve_workflow_reference, validate_manifest_contracts, validate_workflow_file,
        CapabilityCatalogEntry, CapabilityCatalogQuery, CapabilityManifestRecord,
        CatalogValidationLevel, WorkflowCatalogQuery, WorkflowEntry,
    };
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{}_{}", prefix, Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    const WORKFLOW_VALIDATE_ALL_EXCLUDED_GLOBS: &[(&str, &str)] = &[(
        "workflows/fixtures/**",
        "synthetic manifest fixtures intentionally model narrow test cases",
    )];

    #[test]
    fn workflow_add_validation_accepts_current_manifests() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        validate_workflow_file(&repo_root.join("workflows/chatgpt/chatgpt_send.json"))
            .expect("current workflow manifests must be importable into the user catalog");
    }

    #[test]
    fn workflow_validate_all_manifests_under_workflows_glob() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let workflows_root = repo_root.join("workflows");
        let mut json_files = Vec::new();
        collect_json_files(&workflows_root, &mut json_files).expect("collect workflow JSON files");

        let mut globbed_manifest_paths = BTreeSet::new();
        for path in json_files {
            if is_fixture_catalog_path(&workflows_root, &path) {
                continue;
            }
            let content = fs::read_to_string(&path).expect("read workflow JSON");
            let value: serde_json::Value = serde_json::from_str(&content).unwrap_or_else(|err| {
                panic!("parse workflow JSON {}: {err}", path.display());
            });
            if is_manifest_value(&value) {
                globbed_manifest_paths.insert(path);
            }
        }

        let mut records = Vec::new();
        collect_capability_manifest_records_from_root("repo", &workflows_root, &mut records, false)
            .expect("collect manifest records");

        let collected_manifest_paths = records
            .iter()
            .map(|record| record.path.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            globbed_manifest_paths, collected_manifest_paths,
            "manifest collector must cover the workflows/**/*.json glob; excluded globs: {:?}",
            WORKFLOW_VALIDATE_ALL_EXCLUDED_GLOBS
        );
        assert!(
            !records.is_empty(),
            "expected at least one manifest under workflows/"
        );

        let mut issues = Vec::new();
        validate_manifest_contracts(&records, &mut issues);
        assert!(
            issues.is_empty(),
            "workflow manifest validation failed; excluded globs: {:?}; issues: {:#?}",
            WORKFLOW_VALIDATE_ALL_EXCLUDED_GLOBS,
            issues
        );
    }

    #[test]
    fn detect_catalog_source_root_accepts_nested_source_root() {
        let outer = temp_dir("rzn_catalog_outer");
        let inner = outer.join("rzn-browser-main");
        fs::create_dir_all(inner.join("workflows")).expect("create workflows");

        let detected = detect_catalog_source_root(&outer).expect("detect source root");
        assert_eq!(detected, inner);

        fs::remove_dir_all(outer).expect("cleanup");
    }

    #[test]
    fn install_builtin_catalog_skips_tests_and_fixtures() {
        let repo_root = temp_dir("rzn_catalog_repo");
        let builtin_dir = temp_dir("rzn_catalog_builtin");
        let user_dir = temp_dir("rzn_catalog_user");

        fs::create_dir_all(repo_root.join("workflows").join("google")).expect("create google dir");
        fs::create_dir_all(repo_root.join("workflows").join("tests")).expect("create tests dir");
        fs::create_dir_all(
            repo_root
                .join("workflows")
                .join("fixtures")
                .join("manifest"),
        )
        .expect("create fixtures dir");

        fs::write(
            repo_root
                .join("workflows")
                .join("google")
                .join("google-search.json"),
            r#"{"schema_version":"rzn.workflow_manifest","id":"google/search","name":"Search","version":"1.0.0","system":"google","capability":"google.search","steps":[{"id":"s","action":{"kind":"wait_for_timeout"}}]}"#,
        )
        .expect("write workflow");
        fs::write(
            repo_root.join("workflows").join("tests").join("debug.json"),
            r#"{}"#,
        )
        .expect("write test workflow");
        fs::write(repo_root.join("workflows").join("test-basic.json"), r#"{}"#)
            .expect("write top-level test workflow");
        fs::write(
            repo_root
                .join("workflows")
                .join("fixtures")
                .join("manifest")
                .join("local_read_text.manifest.json"),
            r#"{"schema_version":"rzn.workflow_manifest","id":"fixture/local-read-text","name":"Fixture","version":"1.0.0","system":"fixture.local","capability":"fixture.local.read_text","result":{}}"#,
        )
        .expect("write fixture manifest");
        let summary = install_builtin_catalog_to_root(&repo_root, &builtin_dir, &user_dir)
            .expect("install builtin catalog");

        assert_eq!(summary.workflow_files, 1);
        assert!(builtin_dir
            .join("google")
            .join("google-search.json")
            .exists());
        assert!(!builtin_dir.join("tests").join("debug.json").exists());
        assert!(!builtin_dir.join("test-basic.json").exists());
        assert!(!builtin_dir
            .join("fixtures")
            .join("manifest")
            .join("local_read_text.manifest.json")
            .exists());
        assert!(!builtin_dir.join("examples").exists());

        fs::remove_dir_all(repo_root).expect("cleanup repo");
        fs::remove_dir_all(builtin_dir).expect("cleanup builtin");
        fs::remove_dir_all(user_dir).expect("cleanup user");
    }

    fn sample_entry(system: &str, workflow: &str, source: &str, suffix: &str) -> WorkflowEntry {
        WorkflowEntry {
            id: format!("{}/{}", system, workflow),
            system: system.to_string(),
            workflow: workflow.to_string(),
            source: source.to_string(),
            path: PathBuf::from(format!("/tmp/{}-{}-{}.json", system, workflow, suffix)),
            relative_path: format!("{}/{}.json", system, workflow),
            relative_stem: format!("{}/{}", system, workflow),
            name: None,
            description: None,
        }
    }

    #[test]
    fn flat_workflow_alias_does_not_match() {
        let entries = vec![sample_entry("chatgpt", "send", "builtin", "exact")];

        assert!(find_matching_workflow_entry(&entries, "chatgpt-send", "chatgpt-send").is_none());
    }

    fn sample_capability_entry(
        system: &str,
        capability_id: &str,
        source: &str,
        suffix: &str,
    ) -> CapabilityCatalogEntry {
        CapabilityCatalogEntry {
            system: system.to_string(),
            capability_id: capability_id.to_string(),
            workflow: "send".to_string(),
            route: format!("{}/send", system),
            source: source.to_string(),
            manifest_path: format!("/tmp/{source}-{suffix}.json"),
            workflow_path: format!("/tmp/{source}-{suffix}.json"),
            manifest_version: "rzn.workflow_manifest".to_string(),
            content_hash: suffix.to_string(),
            description: None,
            effects: Vec::new(),
        }
    }

    #[test]
    fn manifest_detection_requires_declared_contract_fields() {
        let manifest = serde_json::json!({
            "schema_version": "rzn.workflow_manifest",
            "id": "chatgpt/read",
            "name": "Read",
            "version": "1.0.0",
            "system": "chatgpt",
            "capability": "assistant.conversation.read"
        });
        let workflow = serde_json::json!({"id": "chatgpt/read"});

        assert!(is_manifest_value(&manifest));
        assert!(!is_manifest_value(&workflow));
    }

    #[test]
    fn capability_discovery_skips_fixture_manifests() {
        let root = temp_dir("rzn_catalog_fixture_records");
        fs::create_dir_all(root.join("chatgpt")).expect("create chatgpt dir");
        fs::create_dir_all(root.join("fixtures").join("manifest")).expect("create fixtures dir");
        fs::write(
            root.join("chatgpt").join("read.json"),
            r#"{
              "schema_version": "rzn.workflow_manifest",
              "id": "chatgpt/read",
              "name": "Read",
              "version": "1.0.0",
              "system": "chatgpt",
              "capability": "assistant.conversation.read",
              "result": {}
            }"#,
        )
        .expect("write production manifest");
        fs::write(
            root.join("fixtures")
                .join("manifest")
                .join("local_read_text.manifest.json"),
            r#"{"schema_version":"rzn.workflow_manifest","id":"fixture/local-read-text","name":"Fixture","version":"1.0.0","system":"fixture.local","capability":"fixture.local.read_text","result":{}}"#,
        )
        .expect("write fixture manifest");
        let mut records = Vec::new();
        collect_capability_manifest_records_from_root("repo", &root, &mut records, false)
            .expect("collect records");
        assert_eq!(records.len(), 1);
        assert!(records[0].path.ends_with("chatgpt/read.json"));

        let mut records_with_fixtures = Vec::new();
        collect_capability_manifest_records_from_root(
            "repo",
            &root,
            &mut records_with_fixtures,
            true,
        )
        .expect("collect records with fixtures");
        assert_eq!(records_with_fixtures.len(), 2);

        let mut issues = Vec::new();
        let entries = collect_capability_entries_from_records(
            &CapabilityCatalogQuery::default(),
            &mut issues,
            records,
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].system, "chatgpt");
        assert_eq!(entries[0].capability_id, "assistant.conversation.read");
        assert!(issues.is_empty());

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn capability_ids_preserve_dot_namespace_and_slug_parts() {
        assert_eq!(
            normalize_capability_id("Assistant Conversation.Read Latest"),
            "assistant-conversation.read-latest"
        );
    }

    #[test]
    fn capability_precedence_shadows_lower_priority_duplicate_routes() {
        let entries = effective_capability_entries(vec![
            sample_capability_entry("chatgpt", "chatgpt.send", "builtin", "builtin"),
            sample_capability_entry("chatgpt", "chatgpt.send", "repo", "repo"),
            sample_capability_entry("chatgpt", "chatgpt.read", "builtin", "read"),
        ]);

        assert_eq!(entries.len(), 2);
        assert!(entries
            .iter()
            .any(|entry| entry.capability_id == "chatgpt.send" && entry.source == "repo"));
        assert!(!entries
            .iter()
            .any(|entry| entry.capability_id == "chatgpt.send" && entry.source == "builtin"));
    }

    #[test]
    fn capability_precedence_preserves_same_priority_duplicate_failures() {
        let entries = effective_capability_entries(vec![
            sample_capability_entry("chatgpt", "chatgpt.send", "repo", "a"),
            sample_capability_entry("chatgpt", "chatgpt.send", "repo", "b"),
            sample_capability_entry("chatgpt", "chatgpt.send", "builtin", "shadowed"),
        ]);

        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| entry.source == "repo"));

        let mut issues = Vec::new();
        push_duplicate_capability_route_issues(&entries, &mut issues);
        assert_eq!(issues.len(), 1);
        assert!(matches!(issues[0].level, CatalogValidationLevel::Error));
        assert!(issues[0]
            .message
            .contains("duplicate capability route 'chatgpt:chatgpt.send'"));
    }

    #[test]
    fn top_level_manifest_capability_entry_includes_description_and_effects() {
        let record = CapabilityManifestRecord {
            source: "repo".to_string(),
            path: PathBuf::from("/tmp/workflows/google/google-search.json"),
            content_hash: "hash".to_string(),
            value: serde_json::json!({
                "schema_version": "rzn.workflow_manifest",
                "id": "google/search",
                "name": "Google Search",
                "version": "1.0.0",
                "system": "google",
                "capability": "google.search",
                "summary": "Search Google.",
                "description": "Run a Google search and extract results.",
                "side_effects": [
                    { "class": "browser_state" },
                    { "class": "read_only" },
                    { "class": "read_only" }
                ],
                "result": {}
            }),
        };
        let mut issues = Vec::new();
        let entries = collect_capability_entries_from_records(
            &CapabilityCatalogQuery {
                system_filter: Some("google".to_string()),
                source_filter: None,
            },
            &mut issues,
            vec![record],
        );

        assert!(issues.is_empty());
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].description.as_deref(),
            Some("Run a Google search and extract results.")
        );
        assert_eq!(entries[0].effects, vec!["browser_state", "read_only"]);
    }

    #[test]
    fn strict_contract_validation_targets_effective_manifest_records() {
        let user = CapabilityManifestRecord {
            source: "user".to_string(),
            path: PathBuf::from("/tmp/user/chatgpt_send.json"),
            content_hash: "user".to_string(),
            value: serde_json::json!({
                "schema_version": "rzn.workflow_manifest",
                "id": "chatgpt/send",
                "name": "ChatGPT Send",
                "version": "1.0.0",
                "system": "chatgpt",
                "capability": "chatgpt.send",
                "result": {}
            }),
        };
        let shadowed_builtin = CapabilityManifestRecord {
            source: "builtin".to_string(),
            path: PathBuf::from("/tmp/builtin/chatgpt_send.json"),
            content_hash: "builtin".to_string(),
            value: serde_json::json!({
                "schema_version": "rzn.workflow_manifest",
                "id": "chatgpt/send",
                "name": "ChatGPT Send",
                "version": "1.0.0",
                "system": "chatgpt",
                "capability": "chatgpt.send",
                "help": {"summary": "bad lower priority copy", "parameters": []},
                "result": {}
            }),
        };
        let repo_duplicate_a = CapabilityManifestRecord {
            source: "repo".to_string(),
            path: PathBuf::from("/tmp/repo/a.json"),
            content_hash: "repo-a".to_string(),
            value: serde_json::json!({
                "schema_version": "rzn.workflow_manifest",
                "id": "chatgpt/read-a",
                "name": "ChatGPT Read A",
                "version": "1.0.0",
                "system": "chatgpt",
                "capability": "chatgpt.read",
                "result": {}
            }),
        };
        let repo_duplicate_b = CapabilityManifestRecord {
            source: "repo".to_string(),
            path: PathBuf::from("/tmp/repo/b.json"),
            content_hash: "repo-b".to_string(),
            value: serde_json::json!({
                "schema_version": "rzn.workflow_manifest",
                "id": "chatgpt/read-b",
                "name": "ChatGPT Read B",
                "version": "1.0.0",
                "system": "chatgpt",
                "capability": "chatgpt.read",
                "result": {}
            }),
        };

        let effective_paths = effective_capability_manifest_record_paths(&[
            user.clone(),
            shadowed_builtin.clone(),
            repo_duplicate_a.clone(),
            repo_duplicate_b.clone(),
        ]);

        assert!(effective_paths.contains(&user.path));
        assert!(!effective_paths.contains(&shadowed_builtin.path));
        assert!(effective_paths.contains(&repo_duplicate_a.path));
        assert!(effective_paths.contains(&repo_duplicate_b.path));
    }

    #[test]
    fn workflow_query_hides_shadowed_entries_by_default() {
        let entries = build_named_workflow_entries(
            vec![
                sample_entry("google", "search", "builtin", "builtin"),
                sample_entry("google", "search", "user", "user"),
                sample_entry("google", "maps", "builtin", "maps"),
            ],
            &WorkflowCatalogQuery::default(),
        );

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "google/maps");
        assert!(entries[0].effective);
        assert_eq!(entries[1].id, "google/search");
        assert_eq!(entries[1].source, "user");
        assert_eq!(entries[1].overrides_sources, vec!["builtin"]);
    }

    #[test]
    fn workflow_query_source_filter_includes_shadowed_entries_from_that_source() {
        let entries = build_named_workflow_entries(
            vec![
                sample_entry("google", "search", "builtin", "builtin"),
                sample_entry("google", "search", "user", "user"),
                sample_entry("google", "maps", "builtin", "maps"),
            ],
            &WorkflowCatalogQuery {
                system_filter: None,
                source_filter: Some("builtin".to_string()),
                include_all_sources: false,
            },
        );

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "google/maps");
        assert!(entries[0].effective);
        assert_eq!(entries[1].id, "google/search");
        assert!(!entries[1].effective);
        assert_eq!(entries[1].shadowed_by_source.as_deref(), Some("user"));
    }

    #[test]
    fn resolve_workflow_reference_explains_when_reference_is_only_a_system() {
        let original_builtin = std::env::var("RZN_BUILTIN_WORKFLOWS_DIR").ok();
        let original_user = std::env::var("RZN_WORKFLOWS_DIR").ok();
        let original_runtime = std::env::var("RZN_RUNTIME_DIR").ok();
        let temp_runtime = temp_dir("rzn_catalog_runtime");
        let builtin_dir = temp_runtime.join("builtin");
        let user_dir = temp_runtime.join("user");
        fs::create_dir_all(builtin_dir.join("chatgpt")).expect("create builtin chatgpt dir");
        fs::create_dir_all(&user_dir).expect("create user dir");
        fs::write(
            builtin_dir.join("chatgpt").join("chatgpt_continue_chat.json"),
            r#"{"schema_version":"rzn.workflow_manifest","id":"chatgpt/continue-chat","name":"Continue Chat","description":"Continue chat","version":"1.0.0","system":"chatgpt","capability":"chatgpt.continue_chat","steps":[{"id":"s","action":{"kind":"wait_for_timeout"}}]}"#,
        )
        .expect("write workflow");

        std::env::set_var("RZN_RUNTIME_DIR", &temp_runtime);
        std::env::set_var("RZN_BUILTIN_WORKFLOWS_DIR", &builtin_dir);
        std::env::set_var("RZN_WORKFLOWS_DIR", &user_dir);

        let err = resolve_workflow_reference("chatgpt").expect_err("system-only ref should fail");
        let msg = err.to_string();
        assert!(msg.contains("'chatgpt' is a workflow system"));
        assert!(msg.contains("rzn-browser workflow list chatgpt"));
        assert!(msg.contains("chatgpt continue-chat"));
        assert!(msg.contains("Continue Chat"));

        if let Some(value) = original_runtime {
            std::env::set_var("RZN_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("RZN_RUNTIME_DIR");
        }
        if let Some(value) = original_builtin {
            std::env::set_var("RZN_BUILTIN_WORKFLOWS_DIR", value);
        } else {
            std::env::remove_var("RZN_BUILTIN_WORKFLOWS_DIR");
        }
        if let Some(value) = original_user {
            std::env::set_var("RZN_WORKFLOWS_DIR", value);
        } else {
            std::env::remove_var("RZN_WORKFLOWS_DIR");
        }

        fs::remove_dir_all(temp_runtime).expect("cleanup runtime");
    }
}
