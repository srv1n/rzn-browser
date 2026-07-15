//! Unit tests for the shared workflow run loop.
//!
//! The first block relocates the loader/param/response/output tests that used to
//! live in `native_runner` (they exercise code that moved here). The second block
//! is the `MockTransport` suite that drives `execute_workflow` end to end:
//! happy path, transient retry, external-write no-retry, stop_workflow early exit,
//! per-step watchdog timeout, output selector, and legacy-format workflows.

use super::*;
use crate::workflow_params::apply_parameters;
use rzn_contracts::v2::RunStatusV2;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Relocated unit tests for moved helpers.
// ---------------------------------------------------------------------------

#[test]
fn local_wait_short_circuits_only_sleep_steps() {
    assert!(should_handle_step_locally("wait_for_timeout"));
    assert!(!should_handle_step_locally("wait_for_element"));
    assert!(!should_handle_step_locally("submit_input"));
}

#[test]
fn transient_step_errors_match_extension_bridge_failures() {
    assert!(is_transient_step_error(
        "Could not establish connection. Receiving end does not exist."
    ));
    assert!(is_transient_step_error("Native host timeout after 20000ms"));
    assert!(is_transient_step_error(
        "Extension timeout while waiting for step result"
    ));
    assert!(is_transient_step_error("native_host_disconnected"));
    assert!(is_transient_step_error(
        "Native-host bridge response channel closed"
    ));
    assert!(is_transient_step_error(
        "Native-host extension bridge timeout after 40000ms"
    ));
    assert!(!is_transient_step_error("Selector not found"));
}

#[test]
fn external_write_steps_are_detected_for_no_retry() {
    // Manifest steps carry side_effects at the top level.
    assert!(step_has_external_write(&json!({
        "id": "s9",
        "type": "execute_javascript",
        "side_effects": ["browser_state", "external_write"],
    })));
    // Legacy steps may nest them under `action`.
    assert!(step_has_external_write(&json!({
        "id": "s9",
        "action": { "side_effects": ["external_write"] },
    })));
    // Read-only / state-only steps must remain retriable.
    assert!(!step_has_external_write(&json!({
        "id": "s7",
        "type": "wait_for_element",
        "side_effects": ["read_only"],
    })));
    assert!(!step_has_external_write(&json!({ "id": "s1" })));
}

#[test]
fn apply_parameters_injects_safe_params_for_script_steps() {
    let workflow = json!({
        "browser_automation": {
            "sequences": [{
                "steps": [{
                    "type": "execute_javascript",
                    "script": "return window.__rzn_params.message_body;",
                    "args": []
                }]
            }]
        }
    });
    let params = HashMap::from([("message_body".to_string(), "O'Reilly".to_string())]);

    let applied = apply_parameters(workflow, &params);
    let step = &applied["browser_automation"]["sequences"][0]["steps"][0];

    assert_eq!(step["params"]["message_body"], "O'Reilly");
    assert_eq!(step["script"], "return window.__rzn_params.message_body;");
}

#[test]
fn apply_parameters_expands_chained_param_defaults() {
    let workflow = json!({
        "browser_automation": {
            "sequences": [{
                "steps": [{
                    "type": "navigate_to_url",
                    "url": "{app_url}"
                }]
            }]
        }
    });
    let params = HashMap::from([
        (
            "app_url".to_string(),
            "https://apps.apple.com/{country}/app/id{app_id}".to_string(),
        ),
        ("country".to_string(), "us".to_string()),
        ("app_id".to_string(), "123456789".to_string()),
    ]);

    let applied = apply_parameters(workflow, &params);

    assert_eq!(
        applied
            .pointer("/browser_automation/sequences/0/steps/0/url")
            .and_then(|value| value.as_str()),
        Some("https://apps.apple.com/us/app/id123456789")
    );
}

#[test]
fn runtime_context_is_discovered_from_manifest_workflow_ref() {
    let root = unique_temp_path("runtime-context").join("workflows");
    let workflow_dir = root.join("x");
    fs::create_dir_all(&workflow_dir).unwrap();
    let workflow_path = workflow_dir.join("x_open.json");
    fs::write(
        &workflow_path,
        r#"{
          "browser_automation": {
            "sequences": [{
              "steps": [{ "id": "extract", "type": "extract_structured_data" }]
            }]
          }
        }"#,
    )
    .unwrap();
    fs::write(
        workflow_dir.join("open.json"),
        r#"{
          "schema_version": "rzn.workflow_manifest",
          "id": "x.open",
          "name": "Open X",
          "version": "0.1.0",
          "system": "x",
          "capability": "x.read.unified",
          "side_effects": [{ "class": "read_only" }],
          "runtime": { "actor": "supervisor", "workflow_path": "x/x_open.json" },
          "steps": [],
          "result": {
            "output_selector": { "step_id": "extract", "path": "$" }
          }
        }"#,
    )
    .unwrap();

    let context = load_runtime_context_for_workflow(&workflow_path.to_string_lossy()).unwrap();

    let context = context.expect("manifest context");
    assert_eq!(context.workflow_id, "x.open");
    assert_eq!(context.workflow_version, "0.1.0");
    assert_eq!(context.capability, "x.read.unified");
    assert_eq!(context.declared_side_effects, vec!["read_only"]);
    assert!(context.enforce_side_effects);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn step_payload_threads_manifest_identity_and_side_effect_policy() {
    let context = WorkflowRuntimeContext {
        workflow_id: "x.open".to_string(),
        workflow_version: "0.1.0".to_string(),
        system: "x".to_string(),
        capability: "x.read.unified".to_string(),
        declared_side_effects: vec!["read_only".to_string()],
        enforce_side_effects: true,
        output_selector_step_id: None,
        output_selector_path: None,
    };

    let payload = step_execution_payload(
        Some("session-1"),
        &json!({ "id": "extract", "type": "extract_structured_data" }),
        true,
        Some(&context),
    );

    assert_eq!(payload.get("workflow_id"), Some(&json!("x.open")));
    assert_eq!(payload.get("workflow_version"), Some(&json!("0.1.0")));
    assert_eq!(payload.get("system"), Some(&json!("x")));
    assert_eq!(payload.get("capability"), Some(&json!("x.read.unified")));
    assert_eq!(
        payload.pointer("/side_effect_policy/enforce"),
        Some(&json!(true))
    );
    assert_eq!(
        payload.pointer("/side_effect_policy/declared_side_effects/0"),
        Some(&json!("read_only"))
    );
    assert_eq!(payload.get("use_current_tab"), Some(&json!(true)));
}

#[test]
fn output_extraction_prefers_run_result_output() {
    let response = json!({
        "result": { "legacy": true },
        "run_result": {
            "version": "rzn.run_result.v2",
            "run_id": "run-1",
            "workflow_id": "x.open",
            "status": "succeeded",
            "output": { "markdown": "# done" }
        }
    });

    assert_eq!(
        extract_payload_for_output(&response),
        Some(json!({ "markdown": "# done" }))
    );
}

#[test]
fn manifest_output_selector_picks_named_step_payload() {
    let context = WorkflowRuntimeContext {
        workflow_id: "pubmed/search".to_string(),
        workflow_version: "1.0.0".to_string(),
        system: "pubmed".to_string(),
        capability: "pubmed.search".to_string(),
        declared_side_effects: vec!["read_only".to_string()],
        enforce_side_effects: true,
        output_selector_step_id: Some("extract".to_string()),
        output_selector_path: Some("$.items[0]".to_string()),
    };
    let mut outputs = HashMap::new();
    outputs.insert(
        "extract".to_string(),
        json!({ "items": [{ "title": "selected" }] }),
    );
    outputs.insert("count".to_string(), json!("42"));

    assert_eq!(
        select_workflow_output(Some(&context), &outputs, Some(json!("fallback"))),
        Some(json!({ "title": "selected" }))
    );
}

#[test]
fn manifest_with_steps_loads_as_executable_workflow() {
    let root = unique_temp_path("manifest-runtime").join("workflows");
    let workflow_dir = root.join("google");
    fs::create_dir_all(&workflow_dir).unwrap();
    let manifest_path = workflow_dir.join("google-search.json");
    fs::write(&manifest_path, MANIFEST_GOOGLE_SEARCH).unwrap();

    let workflow = load_workflow_value(&manifest_path.to_string_lossy()).unwrap();
    let steps = workflow
        .pointer("/browser_automation/sequences/0/steps")
        .and_then(|value| value.as_array())
        .expect("steps");

    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].get("type"), Some(&json!("navigate_to_url")));
    assert_eq!(
        steps[0].get("url"),
        Some(&json!("https://www.google.com/search?q={search_query}"))
    );
    assert_eq!(steps[1].get("item_selector"), Some(&json!(".result")));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn manifest_run_loader_keeps_manifest_steps_and_params_authoritative() {
    let root = unique_temp_path("manifest-native-runtime").join("workflows");
    let workflow_dir = root.join("google");
    fs::create_dir_all(&workflow_dir).unwrap();
    let manifest_path = workflow_dir.join("google-search.json");
    fs::write(
        &manifest_path,
        r#"{
          "schema_version": "rzn.workflow_manifest",
          "id": "google/search",
          "name": "Google Search",
          "version": "1.0.0",
          "system": "google",
          "capability": "google.search",
          "params": {
            "properties": {
              "search_query": {
                "kind": "string",
                "required": true
              },
              "locale": {
                "kind": "string",
                "default": "en"
              }
            }
          },
          "side_effects": [{ "class": "browser_state" }],
          "runtime": { "actor": "supervisor" },
          "steps": [
            {
              "id": "open",
              "action": {
                "kind": "navigate_to_url",
                "inputs": { "url": "https://www.google.com/search?q={search_query}&hl={locale}" },
                "side_effects": ["browser_state"]
              }
            }
          ],
          "result": {
            "output_selector": { "step_id": "open", "path": "$" }
          }
        }"#,
    )
    .unwrap();

    let loaded = load_workflow_for_run(
        &manifest_path.to_string_lossy(),
        &HashMap::from([("search_query".to_string(), "rust".to_string())]),
    )
    .unwrap();

    assert!(
        loaded.report_workflow.get("browser_automation").is_none(),
        "manifest run path must not synthesize a legacy workflow object"
    );
    assert!(matches!(loaded.steps[0], RuntimeStep::Manifest { .. }));
    let executor_step = loaded.steps[0].executor_step();
    assert_eq!(executor_step.get("type"), Some(&json!("navigate_to_url")));
    assert_eq!(
        executor_step.get("url"),
        Some(&json!("https://www.google.com/search?q=rust&hl=en"))
    );

    let missing = load_workflow_for_run(&manifest_path.to_string_lossy(), &HashMap::new())
        .expect_err("manifest params must enforce required inputs");
    assert!(missing.to_string().contains("search_query"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn manifest_run_loader_accepts_cli_array_params() {
    let root = unique_temp_path("manifest-array-runtime").join("workflows");
    let workflow_dir = root.join("demo");
    fs::create_dir_all(&workflow_dir).unwrap();
    let manifest_path = workflow_dir.join("upload.json");
    fs::write(
        &manifest_path,
        r##"{
          "schema_version": "rzn.workflow_manifest",
          "id": "demo/upload",
          "name": "Upload",
          "version": "1.0.0",
          "system": "demo",
          "capability": "demo.upload",
          "params": {
            "properties": {
              "paths": {
                "kind": "array",
                "required": true
              }
            }
          },
          "side_effects": [{ "class": "file_write" }],
          "runtime": { "actor": "supervisor" },
          "steps": [
            {
              "id": "upload",
              "action": {
                "kind": "upload_file",
                "inputs": {
                  "selector": "#file",
                  "file_path": "{paths}"
                },
                "side_effects": ["file_write"]
              }
            }
          ],
          "result": {
            "output_selector": { "step_id": "upload", "path": "$" }
          }
        }"##,
    )
    .unwrap();

    let loaded = load_workflow_for_run(
        &manifest_path.to_string_lossy(),
        &HashMap::from([("paths".to_string(), "/tmp/a.txt,/tmp/b.txt".to_string())]),
    )
    .unwrap();
    let executor_step = loaded.steps[0].executor_step();
    assert_eq!(
        executor_step.get("file_path"),
        Some(&json!("[\"/tmp/a.txt\",\"/tmp/b.txt\"]"))
    );

    let loaded_json = load_workflow_for_run(
        &manifest_path.to_string_lossy(),
        &HashMap::from([("paths".to_string(), "[\"/tmp/with space.txt\"]".to_string())]),
    )
    .unwrap();
    let executor_step = loaded_json.steps[0].executor_step();
    assert_eq!(
        executor_step.get("file_path"),
        Some(&json!("[\"/tmp/with space.txt\"]"))
    );

    let bad = load_workflow_for_run(
        &manifest_path.to_string_lossy(),
        &HashMap::from([("paths".to_string(), "[not-json".to_string())]),
    )
    .expect_err("invalid JSON array should fail before runtime");
    assert!(bad
        .to_string()
        .contains("paths: invalid JSON array parameter"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn manifest_run_loader_rejects_unknown_output_selector_step() {
    let root = unique_temp_path("manifest-selector-runtime").join("workflows");
    let workflow_dir = root.join("x");
    fs::create_dir_all(&workflow_dir).unwrap();
    let manifest_path = workflow_dir.join("x-open.json");
    fs::write(
        &manifest_path,
        r#"{
          "schema_version": "rzn.workflow_manifest",
          "id": "x/open",
          "name": "Open X",
          "version": "1.0.0",
          "system": "x",
          "capability": "x.read",
          "side_effects": [{ "class": "read_only" }],
          "runtime": { "actor": "supervisor" },
          "steps": [
            {
              "id": "extract",
              "action": {
                "kind": "extract_structured_data",
                "side_effects": ["read_only"]
              }
            }
          ],
          "result": {
            "output_selector": { "step_id": "missing", "path": "$" }
          }
        }"#,
    )
    .unwrap();

    let err = load_workflow_for_run(&manifest_path.to_string_lossy(), &HashMap::new())
        .expect_err("selector must reference a manifest step");
    assert!(err.to_string().contains("output_selector"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn step_recording_preserves_supervisor_run_result_envelope() {
    let response = json!({
        "ok": true,
        "result": { "legacy": true },
        "run_result": {
            "version": "rzn.run_result.v2",
            "run_id": "run-1",
            "workflow_id": "x.open",
            "status": "succeeded",
            "output": { "selected": true },
            "artifacts": [],
            "warnings": [],
            "steps": []
        }
    });
    let mut step_outputs = HashMap::new();
    let mut final_payload = None;

    record_step_output("extract", &response, &mut step_outputs, &mut final_payload);

    assert_eq!(
        step_outputs.get("extract"),
        Some(&json!({ "selected": true }))
    );
    let final_payload = final_payload.expect("run result");
    assert_eq!(
        final_payload
            .get("version")
            .and_then(|value| value.as_str()),
        Some("rzn.run_result.v2")
    );
    assert_eq!(
        final_payload
            .get("workflow_id")
            .and_then(|value| value.as_str()),
        Some("x.open")
    );
}

#[test]
fn response_success_treats_error_shaped_legacy_response_as_failure() {
    assert!(!response_success(&json!({
        "error": "selector not found",
        "error_code": "SELECTOR_NOT_FOUND"
    })));
    assert!(!response_success(&json!({
        "run_result": {
            "version": "rzn.run_result.v2",
            "run_id": "run-1",
            "workflow_id": "x.open",
            "status": "failed",
            "output": null,
            "artifacts": [],
            "warnings": [],
            "steps": []
        }
    })));
}

#[test]
fn browser_target_flags_are_added_to_session_and_step_payloads() {
    let target = json!({ "browser": "edge" });
    let payload = with_browser_target(json!({ "session_id": "s1" }), Some(&target));

    assert_eq!(
        payload
            .pointer("/browser_target/browser")
            .and_then(Value::as_str),
        Some("edge")
    );
}

// ---------------------------------------------------------------------------
// MockTransport suite driving execute_workflow end to end.
// ---------------------------------------------------------------------------

/// Scripts `browser.execute_step` responses in order; answers session/snapshot
/// calls automatically and records call counts.
struct MockTransport {
    session_id: Option<String>,
    step_responses: Mutex<VecDeque<Result<Value, TransportError>>>,
    execute_calls: Mutex<usize>,
    methods: Mutex<Vec<String>>,
    calls: Mutex<Vec<(String, Value)>>,
}

impl MockTransport {
    fn new(session_id: Option<&str>, steps: Vec<Result<Value, TransportError>>) -> Self {
        Self {
            session_id: session_id.map(str::to_string),
            step_responses: Mutex::new(steps.into_iter().collect()),
            execute_calls: Mutex::new(0),
            methods: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn execute_count(&self) -> usize {
        *self.execute_calls.lock().unwrap()
    }

    fn methods_called(&self) -> Vec<String> {
        self.methods.lock().unwrap().clone()
    }

    fn call_params(&self, method: &str) -> Value {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .find(|(called, _)| called == method)
            .map(|(_, params)| params.clone())
            .expect("recorded transport call")
    }
}

#[async_trait::async_trait]
impl StepTransport for MockTransport {
    async fn call(
        &self,
        method: &str,
        params: Value,
        _timeout_ms: u64,
    ) -> Result<Value, TransportError> {
        self.methods.lock().unwrap().push(method.to_string());
        self.calls
            .lock()
            .unwrap()
            .push((method.to_string(), params));
        match method {
            "browser.session_open" => Ok(json!({ "session_id": self.session_id })),
            "browser.session_close" => Ok(json!({ "ok": true })),
            "browser.snapshot" => Ok(json!({
                "dom_hash": "cafebabe",
                "dom_snapshot": {"prompt": "button#checkout", "elements": [1, 2]}
            })),
            "browser.execute_step" => {
                *self.execute_calls.lock().unwrap() += 1;
                self.step_responses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| Ok(json!({ "success": true })))
            }
            _ => Ok(json!({ "ok": true })),
        }
    }
}

#[derive(Default)]
struct RecordingSink {
    stops: Mutex<Vec<String>>,
    sessions: Mutex<Vec<Option<String>>>,
}

impl RunEventSink for RecordingSink {
    fn on_session_open(&self, session_id: Option<&str>) {
        self.sessions
            .lock()
            .unwrap()
            .push(session_id.map(str::to_string));
    }
    fn on_stop(&self, _step_id: &str, _step_type: &str, reason: &str) {
        self.stops.lock().unwrap().push(reason.to_string());
    }
}

fn test_opts(snapshot_mode: SnapshotMode) -> RunOptions {
    RunOptions {
        run_id: "test-run".to_string(),
        workflow_hash: None,
        params: HashMap::new(),
        deadline: None,
        session: SessionSpec::default(),
        snapshot_mode,
        workflow_path: "test-workflow.json".to_string(),
    }
}

#[tokio::test]
async fn fleet_session_lifecycle_payloads_include_run_metadata_and_outcome() {
    let transport = MockTransport::new(Some("fleet-session"), vec![Ok(json!({ "success": true }))]);
    let sink = RecordingSink::default();
    let mut opts = test_opts(SnapshotMode::None);
    opts.run_id = "fleet-run-1".to_string();
    opts.session = SessionSpec {
        origin: Some("fleet".to_string()),
        job_id: Some("job-1".to_string()),
        ..SessionSpec::default()
    };

    let result = execute_workflow(&transport, &sink, single_legacy_step("step", false), opts).await;

    assert_eq!(result.status, RunStatusV2::Succeeded);
    assert_eq!(
        transport.call_params("browser.session_open"),
        json!({ "origin": "fleet", "run_id": "fleet-run-1", "job_id": "job-1" }),
    );
    assert_eq!(
        transport
            .call_params("browser.session_close")
            .get("outcome"),
        Some(&json!("succeeded")),
    );
}

#[tokio::test]
async fn local_sessions_keep_the_legacy_lifecycle_payloads() {
    let transport = MockTransport::new(Some("local-session"), vec![Ok(json!({ "success": true }))]);
    let sink = RecordingSink::default();

    let result = execute_workflow(
        &transport,
        &sink,
        single_legacy_step("step", false),
        test_opts(SnapshotMode::None),
    )
    .await;

    assert_eq!(result.status, RunStatusV2::Succeeded);
    assert_eq!(transport.call_params("browser.session_open"), json!({}));
    assert_eq!(
        transport.call_params("browser.session_close"),
        json!({ "session_id": "local-session" })
    );
}

fn single_legacy_step(id: &str, external_write: bool) -> LoadedWorkflow {
    let mut step = json!({ "id": id, "type": "execute_javascript", "script": "return 1;" });
    if external_write {
        step["side_effects"] = json!(["external_write"]);
    }
    LoadedWorkflow {
        report_workflow: json!({ "id": "demo/test", "version": "1.0.0" }),
        steps: vec![RuntimeStep::Legacy(step)],
        prefer_current_tab: false,
        runtime_context: None,
    }
}

#[tokio::test]
async fn execute_workflow_runs_manifest_and_selects_step_output() {
    let root = unique_temp_path("run-happy").join("workflows");
    let workflow_dir = root.join("google");
    fs::create_dir_all(&workflow_dir).unwrap();
    let manifest_path = workflow_dir.join("google-search.json");
    fs::write(&manifest_path, MANIFEST_GOOGLE_SEARCH).unwrap();

    let workflow = load_workflow_for_run(
        &manifest_path.to_string_lossy(),
        &HashMap::from([("search_query".to_string(), "rust".to_string())]),
    )
    .unwrap();

    let transport = MockTransport::new(
        Some("sess-happy"),
        vec![
            Ok(json!({ "success": true, "result": { "navigated": true } })),
            Ok(json!({ "success": true, "result": { "items": [ { "title": "hit" } ] } })),
        ],
    );
    let sink = RecordingSink::default();

    let result = execute_workflow(&transport, &sink, workflow, test_opts(SnapshotMode::None)).await;

    assert_eq!(result.status, RunStatusV2::Succeeded);
    assert_eq!(result.workflow_id, "google/search");
    assert_eq!(
        result.output,
        Some(json!({ "items": [ { "title": "hit" } ] }))
    );
    assert_eq!(transport.execute_count(), 2);
    assert_eq!(
        sink.sessions.lock().unwrap().as_slice(),
        &[Some("sess-happy".to_string())]
    );
    // Session was opened and closed through the transport.
    assert!(transport
        .methods_called()
        .contains(&"browser.session_close".to_string()));

    let _ = fs::remove_dir_all(&root);
}

#[tokio::test]
async fn execute_workflow_applies_output_selector_path() {
    let context = WorkflowRuntimeContext {
        workflow_id: "demo/x".to_string(),
        workflow_version: "1.0.0".to_string(),
        system: "demo".to_string(),
        capability: "demo.read".to_string(),
        declared_side_effects: vec!["read_only".to_string()],
        enforce_side_effects: true,
        output_selector_step_id: Some("extract".to_string()),
        output_selector_path: Some("$.items[0].title".to_string()),
    };
    let workflow = LoadedWorkflow {
        report_workflow: json!({ "id": "demo/x", "version": "1.0.0" }),
        steps: vec![RuntimeStep::Legacy(
            json!({ "id": "extract", "type": "extract_structured_data" }),
        )],
        prefer_current_tab: false,
        runtime_context: Some(context),
    };

    let transport = MockTransport::new(
        Some("s"),
        vec![Ok(
            json!({ "success": true, "result": { "items": [ { "title": "picked" } ] } }),
        )],
    );
    let sink = RecordingSink::default();

    let result = execute_workflow(&transport, &sink, workflow, test_opts(SnapshotMode::None)).await;

    assert_eq!(result.status, RunStatusV2::Succeeded);
    assert_eq!(result.output, Some(json!("picked")));
}

#[tokio::test]
async fn execute_workflow_stops_early_on_stop_workflow() {
    let workflow = LoadedWorkflow {
        report_workflow: json!({ "id": "demo/x", "version": "1.0.0" }),
        steps: vec![
            RuntimeStep::Legacy(
                json!({ "id": "one", "type": "execute_javascript", "script": "return 1;" }),
            ),
            RuntimeStep::Legacy(
                json!({ "id": "two", "type": "execute_javascript", "script": "return 2;" }),
            ),
        ],
        prefer_current_tab: false,
        runtime_context: None,
    };

    let transport = MockTransport::new(
        Some("s"),
        vec![
            Ok(
                json!({ "success": true, "result": { "stop_workflow": true, "stop_reason": "halt" } }),
            ),
            Ok(json!({ "success": true, "result": { "ran": "two" } })),
        ],
    );
    let sink = RecordingSink::default();

    let result = execute_workflow(&transport, &sink, workflow, test_opts(SnapshotMode::None)).await;

    assert_eq!(result.status, RunStatusV2::Succeeded);
    // Second step must never run.
    assert_eq!(transport.execute_count(), 1);
    assert_eq!(sink.stops.lock().unwrap().as_slice(), &["halt".to_string()]);
}

#[tokio::test]
async fn execute_workflow_retries_transient_step_error() {
    let workflow = single_legacy_step("retry-me", false);
    let transport = MockTransport::new(
        Some("s"),
        vec![
            Ok(json!({ "success": false, "error": "Native host timeout after 1000ms" })),
            Ok(json!({ "success": true, "result": { "ok": true } })),
        ],
    );
    let sink = RecordingSink::default();

    let result = execute_workflow(&transport, &sink, workflow, test_opts(SnapshotMode::None)).await;

    assert_eq!(result.status, RunStatusV2::Succeeded);
    // One transient failure then success => two execute_step calls.
    assert_eq!(transport.execute_count(), 2);
}

#[tokio::test]
async fn execute_workflow_does_not_retry_external_write_step() {
    let workflow = single_legacy_step("post", true);
    let transport = MockTransport::new(
        Some("s"),
        vec![
            Ok(json!({ "success": false, "error": "Native host timeout after 1000ms" })),
            // Would succeed if retried — proves no retry happens.
            Ok(json!({ "success": true, "result": { "ok": true } })),
        ],
    );
    let sink = RecordingSink::default();

    let result = execute_workflow(&transport, &sink, workflow, test_opts(SnapshotMode::None)).await;

    assert_eq!(result.status, RunStatusV2::Failed);
    assert_eq!(transport.execute_count(), 1);
}

#[tokio::test]
async fn execute_workflow_surfaces_watchdog_timeout_as_failure() {
    let workflow = single_legacy_step("hang", false);
    let transport = MockTransport::new(Some("s"), vec![Err(TransportError::Timeout)]);
    let sink = RecordingSink::default();

    let result = execute_workflow(
        &transport,
        &sink,
        workflow,
        test_opts(SnapshotMode::OnError),
    )
    .await;

    assert_eq!(result.status, RunStatusV2::Failed);
    let summary = result.failure_summary.as_ref().expect("failure summary");
    assert_eq!(summary.failing_step_index, Some(0));
    assert_eq!(summary.error_class, "timeout");
    assert!(result
        .debug
        .as_ref()
        .and_then(|debug| debug.raw.as_ref())
        .and_then(|raw| raw.get("dom_excerpt"))
        .is_some());
    let error = result.error.expect("watchdog failure carries an error");
    assert!(
        error.message.contains("timed out after"),
        "unexpected message: {}",
        error.message
    );
    assert_eq!(transport.execute_count(), 1);
    // OnError mode takes a snapshot through the transport before failing.
    assert!(transport
        .methods_called()
        .iter()
        .any(|method| method == "browser.snapshot"));
}

#[tokio::test]
async fn execute_workflow_propagates_known_failing_step_into_fingerprint() {
    let workflow = LoadedWorkflow {
        report_workflow: json!({ "id": "demo/seven", "version": "1.0.0" }),
        steps: (0..7)
            .map(|index| {
                RuntimeStep::Legacy(json!({
                    "id": format!("step-{index}"),
                    "type": "click"
                }))
            })
            .collect(),
        prefer_current_tab: false,
        runtime_context: None,
    };
    let mut responses = (0..6)
        .map(|_| Ok(json!({"success": true})))
        .collect::<Vec<_>>();
    responses.push(Ok(json!({
        "success": false,
        "error": "selector not found"
    })));
    let transport = MockTransport::new(Some("s"), responses);
    let sink = RecordingSink::default();
    let mut opts = test_opts(SnapshotMode::None);
    opts.workflow_hash = Some("h".into());

    let result = execute_workflow(&transport, &sink, workflow, opts).await;

    let summary = result.failure_summary.expect("failure summary");
    assert_eq!(summary.failing_step_index, Some(6));
    assert_eq!(summary.error_class, "selector_not_found");
    assert_eq!(summary.fingerprint, "333a6f8f918a2812");
}

#[tokio::test]
async fn execute_workflow_runs_legacy_format() {
    let root = unique_temp_path("run-legacy").join("workflows");
    let workflow_dir = root.join("demo");
    fs::create_dir_all(&workflow_dir).unwrap();
    let path = workflow_dir.join("legacy.json");
    fs::write(
        &path,
        r#"{
          "id": "demo.legacy",
          "version": "1.0.0",
          "browser_automation": {
            "sequences": [{
              "steps": [
                { "id": "open", "type": "navigate_to_url", "url": "https://example.com" },
                { "id": "read", "type": "extract_structured_data" }
              ]
            }]
          }
        }"#,
    )
    .unwrap();

    let workflow = load_workflow_for_run(&path.to_string_lossy(), &HashMap::new()).unwrap();
    assert!(matches!(workflow.steps[0], RuntimeStep::Legacy(_)));

    let transport = MockTransport::new(
        Some("s"),
        vec![
            Ok(json!({ "success": true, "result": { "opened": true } })),
            Ok(json!({ "success": true, "result": { "text": "hello" } })),
        ],
    );
    let sink = RecordingSink::default();

    let result = execute_workflow(&transport, &sink, workflow, test_opts(SnapshotMode::None)).await;

    assert_eq!(result.status, RunStatusV2::Succeeded);
    assert_eq!(result.workflow_id, "rzn.legacy.workflow");
    assert_eq!(result.output, Some(json!({ "text": "hello" })));
    assert_eq!(transport.execute_count(), 2);

    let _ = fs::remove_dir_all(&root);
}

fn unique_temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{}-{}", Uuid::new_v4(), name))
}

const MANIFEST_GOOGLE_SEARCH: &str = r#"{
  "schema_version": "rzn.workflow_manifest",
  "id": "google/search",
  "name": "Google Search",
  "version": "1.0.0",
  "system": "google",
  "capability": "google.search",
  "params": {
    "properties": {
      "search_query": {
        "kind": "string",
        "required": true,
        "description": "Query text."
      }
    }
  },
  "side_effects": [
    { "class": "browser_state" },
    { "class": "read_only" }
  ],
  "runtime": { "actor": "supervisor" },
  "steps": [
    {
      "id": "open",
      "action": {
        "kind": "navigate_to_url",
        "inputs": { "url": "https://www.google.com/search?q={search_query}" },
        "side_effects": ["browser_state"]
      }
    },
    {
      "id": "extract",
      "action": {
        "kind": "extract_structured_data",
        "inputs": {
          "item_selector": ".result",
          "fields": [{ "name": "title", "selector": "h3" }]
        },
        "side_effects": ["read_only"]
      }
    }
  ],
  "result": {
    "output_schema": { "type": "array" },
    "output_selector": { "step_id": "extract", "path": "$" }
  }
}"#;
