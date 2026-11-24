//! Keeps read workflows on the item contract (`result.contract`; backend
//! `docs/plans/browser-fleet/design.md` section 6):
//!
//! - every manifest declares `result.contract`: `rzn.item/v2`, `rzn.items/v2` or `none`;
//! - an item contract ships `<system>/samples/<workflow file stem>.sample.json`, whose `output`
//!   passes the vendored rzn-tools schema and `check()` rules (`rzn_core::item_contract`);
//! - a family with an item contract ships `<system>/describe.json` (`rzn.describe/v2`) whose
//!   tools are workflows of that family;
//! - read workflows (no `external_write`) still on `none` never exceed one baseline, which only
//!   goes down;
//! - read workflows make no request of their own (section 11 ruling 7): no `fetch(`,
//!   `XMLHttpRequest`, `navigator.sendBeacon`, `new WebSocket` or `EventSource` in a step, and
//!   no `same_origin_request` step. They keep what the page loads (`capture_responses`). The
//!   read workflows still doing so never exceed one baseline, which only goes down.

use rzn_contracts::workflow::OutputContract;
use rzn_core::item_contract::check_output;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// Read workflows (no `external_write`) still on `result.contract: none`. Lower it in the change
/// that moves one; never raise it. The family and ratchet rules run in this module's repo test.
#[cfg(test)]
pub(crate) const READ_WORKFLOWS_ON_NONE_BASELINE: usize = 87;

/// Read workflows that still make requests of their own (see [`own_requests`]). Lower it in
/// the change that moves one to `capture_responses`; never raise it.
#[cfg(test)]
pub(crate) const READ_WORKFLOWS_MAKING_REQUESTS_BASELINE: usize = 11;

/// Script calls that issue a request from the page.
#[cfg(test)]
const OWN_REQUEST_CALLS: [&str; 5] = [
    "fetch(",
    "XMLHttpRequest",
    "navigator.sendBeacon",
    "new WebSocket",
    "EventSource",
];

pub(crate) const SAMPLES_DIR: &str = "samples";
#[cfg(test)]
pub(crate) const DESCRIBE_FILE: &str = "describe.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContractLintIssue {
    pub field: String,
    pub message: String,
}

impl ContractLintIssue {
    fn new(field: &str, message: impl Into<String>) -> Self {
        Self {
            field: field.to_string(),
            message: message.into(),
        }
    }
}

/// The contract a manifest declares; `None` when it is missing or not a known value.
pub(crate) fn declared_contract(manifest: &Value) -> Option<OutputContract> {
    serde_json::from_value(manifest.pointer("/result/contract")?.clone()).ok()
}

pub(crate) fn sample_path(manifest_path: &Path) -> PathBuf {
    let stem = manifest_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("workflow");
    manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(SAMPLES_DIR)
        .join(format!("{stem}.sample.json"))
}

/// A manifest with an `external_write` side effect (post, reply, send). Everything else reads.
pub(crate) fn writes_externally(manifest: &Value) -> bool {
    manifest["side_effects"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|effect| {
            effect.as_str().or_else(|| effect["class"].as_str()) == Some("external_write")
        })
}

/// The per-manifest rules: contract declared, and for an item contract a passing sample.
pub(crate) fn lint_manifest(manifest_path: &Path, manifest: &Value) -> Vec<ContractLintIssue> {
    let mut issues = Vec::new();
    if manifest.pointer("/result/contract").is_none() {
        issues.push(ContractLintIssue::new(
            "result.contract",
            "declare result.contract: rzn.item/v2, rzn.items/v2, or none",
        ));
        return issues;
    }
    // An unknown value is reported by `validate_manifest_value`.
    let Some(contract) = declared_contract(manifest).filter(|c| c.is_item()) else {
        return issues;
    };
    if writes_externally(manifest) {
        issues.push(ContractLintIssue::new(
            "result.contract",
            "a workflow with an external_write side effect keeps free-form output (contract none)",
        ));
    }
    let path = sample_path(manifest_path);
    let sample = match read_json(&path) {
        Ok(sample) => sample,
        Err(error) => {
            issues.push(ContractLintIssue::new(
                "result.contract",
                format!(
                    "{} needs a sample output at {}: {error}",
                    contract.as_str(),
                    path.display()
                ),
            ));
            return issues;
        }
    };
    let id = manifest["id"].as_str().unwrap_or("");
    if sample["sample_of"].as_str() != Some(id) {
        issues.push(ContractLintIssue::new(
            "sample.sample_of",
            format!("{} must name the workflow id {id}", path.display()),
        ));
    }
    if !matches!(sample["source"].as_str(), Some("recorded" | "hand-built")) {
        issues.push(ContractLintIssue::new(
            "sample.source",
            format!("{} must say recorded or hand-built", path.display()),
        ));
    }
    if let Err(errors) = check_output(contract, &sample["output"]) {
        for error in errors {
            issues.push(ContractLintIssue::new(
                "sample.output",
                format!("{}: {error}", path.display()),
            ));
        }
    }
    issues
}

#[cfg(test)]
/// The family rule for one `workflows/<system>/` directory, given every manifest in it.
pub(crate) fn lint_family_describe(
    family_dir: &Path,
    manifests: &[&Value],
) -> Vec<ContractLintIssue> {
    let mut issues = Vec::new();
    let contract_of = |id: &str| {
        manifests
            .iter()
            .find(|manifest| manifest["id"].as_str() == Some(id))
            .map(|manifest| declared_contract(manifest))
    };
    if !manifests
        .iter()
        .any(|manifest| declared_contract(manifest).is_some_and(OutputContract::is_item))
    {
        return issues;
    }
    let path = family_dir.join(DESCRIBE_FILE);
    let describe = match read_json(&path) {
        Ok(describe) => describe,
        Err(error) => {
            issues.push(ContractLintIssue::new(
                "describe",
                format!(
                    "a family with an item contract needs {}: {error}",
                    path.display()
                ),
            ));
            return issues;
        }
    };
    if let Err(errors) = rzn_core::item_contract::check_describe(&describe) {
        for error in errors {
            issues.push(ContractLintIssue::new(
                "describe",
                format!("{}: {error}", path.display()),
            ));
        }
        return issues;
    }
    for kind in describe["kinds"].as_array().into_iter().flatten() {
        let list_tool = kind["list_tool"].as_str().unwrap_or("");
        if contract_of(list_tool) != Some(Some(OutputContract::ItemsV2)) {
            issues.push(ContractLintIssue::new(
                "describe.list_tool",
                format!(
                    "{}: list_tool {list_tool} must be a workflow of this family returning rzn.items/v2",
                    path.display()
                ),
            ));
        }
        let get_tool = kind["get_tool"].as_str().unwrap_or("");
        if contract_of(get_tool).is_none() {
            issues.push(ContractLintIssue::new(
                "describe.get_tool",
                format!(
                    "{}: get_tool {get_tool} is not a workflow of this family",
                    path.display()
                ),
            ));
        }
    }
    issues
}

#[cfg(test)]
/// Read workflows still on `none`.
pub(crate) fn read_workflows_on_none<'a>(manifests: impl IntoIterator<Item = &'a Value>) -> usize {
    manifests
        .into_iter()
        .filter(|manifest| {
            !writes_externally(manifest)
                && declared_contract(manifest) == Some(OutputContract::None)
        })
        .count()
}

#[cfg(test)]
/// What a read workflow uses to make its own requests: the request calls found in its steps'
/// strings and `same_origin_request` steps. Empty for a write workflow, which may post.
pub(crate) fn own_requests(manifest: &Value) -> Vec<String> {
    let mut found = Vec::new();
    if writes_externally(manifest) {
        return found;
    }
    for step in manifest["steps"].as_array().into_iter().flatten() {
        if step["action"]["kind"].as_str() == Some("same_origin_request") {
            found.push("same_origin_request".to_string());
        }
        let mut strings = Vec::new();
        collect_strings(&step["action"], &mut strings);
        for call in OWN_REQUEST_CALLS {
            if strings.iter().any(|text| calls(text, call)) {
                found.push(call.to_string());
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

#[cfg(test)]
fn collect_strings<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => out.push(text),
        Value::Array(items) => items.iter().for_each(|item| collect_strings(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect_strings(item, out)),
        _ => {}
    }
}

/// `call` appears in `text` as its own identifier (`prefetch(` is not `fetch(`).
#[cfg(test)]
fn calls(text: &str, call: &str) -> bool {
    text.match_indices(call).any(|(at, _)| {
        !text[..at]
            .chars()
            .next_back()
            .is_some_and(|before| before.is_alphanumeric() || before == '_' || before == '$')
    })
}

#[cfg(test)]
/// The own-request ratchet, same rule as [`ratchet_issue`].
pub(crate) fn own_request_ratchet_issue(offenders: &[String]) -> Option<ContractLintIssue> {
    use std::cmp::Ordering;
    let count = offenders.len();
    let base = READ_WORKFLOWS_MAKING_REQUESTS_BASELINE;
    let message = match count.cmp(&base) {
        Ordering::Equal => return None,
        Ordering::Greater => format!(
            "{count} read workflows make requests of their own, over the baseline {base}; read \
             what the page loads with capture_responses instead: {offenders:#?}"
        ),
        Ordering::Less => format!(
            "{count} read workflows make requests of their own; lower \
             READ_WORKFLOWS_MAKING_REQUESTS_BASELINE from {base} to {count}"
        ),
    };
    Some(ContractLintIssue::new("steps", message))
}

#[cfg(test)]
/// The ratchet: the count must equal the baseline, so a move to an item contract lowers it in
/// the same change and nothing raises it.
pub(crate) fn ratchet_issue(count: usize) -> Option<ContractLintIssue> {
    use std::cmp::Ordering;
    let message = match count.cmp(&READ_WORKFLOWS_ON_NONE_BASELINE) {
        Ordering::Equal => return None,
        Ordering::Greater => format!(
            "{count} read workflows are on result.contract none, over the baseline \
             {READ_WORKFLOWS_ON_NONE_BASELINE}; a new read workflow returns rzn.item/v2 or rzn.items/v2"
        ),
        Ordering::Less => format!(
            "{count} read workflows are on result.contract none; lower \
             READ_WORKFLOWS_ON_NONE_BASELINE from {READ_WORKFLOWS_ON_NONE_BASELINE} to {count}"
        ),
    };
    Some(ContractLintIssue::new("result.contract", message))
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&text).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn repo_workflows() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../workflows")
    }

    /// Every production manifest, grouped by family directory.
    fn production_manifests() -> BTreeMap<PathBuf, Vec<(PathBuf, Value)>> {
        let mut families = BTreeMap::new();
        let mut dirs = vec![repo_workflows()];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(&dir).expect("read workflows dir") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|name| name != "fixtures") {
                        dirs.push(path);
                    }
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "json") {
                    continue;
                }
                let value = read_json(&path).expect("workflow JSON parses");
                if value["schema_version"] == "rzn.workflow_manifest" {
                    let family = path.parent().expect("family dir").to_path_buf();
                    families
                        .entry(family)
                        .or_insert_with(Vec::new)
                        .push((path, value));
                }
            }
        }
        families
    }

    #[test]
    fn every_production_workflow_is_on_the_item_contract_lint() {
        let families = production_manifests();
        let mut issues = Vec::new();
        for (family, manifests) in &families {
            for (path, manifest) in manifests {
                for issue in lint_manifest(path, manifest) {
                    issues.push(format!(
                        "{}: {}: {}",
                        path.display(),
                        issue.field,
                        issue.message
                    ));
                }
            }
            let values = manifests.iter().map(|(_, value)| value).collect::<Vec<_>>();
            for issue in lint_family_describe(family, &values) {
                issues.push(format!("{}: {}", issue.field, issue.message));
            }
        }
        let count = read_workflows_on_none(families.values().flatten().map(|(_, value)| value));
        if let Some(issue) = ratchet_issue(count) {
            issues.push(issue.message);
        }
        assert!(issues.is_empty(), "item contract lint: {issues:#?}");
    }

    #[test]
    fn read_workflows_make_no_requests_of_their_own() {
        let offenders = production_manifests()
            .values()
            .flatten()
            .filter_map(|(path, manifest)| {
                let calls = own_requests(manifest);
                let rel = path.strip_prefix(repo_workflows()).unwrap_or(path);
                (!calls.is_empty()).then(|| format!("{}: {}", rel.display(), calls.join(", ")))
            })
            .collect::<Vec<_>>();
        if let Some(issue) = own_request_ratchet_issue(&offenders) {
            panic!("{}", issue.message);
        }
    }

    #[test]
    fn own_requests_finds_request_calls_in_read_workflows_only() {
        let read = json!({"side_effects": [{"class": "read_only"}], "steps": [
            {"action": {"kind": "execute_javascript", "inputs": {"script": "await window.fetch('/x')"}}},
            {"action": {"kind": "execute_javascript", "inputs": {"script": "prefetch(); new WebSocket(u)"}}},
            {"action": {"kind": "same_origin_request", "inputs": {"path": "/api"}}}]});
        assert_eq!(
            own_requests(&read),
            ["fetch(", "new WebSocket", "same_origin_request"]
        );
        let clean = json!({"steps": [{"action": {"kind": "execute_javascript",
            "inputs": {"script": "prefetch(); return params.captured_responses"}}}]});
        assert!(own_requests(&clean).is_empty());
        let mut write = read.clone();
        write["side_effects"] = json!(["external_write"]);
        assert!(own_requests(&write).is_empty());
        let over = vec!["x".to_string(); READ_WORKFLOWS_MAKING_REQUESTS_BASELINE + 1];
        assert!(own_request_ratchet_issue(&over).is_some());
    }

    #[test]
    fn missing_contract_sample_and_write_effects_are_reported() {
        let dir = std::env::temp_dir().join(format!("rzn_contract_lint_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join(SAMPLES_DIR)).expect("create samples dir");
        let path = dir.join("demo_list.json");
        let mut manifest = json!({"id": "demo/list", "result": {}});
        assert_eq!(lint_manifest(&path, &manifest)[0].field, "result.contract");

        manifest["result"]["contract"] = json!("none");
        assert!(lint_manifest(&path, &manifest).is_empty());

        manifest["result"]["contract"] = json!("rzn.items/v2");
        let issues = lint_manifest(&path, &manifest);
        assert!(
            issues[0].message.contains("needs a sample output"),
            "{issues:?}"
        );

        let sample = json!({"sample_of": "demo/list", "source": "hand-built",
            "output": {"type": "rzn.items/v2", "items": [], "listing": "everything"}});
        fs::write(sample_path(&path), sample.to_string()).expect("write sample");
        let issues = lint_manifest(&path, &manifest);
        assert!(issues.iter().all(|issue| issue.field == "sample.output") && !issues.is_empty());

        let sample = json!({"sample_of": "demo/list", "source": "hand-built",
            "output": {"type": "rzn.items/v2", "items": [], "listing": "feed"}});
        fs::write(sample_path(&path), sample.to_string()).expect("write sample");
        assert!(lint_manifest(&path, &manifest).is_empty());

        manifest["side_effects"] = json!([{"class": "external_write"}]);
        assert_eq!(lint_manifest(&path, &manifest).len(), 1);

        let values = [&manifest];
        let issues = lint_family_describe(&dir, &values);
        assert_eq!(issues[0].field, "describe");
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn the_ratchet_only_moves_down() {
        assert!(ratchet_issue(READ_WORKFLOWS_ON_NONE_BASELINE).is_none());
        assert!(ratchet_issue(READ_WORKFLOWS_ON_NONE_BASELINE + 1).is_some());
        assert!(ratchet_issue(READ_WORKFLOWS_ON_NONE_BASELINE - 1)
            .is_some_and(|issue| issue.message.contains("lower")));
        let read =
            json!({"result": {"contract": "none"}, "side_effects": [{"class": "read_only"}]});
        let write = json!({"result": {"contract": "none"}, "side_effects": ["external_write"]});
        assert_eq!(read_workflows_on_none([&read, &write]), 1);
    }
}
