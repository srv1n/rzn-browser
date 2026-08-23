use rzn_contracts::browser::*;
use serde_json::{json, Value};

#[test]
fn cloud_command_redelivery_keeps_command_id_as_bridge_req_id() {
    let first = cloud_command("lease_a", Some("conflicting-session"));
    let redelivery = cloud_command("lease_b", None);

    let first_bridge = local_bridge_command(&first);
    let redelivery_bridge = local_bridge_command(&redelivery);

    assert_eq!(first.command_id, redelivery.command_id);
    assert_ne!(first.lease_id, redelivery.lease_id);
    assert_eq!(first_bridge["req_id"], json!("cmd_abc"));
    assert_eq!(redelivery_bridge["req_id"], json!("cmd_abc"));
    assert_eq!(first_bridge["payload"]["session_id"], json!("sess_789"));
    assert_eq!(
        redelivery_bridge["payload"]["session_id"],
        json!("sess_789")
    );
    assert_eq!(
        first_bridge["payload"]["step"]["type"],
        json!("navigate_to_url")
    );
}

fn cloud_command(lease_id: &str, payload_session_id: Option<&str>) -> CloudCommandEnvelope {
    let mut payload = json!({
        "step": {
            "id": "step-1",
            "type": "navigate_to_url",
            "url": "https://example.com"
        }
    });
    if let Some(session_id) = payload_session_id {
        payload["session_id"] = json!(session_id);
    }

    CloudCommandEnvelope {
        version: CLOUD_CONTRACT.to_string(),
        message_type: "command.execute".to_string(),
        actor_id: "act_123".to_string(),
        run_id: "run_456".to_string(),
        session_id: "sess_789".to_string(),
        command_id: "cmd_abc".to_string(),
        lease_id: lease_id.to_string(),
        deadline_ms: 1_710_000_000_000,
        trace_id: Some("trace_1".to_string()),
        parent_command_id: None,
        planner_step_index: Some(0),
        payload: CloudCommandPayload {
            kind: CloudCommandKind::BrowserCommand,
            command: Some(CloudBrowserCommand {
                cmd: "execute_step".to_string(),
                payload: Some(payload),
                data: None,
            }),
            side_effecting: Some(true),
            idempotency_policy: Some("single_delivery".to_string()),
            metadata: None,
        },
    }
}

fn local_bridge_command(command: &CloudCommandEnvelope) -> Value {
    let browser_command = command
        .payload
        .command
        .as_ref()
        .expect("browser command payload");
    let mut payload = browser_command.payload.clone().unwrap_or_else(|| json!({}));

    // The cloud envelope is authoritative. A stale nested payload session_id
    // must not fork tab/session affinity during migration.
    payload["session_id"] = json!(command.session_id);

    json!({
        "cmd": browser_command.cmd,
        "req_id": command.command_id,
        "payload": payload
    })
}
