use rzn_contracts::v1::*;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;

fn restart_matrix_fixture() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/features/local_supervisor_runtime/restart_matrix.v1.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read restart matrix {}: {}", path.display(), err));
    serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("parse restart matrix {}: {}", path.display(), err))
}

#[test]
fn restart_matrix_fixture_covers_lrt_producers_and_churn() {
    let fixture = restart_matrix_fixture();
    assert_eq!(
        fixture.get("version").and_then(Value::as_str),
        Some("rzn.local.restart_matrix.v1")
    );

    let scenarios = fixture
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios array");
    assert!(!scenarios.is_empty(), "restart matrix must not be empty");

    let mut ids = HashSet::new();
    let mut producers = HashSet::new();
    let mut churn = HashSet::new();
    let mut ci_by_producer: HashMap<String, usize> = HashMap::new();

    for scenario in scenarios {
        let id = scenario
            .get("id")
            .and_then(Value::as_str)
            .expect("scenario id");
        assert!(ids.insert(id.to_string()), "duplicate scenario id: {id}");

        let producer = scenario
            .get("producer")
            .and_then(Value::as_str)
            .expect("scenario producer");
        producers.insert(producer.to_string());

        let churn_kind = scenario
            .get("churn")
            .and_then(Value::as_str)
            .expect("scenario churn");
        churn.insert(churn_kind.to_string());

        let assertions = scenario
            .get("assertions")
            .and_then(Value::as_array)
            .expect("scenario assertions");
        assert!(
            assertions.len() >= 2,
            "scenario {id} should name concrete assertions"
        );

        if scenario
            .get("ci_safe")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            *ci_by_producer.entry(producer.to_string()).or_default() += 1;
        }
    }

    for required in ["cli", "mcp", "cloud", "reason_app", "extension"] {
        assert!(
            producers.contains(required),
            "missing producer coverage for {required}"
        );
    }

    for required in [
        "supervisor_restart",
        "native_host_restart",
        "extension_service_worker_restart",
        "chrome_restart",
        "historical_endpoint_file_present",
        "app_restart",
    ] {
        assert!(
            churn.contains(required),
            "missing churn coverage for {required}"
        );
    }

    for required in ["cli", "mcp", "cloud", "reason_app"] {
        assert!(
            ci_by_producer.get(required).copied().unwrap_or(0) > 0,
            "missing CI-safe scaffold for {required}"
        );
    }
}

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

fn cloud_command(lease_id: &str, payload_session_id: Option<&str>) -> CloudCommandEnvelopeV1 {
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

    CloudCommandEnvelopeV1 {
        version: CLOUD_CONTRACT_VERSION.to_string(),
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
        payload: CloudCommandPayloadV1 {
            kind: CloudCommandKindV1::BrowserCommand,
            command: Some(CloudBrowserCommandV1 {
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

fn local_bridge_command(command: &CloudCommandEnvelopeV1) -> Value {
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
