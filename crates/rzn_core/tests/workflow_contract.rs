use rzn_core::workflow_contract::*;
use serde_json::{json, Map};
use std::collections::BTreeMap;

#[test]
fn core_validates_manifest_strings_and_normalizes_params() {
    let manifest_json = serde_json::to_string(&manifest()).expect("manifest json");
    let manifest = validate_manifest_str(&manifest_json).expect("manifest validates");

    let params = normalize_manifest_params(
        &manifest,
        &json!({
            "query": "rzn",
            "limit": "7"
        }),
    )
    .expect("params normalize");

    assert_eq!(params.get("query"), Some(&json!("rzn")));
    assert_eq!(params.get("limit"), Some(&json!(7)));
}

#[test]
fn core_validates_run_envelope_strings() {
    let manifest = manifest();
    let envelope = json!({
        "version": RUN_REQUEST_CONTRACT,
        "run_id": "run_1",
        "workflow_id": manifest.id,
        "workflow_version": manifest.version,
        "system": manifest.system,
        "capability": manifest.capability,
        "params": {
            "query": "rzn"
        },
        "policy": {
            "allow_side_effects": ["browser_state"],
            "dry_run": false
        }
    });

    let envelope_json = serde_json::to_string(&envelope).expect("envelope json");
    validate_run_envelope_str(&manifest, &envelope_json).expect("envelope validates");
}

fn manifest() -> WorkflowManifest {
    let mut properties = BTreeMap::new();
    properties.insert(
        "query".to_string(),
        ParamDef {
            kind: ParamKind::String,
            required: true,
            sensitive: false,
            description: None,
            default: None,
            enum_values: Vec::new(),
            min: None,
            max: None,
            min_length: Some(1),
            max_length: None,
        },
    );
    properties.insert(
        "limit".to_string(),
        ParamDef {
            kind: ParamKind::Integer,
            required: false,
            sensitive: false,
            description: None,
            default: Some(json!(10)),
            enum_values: Vec::new(),
            min: Some(1),
            max: Some(20),
            min_length: None,
            max_length: None,
        },
    );

    WorkflowManifest {
        schema_version: WORKFLOW_CONTRACT.to_string(),
        id: "google.search".to_string(),
        name: "Google Search".to_string(),
        version: "2.0.0".to_string(),
        system: "google".to_string(),
        capability: "search".to_string(),
        summary: None,
        description: None,
        params: ParamSchema {
            properties,
            additional_params: false,
        },
        side_effects: vec![SideEffectDeclaration {
            class: SideEffectClass::BrowserState,
            idempotency: IdempotencyPolicy::SafeRetry,
            confirmation_required: false,
            scopes: Vec::new(),
        }],
        runtime: RuntimeRequirements::default(),
        steps: vec![WorkflowStep {
            id: "open".to_string(),
            name: None,
            action: WorkflowAction {
                kind: WorkflowActionKind::Navigate,
                custom_kind: None,
                target: None,
                inputs: Map::new(),
                options: Map::new(),
                side_effects: vec![SideEffectClass::BrowserState],
            },
            timeout_ms: None,
            retry: RetryPolicy::default(),
            continue_on_error: false,
        }],
        result: ResultContract::default(),
        help: None,
        metadata: None,
    }
}
