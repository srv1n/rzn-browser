use rzn_contracts::browser::*;
use std::collections::HashMap;

#[test]
fn snapshot_roundtrips() {
    let mut attrs = HashMap::new();
    attrs.insert("id".to_string(), "q".to_string());

    let snap = BrowserSnapshot {
        version: BROWSER_CONTRACT.to_string(),
        dom_hash: "abc123".to_string(),
        metadata: SnapshotMetadata {
            timestamp: 123456,
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            viewport: Viewport {
                width: 1200,
                height: 800,
            },
        },
        elements: vec![BrowserElement {
            encoded_id: "elem_0".to_string(),
            tag: "input".to_string(),
            text: None,
            attributes: attrs,
            selector: "#q".to_string(),
            spatial_info: Some(SpatialInfo {
                x: 10,
                y: 20,
                width: 100,
                height: 30,
                area: 3000,
                viewport_position: "top".to_string(),
            }),
        }],
        prompt: Some("PROMPT".to_string()),
        capabilities: None,
    };

    let json = serde_json::to_string(&snap).expect("serialize");
    let back: BrowserSnapshot = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(snap, back);
}

#[test]
fn action_roundtrips() {
    let a = BrowserAction::FillInputField {
        target: BrowserTarget::from_encoded_id("elem_0"),
        value: "hello".to_string(),
        clear_first: Some(true),
        simulate_typing: Some(true),
        delay_ms: Some(25),
        timeout_ms: Some(5000),
    };

    let json = serde_json::to_string(&a).expect("serialize");
    let back: BrowserAction = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(a, back);
}

#[test]
fn cloud_command_envelope_roundtrips() {
    let envelope = CloudCommandEnvelope {
        version: CLOUD_CONTRACT.to_string(),
        message_type: "command.execute".to_string(),
        actor_id: "act_123".to_string(),
        run_id: "run_456".to_string(),
        session_id: "sess_789".to_string(),
        command_id: "cmd_abc".to_string(),
        lease_id: "lease_xyz".to_string(),
        deadline_ms: 1710000000000,
        trace_id: Some("trace_1".to_string()),
        parent_command_id: None,
        planner_step_index: Some(3),
        payload: CloudCommandPayload {
            kind: CloudCommandKind::BrowserCommand,
            command: Some(CloudBrowserCommand {
                cmd: "execute_step".to_string(),
                payload: Some(serde_json::json!({
                    "step": {
                        "id": "step-1",
                        "name": "Click Sign In",
                        "type": "click_element",
                        "selector": "@e12"
                    }
                })),
                data: Some(serde_json::json!({
                    "session_id": "sess_789"
                })),
            }),
            side_effecting: Some(true),
            idempotency_policy: Some("single_delivery".to_string()),
            metadata: None,
        },
    };

    let json = serde_json::to_string(&envelope).expect("serialize");
    let back: CloudCommandEnvelope = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(envelope, back);
}

#[test]
fn cloud_command_result_roundtrips() {
    let result = CloudCommandResult {
        version: CLOUD_CONTRACT.to_string(),
        message_type: "command.result".to_string(),
        actor_id: "act_123".to_string(),
        run_id: "run_456".to_string(),
        session_id: "sess_789".to_string(),
        command_id: "cmd_abc".to_string(),
        lease_id: "lease_xyz".to_string(),
        success: true,
        finished_at_ms: 1710000000123,
        trace_id: Some("trace_1".to_string()),
        result: Some(BrowserActionResult {
            success: true,
            error_code: None,
            error: None,
            current_url: Some("https://example.com".to_string()),
            current_tab_id: Some(42),
            current_tab_ref: Some("rzn://browser/browser-instance/tab/42".to_string()),
            dom_hash: Some("hash_123".to_string()),
            dom_snapshot: None,
            capabilities: Some(Capabilities {
                extension_actor: true,
                cdp_available: true,
                cdp_enabled: false,
                cdp_attached: false,
            }),
            raw: Some(serde_json::json!({
                "success": true
            })),
        }),
        error: None,
    };

    let json = serde_json::to_string(&result).expect("serialize");
    let back: CloudCommandResult = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(result, back);
}
