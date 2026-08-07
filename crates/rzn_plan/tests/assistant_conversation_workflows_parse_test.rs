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
        "workflows/chatgpt/chatgpt_send.json",
        "workflows/chatgpt/chatgpt_artifact_url.json",
        "workflows/chatgpt/chatgpt_fetch_estuary_base64.json",
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

            if rel == "workflows/chatgpt/chatgpt_read.json" {
                assert_eq!(
                    value.pointer("/params/properties/chat_url/required"),
                    Some(&Value::Bool(true)),
                    "chatgpt/read must require the stored direct chat URL"
                );
                assert_eq!(
                    steps[0].pointer("/action/inputs/url"),
                    Some(&Value::String("{chat_url}".to_string())),
                    "chatgpt/read must open the destination URL directly"
                );
                assert_eq!(
                    steps[0].pointer("/action/inputs/skip_if_url_contains"),
                    Some(&Value::String("/c/{chat_id}".to_string())),
                    "chatgpt/read must retain its exact-conversation reuse guard"
                );
            }
            if rel == "workflows/chatgpt/chatgpt_send.json" {
                assert_eq!(
                    value.pointer("/params/properties/entry_url/required"),
                    Some(&Value::Bool(true)),
                    "chatgpt/send must require its direct entry URL"
                );
                assert_eq!(
                    steps[0].pointer("/action/inputs/url"),
                    Some(&Value::String("{entry_url}".to_string())),
                    "chatgpt/send must not stage ChatGPT root before continuation"
                );
                let workflow_text = serde_json::to_string(&value).expect("serialize send workflow");
                for intelligence in ["Medium", "High", "Extra High", "Pro"] {
                    assert!(
                        workflow_text.contains(intelligence),
                        "chatgpt/send must accept and verify the live intelligence tier {intelligence}"
                    );
                }
                assert!(
                    value
                        .pointer("/params/properties/require_exact_model")
                        .is_some(),
                    "chatgpt/send must keep exposing require_exact_model"
                );
                assert!(
                    workflow_text.contains("model_selection_verify_failed"),
                    "chatgpt/send must retain its exact-selection failure contract"
                );
            }
            if matches!(
                rel,
                "workflows/chatgpt/chatgpt_artifact_url.json"
                    | "workflows/chatgpt/chatgpt_fetch_estuary_base64.json"
            ) {
                assert_eq!(
                    value.pointer("/params/properties/chat_url/required"),
                    Some(&Value::Bool(true)),
                    "{} must require the saved direct chat URL",
                    rel
                );
                assert_eq!(
                    steps[0].pointer("/action/inputs/url"),
                    Some(&Value::String("{chat_url}".to_string())),
                    "{} must not stage ChatGPT root before its work",
                    rel
                );
            }
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
