use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
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
    pub legacy_user_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuiltinCatalogInstallSummary {
    pub builtin_dir: String,
    pub workflow_files: usize,
    pub example_files: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct NamedWorkflowEntry {
    pub id: String,
    pub system: String,
    pub workflow: String,
    pub legacy_alias: String,
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

#[derive(Debug, Clone)]
struct WorkflowEntry {
    id: String,
    system: String,
    workflow: String,
    legacy_alias: String,
    source: String,
    path: PathBuf,
    relative_path: String,
    relative_stem: String,
    file_name: String,
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
    let legacy_user_dir = dirs::home_dir().map(|home| home.join(".rzn").join("workflows"));

    WorkflowRoots {
        runtime_dir,
        builtin_dir,
        user_dir,
        legacy_user_dir,
    }
}

pub fn default_runtime_dir() -> PathBuf {
    if let Some(dir) = dirs::data_local_dir() {
        return dir.join("RZN");
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".rzn");
    }
    PathBuf::from(".rzn")
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
    let backup_dir = unique_sibling_dir(builtin_dir, "old");
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

    let install_result = (|| -> Result<(usize, usize)> {
        let workflow_files = copy_workflow_catalog(&workflows_src, &staging_dir)?;
        let example_files = copy_browser_examples(
            &source_root.join("examples").join("browser_automation"),
            &staging_dir.join("examples").join("browser_automation"),
        )?;
        Ok((workflow_files, example_files))
    })();

    let (workflow_files, example_files) = match install_result {
        Ok(counts) => counts,
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
        example_files,
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
    for entry in &entries {
        if normalized_input == entry.id
            || normalized_no_ext == entry.id
            || normalized_input == entry.relative_path
            || normalized_no_ext == entry.relative_stem
            || normalized_input == entry.file_name
            || normalized_no_ext == strip_json_ext(&entry.file_name)
            || flat_alias == entry.legacy_alias
            || flat_alias == slugify(&entry.id)
        {
            return Ok(entry.path.clone());
        }
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
            "'{}' is a workflow system, not a runnable workflow.\nTry `rzn-browser list {}` to inspect it, or run one of:\n{}",
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
    let has_sequences = value
        .get("browser_automation")
        .and_then(|v| v.get("sequences"))
        .and_then(|v| v.as_array())
        .map(|sequences| !sequences.is_empty())
        .unwrap_or(false);
    if !has_sequences {
        return Err(anyhow!(
            "invalid workflow {}: expected browser_automation.sequences[]",
            path.display()
        ));
    }
    Ok(())
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
        if rel_path.starts_with("tests/") || rel_path.starts_with("test-") {
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

fn copy_browser_examples(source_root: &Path, dest_root: &Path) -> Result<usize> {
    if !source_root.is_dir() {
        return Ok(0);
    }

    let mut files = Vec::new();
    collect_json_files(source_root, &mut files)?;

    let mut copied = 0usize;
    for file in files {
        let relative = file
            .strip_prefix(source_root)
            .with_context(|| format!("strip prefix {}", source_root.display()))?;
        let dest = dest_root.join(relative);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::copy(&file, &dest)
            .with_context(|| format!("copy {} -> {}", file.display(), dest.display()))?;
        copied += 1;
    }

    Ok(copied)
}

fn collect_workflow_entries() -> Result<Vec<WorkflowEntry>> {
    let mut entries = collect_all_workflow_entries()?;
    let mut seen_ids: HashSet<String> = HashSet::new();
    entries.retain(|entry| seen_ids.insert(entry.id.clone()));
    Ok(entries)
}

fn collect_all_workflow_entries() -> Result<Vec<WorkflowEntry>> {
    let roots = workflow_roots();
    let mut entries = Vec::new();

    collect_entries_from_root("user", &roots.user_dir, &mut entries)?;
    collect_entries_from_root("builtin", &roots.builtin_dir, &mut entries)?;
    if let Some(legacy_dir) = roots.legacy_user_dir.as_ref() {
        if legacy_dir != &roots.user_dir {
            collect_entries_from_root("legacy", legacy_dir, &mut entries)?;
        }
    }
    Ok(entries)
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
                legacy_alias: entry.legacy_alias,
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
        let relative_stem = strip_json_ext(&relative_path).to_string();
        let file_name = file
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let Some(identity) = infer_identity_from_relative_path(relative) else {
            continue;
        };
        let id = format!("{}/{}", identity.system, identity.workflow);
        let legacy_alias = format!("{}-{}", identity.system, identity.workflow);
        let (name, description) = read_workflow_metadata(&file);

        entries.push(WorkflowEntry {
            id,
            system: identity.system,
            workflow: identity.workflow,
            legacy_alias,
            source: source.to_string(),
            path: file,
            relative_path,
            relative_stem,
            file_name,
            name,
            description,
        });
    }
    entries.sort_by(|a, b| {
        source_rank(&a.source)
            .cmp(&source_rank(&b.source))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(())
}

fn infer_identity_from_relative_path(relative: &Path) -> Option<WorkflowIdentity> {
    let components: Vec<String> = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    if components.is_empty() {
        return None;
    }
    let file_stem = Path::new(components.last()?).file_stem()?.to_string_lossy();
    if components.len() == 1 {
        let stem = file_stem.to_string();
        let (system, workflow) = stem.split_once('-')?;
        let system = slugify(system);
        let workflow = normalize_workflow_name(&system, workflow);
        if system.is_empty() || workflow.is_empty() {
            return None;
        }
        return Some(WorkflowIdentity { system, workflow });
    }

    let system_component =
        if components.first()?.eq_ignore_ascii_case("generated") && components.len() >= 3 {
            components.get(1)?
        } else if components.len() >= 2 {
            components.first()?
        } else {
            return None;
        };
    let system = slugify(system_component);
    let workflow = normalize_workflow_name(&system, &file_stem);
    if system.is_empty() {
        return None;
    }
    Some(WorkflowIdentity { system, workflow })
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

fn read_workflow_metadata(path: &Path) -> (Option<String>, Option<String>) {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return (None, None),
    };
    let value: Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(_) => return (None, None),
    };
    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let description = value
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    (name, description)
}

fn read_workflow_identity_metadata(path: &Path) -> Option<WorkflowIdentity> {
    let content = fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&content).ok()?;

    let from_fields = value
        .get("system")
        .and_then(|v| v.as_str())
        .zip(value.get("workflow").and_then(|v| v.as_str()))
        .map(|(system, workflow)| WorkflowIdentity {
            system: slugify(system),
            workflow: slugify(workflow),
        });
    if let Some(identity) = from_fields {
        if !identity.system.is_empty() && !identity.workflow.is_empty() {
            return Some(identity);
        }
    }

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
        "user" => 0,
        "builtin" => 1,
        "legacy" => 2,
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

fn is_true(value: &bool) -> bool {
    *value
}

#[cfg(test)]
mod tests {
    use super::{
        build_named_workflow_entries, detect_catalog_source_root, install_builtin_catalog_to_root,
        resolve_workflow_reference, WorkflowCatalogQuery, WorkflowEntry,
    };
    use std::fs;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{}_{}", prefix, Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn detect_catalog_source_root_accepts_archive_layout() {
        let outer = temp_dir("rzn_catalog_outer");
        let inner = outer.join("rzn-browser-main");
        fs::create_dir_all(inner.join("workflows")).expect("create workflows");

        let detected = detect_catalog_source_root(&outer).expect("detect source root");
        assert_eq!(detected, inner);

        fs::remove_dir_all(outer).expect("cleanup");
    }

    #[test]
    fn install_builtin_catalog_copies_examples_and_skips_tests() {
        let repo_root = temp_dir("rzn_catalog_repo");
        let builtin_dir = temp_dir("rzn_catalog_builtin");
        let user_dir = temp_dir("rzn_catalog_user");

        fs::create_dir_all(repo_root.join("workflows").join("google")).expect("create google dir");
        fs::create_dir_all(repo_root.join("workflows").join("tests")).expect("create tests dir");
        fs::create_dir_all(repo_root.join("examples").join("browser_automation"))
            .expect("create examples dir");

        fs::write(
            repo_root
                .join("workflows")
                .join("google")
                .join("google-search.json"),
            r#"{"browser_automation":{"sequences":[{"name":"x","steps":[]}]}}"#,
        )
        .expect("write workflow");
        fs::write(
            repo_root.join("workflows").join("tests").join("debug.json"),
            r#"{"browser_automation":{"sequences":[{"name":"x","steps":[]}]}}"#,
        )
        .expect("write test workflow");
        fs::write(
            repo_root.join("workflows").join("test-basic.json"),
            r#"{"browser_automation":{"sequences":[{"name":"x","steps":[]}]}}"#,
        )
        .expect("write top-level test workflow");
        fs::write(
            repo_root
                .join("examples")
                .join("browser_automation")
                .join("search_google.json"),
            r#"{"browser_automation":{"sequences":[{"name":"x","steps":[]}]}}"#,
        )
        .expect("write example workflow");

        let summary = install_builtin_catalog_to_root(&repo_root, &builtin_dir, &user_dir)
            .expect("install builtin catalog");

        assert_eq!(summary.workflow_files, 1);
        assert_eq!(summary.example_files, 1);
        assert!(builtin_dir
            .join("google")
            .join("google-search.json")
            .exists());
        assert!(!builtin_dir.join("tests").join("debug.json").exists());
        assert!(!builtin_dir.join("test-basic.json").exists());
        assert!(builtin_dir
            .join("examples")
            .join("browser_automation")
            .join("search_google.json")
            .exists());

        fs::remove_dir_all(repo_root).expect("cleanup repo");
        fs::remove_dir_all(builtin_dir).expect("cleanup builtin");
        fs::remove_dir_all(user_dir).expect("cleanup user");
    }

    fn sample_entry(system: &str, workflow: &str, source: &str, suffix: &str) -> WorkflowEntry {
        WorkflowEntry {
            id: format!("{}/{}", system, workflow),
            system: system.to_string(),
            workflow: workflow.to_string(),
            legacy_alias: format!("{}-{}", system, workflow),
            source: source.to_string(),
            path: PathBuf::from(format!("/tmp/{}-{}-{}.json", system, workflow, suffix)),
            relative_path: format!("{}/{}.json", system, workflow),
            relative_stem: format!("{}/{}", system, workflow),
            file_name: format!("{}-{}.json", system, workflow),
            name: None,
            description: None,
        }
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
            builtin_dir.join("chatgpt").join("chatgpt_continue_chat_v1.json"),
            r#"{"id":"chatgpt/continue-chat-v1","name":"Continue Chat","description":"Continue chat","browser_automation":{"sequences":[{"name":"main","description":"Main","required_variables":[],"steps":[]}]}}"#,
        )
        .expect("write workflow");

        std::env::set_var("RZN_RUNTIME_DIR", &temp_runtime);
        std::env::set_var("RZN_BUILTIN_WORKFLOWS_DIR", &builtin_dir);
        std::env::set_var("RZN_WORKFLOWS_DIR", &user_dir);

        let err = resolve_workflow_reference("chatgpt").expect_err("system-only ref should fail");
        let msg = err.to_string();
        assert!(msg.contains("'chatgpt' is a workflow system"));
        assert!(msg.contains("rzn-browser list chatgpt"));
        assert!(msg.contains("chatgpt continue-chat-v1"));
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
