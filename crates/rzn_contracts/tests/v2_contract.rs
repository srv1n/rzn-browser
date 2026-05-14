use rzn_contracts::v2::*;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

#[test]
fn manifest_v2_roundtrips_and_validates_side_effects() {
    let manifest = fixture_manifest();

    let raw = serde_json::to_string(&manifest).expect("serialize manifest");
    let back: WorkflowManifestV2 = serde_json::from_str(&raw).expect("deserialize manifest");

    assert_eq!(manifest, back);
    back.validate().expect("manifest validates");
}

#[test]
fn manifest_v2_rejects_unknown_fields() {
    let mut value = serde_json::to_value(fixture_manifest()).expect("manifest json");
    value["legacy_field"] = json!(true);

    let err = validate_manifest_value(&value).expect_err("unknown fields fail");
    assert!(
        err[0].message.contains("unknown field"),
        "unexpected error: {err:?}"
    );
}

#[test]
fn manifest_v2_rejects_undeclared_step_side_effects() {
    let mut manifest = fixture_manifest();
    manifest.steps[0]
        .action
        .side_effects
        .push(SideEffectClassV2::ExternalWrite);

    let errors = manifest
        .validate()
        .expect_err("undeclared side effect fails");
    assert!(errors.iter().any(|error| {
        error.field == "steps[0].action.side_effects" && error.message.contains("external_write")
    }));
}

#[test]
fn manifest_v2_rejects_zero_retry_attempts() {
    let mut manifest = fixture_manifest();
    manifest.steps[0].retry.max_attempts = 0;

    let errors = manifest.validate().expect_err("zero total attempts fails");

    assert!(errors.iter().any(|error| {
        error.field == "steps[0].retry.max_attempts"
            && error
                .message
                .contains("total attempts including the initial try")
    }));
}

#[test]
fn retry_policy_max_attempts_is_total_attempts() {
    let retry = RetryPolicyV1 {
        max_attempts: 3,
        backoff_ms: Some(250),
    };

    assert_eq!(retry.retry_budget(), 2);
}

#[test]
fn side_effect_classes_include_external_read_and_network_access() {
    assert_eq!(
        serde_json::to_value(SideEffectClassV2::ExternalRead).expect("serialize"),
        json!("external_read")
    );
    assert_eq!(
        serde_json::to_value(SideEffectClassV2::NetworkAccess).expect("serialize"),
        json!("network_access")
    );
}

#[test]
fn registered_engine_action_kinds_roundtrip() {
    let action_kinds = [
        "apply_filter_by_text",
        "assert_selector_state",
        "assert_text_in_element",
        "assert_url_matches",
        "capture_ui_bundle",
        "clear_cookies",
        "clear_enhanced_caches",
        "clear_local_storage",
        "click",
        "click_element",
        "close_current_tab",
        "configure_captcha_solver",
        "date_set_range",
        "dbl_click_element",
        "detect_popups",
        "dismiss_popups",
        "download",
        "download_file",
        "download_images",
        "drag_and_drop",
        "eval_isolated_world",
        "eval_main_world",
        "execute_extraction_plan",
        "execute_javascript",
        "extract",
        "extract_page_assets",
        "extract_structured_data",
        "fill_and_submit",
        "fill_input_field",
        "get_cookies",
        "get_current_url",
        "get_element_attribute",
        "get_element_count",
        "get_element_text",
        "get_element_value",
        "get_local_storage_item",
        "get_page_source",
        "get_performance_stats",
        "handle_captcha",
        "hover_element",
        "infinite_scroll",
        "inspect_click_surface",
        "inspect_element",
        "navigate",
        "navigate_to_url",
        "observe",
        "open_new_tab",
        "press_key",
        "press_special_key",
        "read_field_value",
        "request_user_intervention",
        "same_origin_request",
        "scroll",
        "scroll_element_into_view",
        "scroll_window_to",
        "select_option",
        "select_option_in_dropdown",
        "select_result",
        "semantic_action",
        "set_cookie",
        "set_local_storage_item",
        "simulate_human_behavior",
        "submit_input",
        "submit_text_query",
        "switch_to_tab",
        "take_screenshot",
        "type_text",
        "upload_file",
        "verify_ui_change",
        "wait",
        "wait_for_auth",
        "wait_for_element",
        "wait_for_navigation",
        "wait_for_network_idle",
        "wait_for_no_popups",
        "wait_for_timeout",
        "wait_for_totp",
        "wait_for_verification",
    ];

    for kind in action_kinds {
        let decoded: ActionKindV2 =
            serde_json::from_value(json!(kind)).unwrap_or_else(|error| panic!("{kind}: {error}"));
        assert_eq!(decoded.engine_step_type(), Some(kind), "{kind}");
    }
}

#[test]
fn params_v2_apply_defaults_coerce_and_reject_unknowns() {
    let manifest = fixture_manifest();
    let params = manifest
        .normalize_params(&json!({
            "query": 42,
            "limit": "5",
            "dry_run": "yes"
        }))
        .expect("params normalize");

    assert_eq!(params.get("query"), Some(&json!("42")));
    assert_eq!(params.get("limit"), Some(&json!(5)));
    assert_eq!(params.get("dry_run"), Some(&json!(true)));
    assert_eq!(params.get("locale"), Some(&json!("en-US")));

    let errors = manifest
        .normalize_params(&json!({
            "query": "rust",
            "unknown": true
        }))
        .expect_err("unknown param fails");
    assert_eq!(errors[0].field, "unknown");
}

#[test]
fn run_envelope_v1_validates_against_manifest_identity_and_params() {
    let manifest = fixture_manifest();
    let mut params = Map::new();
    params.insert("query".to_string(), json!("rust"));
    params.insert("limit".to_string(), json!(3));

    let envelope = RunEnvelopeV1 {
        version: RUN_ENVELOPE_VERSION.to_string(),
        run_id: "run_123".to_string(),
        workflow_id: manifest.id.clone(),
        workflow_version: manifest.version.clone(),
        system: manifest.system.clone(),
        capability: manifest.capability.clone(),
        actor_id: Some("actor_1".to_string()),
        trace_id: Some("trace_1".to_string()),
        deadline_ms: Some(1_710_000_000_000),
        params,
        policy: RunPolicyV1 {
            allow_side_effects: vec![SideEffectClassV2::BrowserState],
            dry_run: false,
        },
    };

    envelope
        .validate_for_manifest(&manifest)
        .expect("envelope validates");

    let mut wrong = envelope.clone();
    wrong.system = "other".to_string();
    let errors = wrong
        .validate_for_manifest(&manifest)
        .expect_err("system mismatch fails");
    assert!(errors.iter().any(|error| error.field == "system"));
}

fn fixture_manifest() -> WorkflowManifestV2 {
    let mut properties = BTreeMap::new();
    properties.insert(
        "query".to_string(),
        ParamDefV2 {
            kind: ParamKindV2::String,
            required: true,
            sensitive: false,
            description: Some("Search query".to_string()),
            default: None,
            enum_values: Vec::new(),
            min: None,
            max: None,
            min_length: Some(1),
            max_length: Some(200),
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
    properties.insert(
        "dry_run".to_string(),
        ParamDefV2 {
            kind: ParamKindV2::Boolean,
            required: false,
            sensitive: false,
            description: None,
            default: Some(json!(false)),
            enum_values: Vec::new(),
            min: None,
            max: None,
            min_length: None,
            max_length: None,
        },
    );
    properties.insert(
        "locale".to_string(),
        ParamDefV2 {
            kind: ParamKindV2::String,
            required: false,
            sensitive: false,
            description: None,
            default: Some(json!("en-US")),
            enum_values: vec![json!("en-US"), json!("fr-FR")],
            min: None,
            max: None,
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
        summary: Some("Search Google and return structured results.".to_string()),
        description: None,
        params: ParamSchemaV2 {
            properties,
            additional_params: false,
        },
        side_effects: vec![SideEffectDeclarationV2 {
            class: SideEffectClassV2::BrowserState,
            idempotency: IdempotencyPolicyV1::SafeRetry,
            confirmation_required: false,
            scopes: vec!["tab".to_string()],
        }],
        runtime: RuntimeRequirementsV1::default(),
        steps: vec![StepV2 {
            id: "open_search".to_string(),
            name: Some("Open search URL".to_string()),
            action: ActionV2 {
                kind: ActionKindV2::Navigate,
                custom_kind: None,
                target: None,
                inputs: Map::from_iter([(
                    "url".to_string(),
                    Value::String("https://www.google.com/search?q={query}".to_string()),
                )]),
                options: Map::new(),
                side_effects: vec![SideEffectClassV2::BrowserState],
            },
            timeout_ms: Some(10_000),
            retry: RetryPolicyV1::default(),
            continue_on_error: false,
        }],
        result: ResultContractV2 {
            output_schema: Some(json!({"type": "array"})),
            output_selector: Some(OutputSelectorV1 {
                step_id: "open_search".to_string(),
                path: Some("$.results".to_string()),
            }),
            artifact_policy: ArtifactPolicyV1 {
                max_inline_bytes: Some(16_384),
                prefer_downloads: false,
            },
            include_debug: false,
        },
        help: Some(HelpBlockV2 {
            summary: "Run a Google search.".to_string(),
            parameters: BTreeMap::from([("query".to_string(), "Search query.".to_string())]),
            examples: vec![json!({"query": "rust"})],
            returns: vec!["Structured search results.".to_string()],
            notes: Vec::new(),
        }),
        metadata: None,
    }
}
