use rzn_contracts::v2::ActionKindV2;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn production_manifest_actions_are_first_class_contract_kinds() {
    let workflow_root = repo_root().join("workflows");
    let mut observed = BTreeSet::new();
    let mut custom_actions = Vec::new();
    let mut unknown_actions = Vec::new();

    for path in workflow_files(&workflow_root) {
        let rel = path.strip_prefix(&workflow_root).unwrap_or(&path);
        if rel.components().any(|component| {
            let text = component.as_os_str().to_string_lossy();
            text == "fixtures" || text.starts_with("test")
        }) {
            continue;
        }

        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if value.get("schema_version").and_then(Value::as_str) != Some("rzn.workflow_manifest") {
            continue;
        }

        let Some(steps) = value.get("steps").and_then(Value::as_array) else {
            continue;
        };
        for step in steps {
            let step_id = step
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("<missing>");
            let Some(kind) = step
                .get("action")
                .and_then(|action| action.get("kind"))
                .and_then(Value::as_str)
            else {
                unknown_actions.push(format!("{}:{step_id}:<missing>", path.display()));
                continue;
            };

            observed.insert(kind.to_string());
            if kind == "custom" {
                let custom_kind = step
                    .get("action")
                    .and_then(|action| action.get("custom_kind"))
                    .and_then(Value::as_str)
                    .unwrap_or("<missing>");
                custom_actions.push(format!("{}:{step_id}:{custom_kind}", path.display()));
                continue;
            }

            let decoded = serde_json::from_value::<ActionKindV2>(Value::String(kind.to_string()));
            match decoded {
                Ok(decoded) if decoded.engine_step_type() == Some(kind) => {}
                Ok(decoded) => unknown_actions.push(format!(
                    "{}:{step_id}:{kind}:mapped_to:{:?}",
                    path.display(),
                    decoded.engine_step_type()
                )),
                Err(error) => {
                    unknown_actions.push(format!("{}:{step_id}:{kind}:{error}", path.display()))
                }
            }
        }
    }

    assert!(
        !observed.is_empty(),
        "no production manifest actions observed"
    );
    assert!(
        custom_actions.is_empty(),
        "production manifests should not use custom for built-in actions:\n{}",
        custom_actions.join("\n")
    );
    assert!(
        unknown_actions.is_empty(),
        "production manifest action kinds missing contract coverage:\n{}\nobserved: {:?}",
        unknown_actions.join("\n"),
        observed
    );
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .to_path_buf()
}

fn workflow_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_json_files(root, &mut files);
    files.sort();
    files
}

fn collect_json_files(path: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some("json") {
            files.push(path);
        }
    }
}
