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
        "version": RUN_ENVELOPE_VERSION,
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

fn manifest() -> WorkflowManifestV2 {
    let mut properties = BTreeMap::new();
    properties.insert(
        "query".to_string(),
        ParamDefV2 {
            kind: ParamKindV2::String,
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
        ParamDefV2 {
            kind: ParamKindV2::Integer,
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

    WorkflowManifestV2 {
        schema_version: WORKFLOW_CONTRACT_VERSION.to_string(),
        id: "google.search".to_string(),
        name: "Google Search".to_string(),
        version: "2.0.0".to_string(),
        system: "google".to_string(),
        capability: "search".to_string(),
        summary: None,
        description: None,
        params: ParamSchemaV2 {
            properties,
            additional_params: false,
        },
        side_effects: vec![SideEffectDeclarationV2 {
            class: SideEffectClassV2::BrowserState,
            idempotency: IdempotencyPolicyV1::SafeRetry,
            confirmation_required: false,
            scopes: Vec::new(),
        }],
        runtime: RuntimeRequirementsV1::default(),
        steps: vec![StepV2 {
            id: "open".to_string(),
            name: None,
            action: ActionV2 {
                kind: ActionKindV2::Navigate,
                custom_kind: None,
                target: None,
                inputs: Map::new(),
                options: Map::new(),
                side_effects: vec![SideEffectClassV2::BrowserState],
            },
            timeout_ms: None,
            retry: RetryPolicyV1::default(),
            continue_on_error: false,
        }],
        result: ResultContractV2::default(),
        help: None,
        metadata: None,
    }
}
