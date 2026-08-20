use serde_json::{json, Value};

pub(crate) fn native_host_cloud_bridge_status() -> Value {
    json!({
        "runtime_owner": "supervisor",
        "supervisor_bridge_status": "native_host_compatibility_adapter",
        "native_host_dispatch": "disabled",
        "result_replay": "supervisor caches cloud command results by command_id for idempotent replay",
        "status": "native host is transport-only; cloud.status is owned by rzn.local.v1"
    })
}
