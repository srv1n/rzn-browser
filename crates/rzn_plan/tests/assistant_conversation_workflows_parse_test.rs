use std::path::PathBuf;

use rzn_core::dsl::Workflow;
use serde_json::Value;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

#[test]
fn assistant_conversation_workflows_parse_as_v1_workflow() {
    let workflows = [
        "workflows/chatgpt/chatgpt_recent_chats.json",
        "workflows/chatgpt/chatgpt_read.json",
        "workflows/claude/claude_recent_chats.json",
        "workflows/claude/claude_export_chat.json",
        "workflows/claude/claude_send.json",
    ];

    for rel in workflows {
        let path = repo_root().join(rel);
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
        let value: Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse {} as JSON: {}", path.display(), e));

        if value.get("schema_version").and_then(Value::as_str) == Some("rzn.workflow_manifest") {
            let steps = value
                .get("steps")
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("{} manifest has no steps array", rel));
            assert!(!steps.is_empty(), "{} manifest has no steps", rel);
            continue;
        }

        let wf: Workflow = serde_json::from_value(value)
            .unwrap_or_else(|e| panic!("parse {} as Workflow: {}", path.display(), e));

        assert!(
            !wf.browser_automation.sequences.is_empty(),
            "{} has no sequences",
            rel
        );
        for seq in &wf.browser_automation.sequences {
            assert!(
                !seq.steps.is_empty(),
                "{} sequence '{}' has no steps",
                rel,
                seq.name
            );
        }
    }
}
