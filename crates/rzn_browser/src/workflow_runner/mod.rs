//! Shared workflow run loop extracted from `native_runner`.
//!
//! `native_runner` is the CLI glue: it owns argument handling, the local-socket
//! JSON-RPC client (a [`StepTransport`] impl), a [`RunEventSink`] that prints the
//! `[OK]/[ERR]/[STOP]` progress lines, and post-processing. Everything about
//! *running the workflow itself* — loading/parsing both manifest-v2 and legacy
//! formats, param normalization/injection, the per-step execution loop (transient
//! retry, external-write guard, per-step watchdog), stop_workflow handling, output
//! selection and `RunResultV2` assembly — lives here so both the CLI and the
//! supervisor's in-process fleet loop can drive it.
//!
//! The loop never prints and never dials a socket directly: it talks to the
//! browser session layer through [`StepTransport`] and reports progress through
//! [`RunEventSink`].

use crate::workflow_failure_report::{build_failure_context, WorkflowRunFailure};
use crate::workflow_params::{apply_parameters, inject_script_params};
use anyhow::{anyhow, Context, Result};
use rzn_contracts::v2::{
    validate_manifest_value, DebugBundleV1, ParamDefV2, ParamKindV2, RunErrorV1, RunResultV2,
    RunStatusV2, StepV2, WorkflowManifestV2, RUN_RESULT_VERSION,
};
use rzn_core::dsl;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tokio::time::Duration;
use uuid::Uuid;

const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 30000;
const DEFAULT_NATIVE_STEP_RPC_GRACE_MS: u64 = 5000;

// ---------------------------------------------------------------------------
// Public run-loop surface (implemented by the CLI and the fleet loop).
// ---------------------------------------------------------------------------

/// Error surfaced by a [`StepTransport`] call.
///
/// `Timeout` is the client-side watchdog firing (the request never returned
/// within its budget); `Call` wraps an underlying RPC/transport failure.
#[derive(Debug)]
pub enum TransportError {
    Timeout,
    Call(anyhow::Error),
}

impl TransportError {
    pub fn into_anyhow(self) -> anyhow::Error {
        match self {
            TransportError::Timeout => anyhow!("transport call timed out"),
            TransportError::Call(err) => err,
        }
    }
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::Timeout => f.write_str("transport call timed out"),
            TransportError::Call(err) => write!(f, "{}", err),
        }
    }
}

impl std::error::Error for TransportError {}

/// How the runner talks to the browser session layer.
///
/// `timeout_ms` is a client-side watchdog on the whole call; `0` means "no
/// watchdog, await directly" (used for session open/close and snapshots, which
/// the CLI never wrapped in an outer timeout).
#[async_trait::async_trait]
pub trait StepTransport: Send + Sync {
    async fn call(
        &self,
        method: &str,
        params: Value,
        timeout_ms: u64,
    ) -> Result<Value, TransportError>;
}

/// Progress sink so the runner never prints directly.
///
/// Every method carries the raw data the CLI needs to reproduce its exact
/// stdout lines; all methods default to no-ops so an in-process / headless
/// caller (e.g. the fleet loop) can implement only what it cares about.
pub trait RunEventSink: Send + Sync {
    /// Session opened; `session_id` is `None` when the layer returned none.
    fn on_session_open(&self, _session_id: Option<&str>) {}
    /// A step is about to run (`idx` is zero-based, `total` is the step count).
    fn on_step_start(&self, _idx: usize, _total: usize, _step_id: &str, _step_type: &str) {}
    /// A step produced a (final) response — success or the failing response.
    fn on_step_response(&self, _step_id: &str, _step_type: &str, _response: &Value) {}
    /// A best-effort snapshot completed; `dom_hash` is `None` when unavailable.
    fn on_snapshot(&self, _dom_hash: Option<&str>) {}
    /// A step requested the workflow halt early.
    fn on_stop(&self, _step_id: &str, _step_type: &str, _reason: &str) {}
    /// The assembled run result value (what the CLI pretty-prints).
    fn on_result(&self, _run_result: &Value) {}
}

/// Session target for the run (browser routing + existing-session reqs).
#[derive(Debug, Clone, Default)]
pub struct SessionSpec {
    pub browser_target: Option<Value>,
    /// Optional run lifecycle metadata forwarded to the browser session layer.
    /// Absent metadata preserves the legacy local-session protocol.
    pub origin: Option<String>,
    pub job_id: Option<String>,
}

/// Options for a single [`execute_workflow`] invocation.
pub struct RunOptions {
    pub run_id: String,
    /// Canonical hash identity used by failure fingerprints. Fleet callers pass
    /// the server-assigned value; local callers use the workflow file digest.
    pub workflow_hash: Option<String>,
    pub params: HashMap<String, String>,
    pub deadline: Option<Duration>,
    pub session: SessionSpec,
    pub snapshot_mode: SnapshotMode,
    /// Original workflow path (used only to build failure-report context).
    pub workflow_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotMode {
    None,
    AfterStep,
    OnError,
}

/// Run a fully-loaded workflow to completion and return a typed `RunResultV2`.
///
/// This is the contract entry point for both the CLI (via [`run_workflow`],
/// below, which preserves the CLI's richer `Result<Option<Value>>` behavior)
/// and the fleet loop. Progress is reported through `sink`; steps are driven
/// through `transport`.
pub async fn execute_workflow(
    transport: &dyn StepTransport,
    sink: &dyn RunEventSink,
    workflow: LoadedWorkflow,
    opts: RunOptions,
) -> RunResultV2 {
    let workflow_id = workflow
        .runtime_context
        .as_ref()
        .map(|context| context.workflow_id.clone())
        .unwrap_or_else(|| "rzn.legacy.workflow".to_string());

    match run_workflow(transport, sink, &workflow, &opts).await {
        Ok(Some(value)) => run_result_from_output_value(value, &opts.run_id, &workflow_id),
        Ok(None) => run_result_shell(
            RunStatusV2::Succeeded,
            None,
            &opts.run_id,
            &workflow_id,
            None,
        ),
        Err(err) => {
            let mut result = run_result_shell(
                RunStatusV2::Failed,
                None,
                &opts.run_id,
                &workflow_id,
                Some(RunErrorV1 {
                    code: "step_failed".to_string(),
                    message: err.to_string(),
                    step_id: None,
                    retry_hint: None,
                }),
            );
            enrich_failure_result(
                &mut result,
                &err,
                opts.workflow_hash.as_deref().unwrap_or_else(|| ""),
            );
            result
        }
    }
}

pub(crate) fn run_result_from_output_value(
    value: Value,
    run_id: &str,
    workflow_id: &str,
) -> RunResultV2 {
    if let Ok(result) = serde_json::from_value::<RunResultV2>(value.clone()) {
        return result;
    }
    run_result_shell(
        RunStatusV2::Succeeded,
        Some(value),
        run_id,
        workflow_id,
        None,
    )
}

pub(crate) fn run_result_shell(
    status: RunStatusV2,
    output: Option<Value>,
    run_id: &str,
    workflow_id: &str,
    error: Option<RunErrorV1>,
) -> RunResultV2 {
    RunResultV2 {
        version: RUN_RESULT_VERSION.to_string(),
        run_id: run_id.to_string(),
        workflow_id: workflow_id.to_string(),
        status,
        output,
        artifacts: Vec::new(),
        warnings: Vec::new(),
        steps: Vec::new(),
        debug: None,
        error,
        failure_summary: None,
    }
}

pub(crate) fn enrich_failure_result(
    result: &mut RunResultV2,
    error: &anyhow::Error,
    workflow_hash: &str,
) {
    let Some(failure) = error.downcast_ref::<WorkflowRunFailure>() else {
        return;
    };
    result.failure_summary = Some(crate::workflow_health::failure_summary(
        workflow_hash,
        Some(failure.failing_step_index),
        "step_failed",
        &failure.classification_message,
    ));
    if let Some(capture) = failure.failure_capture.clone() {
        result.debug = Some(DebugBundleV1 {
            trace_id: None,
            events: Vec::new(),
            raw: Some(capture),
        });
    }
}

/// The shared step loop. Returns the CLI's historical `Result<Option<Value>>`:
/// `Ok(Some(run_result_value))` / `Ok(None)` on success, `Err(WorkflowRunFailure)`
/// on step failure, and a plain `Err` on an underlying transport error — exactly
/// as `native_runner`'s old inline loop did. Session open/close happens here so a
/// single call is self-contained.
pub(crate) async fn run_workflow(
    transport: &dyn StepTransport,
    sink: &dyn RunEventSink,
    workflow: &LoadedWorkflow,
    opts: &RunOptions,
) -> Result<Option<Value>> {
    let mut session_id: Option<String> = None;
    let mut final_payload: Option<Value> = None;
    let mut step_outputs: HashMap<String, Value> = HashMap::new();

    let result: Result<()> = async {
        let session_resp = transport
            .call(
                "browser.session_open",
                session_open_payload(opts),
                0,
            )
            .await
            .map_err(TransportError::into_anyhow)?;
        session_id = extract_session_id(&session_resp);
        sink.on_session_open(session_id.as_deref());

        for (idx, step) in workflow.steps.iter().enumerate() {
            let step_id = step.id();
            let step_type = step.step_type();
            let executor_step = step.executor_step();

            sink.on_step_start(idx, workflow.steps.len(), step_id, &step_type);

            let timeout_ms = step.timeout_ms();
            let rpc_grace_ms = std::env::var("RZN_SUPERVISOR_STEP_RPC_GRACE_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(DEFAULT_NATIVE_STEP_RPC_GRACE_MS);
            let rpc_timeout_ms = timeout_ms.saturating_add(rpc_grace_ms).max(timeout_ms);

            if should_handle_step_locally(&step_type) {
                tokio::time::sleep(Duration::from_millis(timeout_ms)).await;
                let response = json!({ "ok": true, "success": true, "waited_ms": timeout_ms });
                sink.on_step_response(step_id, &step_type, &response);
                continue;
            }

            let payload = step_execution_payload(
                session_id.as_deref(),
                &executor_step,
                workflow.prefer_current_tab,
                workflow.runtime_context.as_ref(),
            );
            let payload = with_browser_target(payload, opts.session.browser_target.as_ref());
            // A step that performs an external write may have already applied its
            // side effect (e.g. posted a comment) even when the transport times out
            // before the response comes back. Retrying such a step risks a duplicate
            // write, so these fail fast instead of looping on transient errors.
            let step_writes_externally = step_has_external_write(&executor_step);
            // Hard per-step watchdog. Bounds the CLI's wall-clock wait to the step's
            // own timeout budget so a single hung step cannot hold the single-instance
            // browser queue open for the supervisor's global 10-minute ceiling.
            let step_watchdog_ms = rpc_timeout_ms.saturating_add(rpc_grace_ms);
            let deadline = tokio::time::Instant::now() + Duration::from_millis(rpc_timeout_ms);
            let stop_reason: Option<String>;
            loop {
                let response = match transport
                    .call(
                        "browser.execute_step",
                        with_timeout(payload.clone(), rpc_timeout_ms),
                        step_watchdog_ms,
                    )
                    .await
                {
                    Ok(response) => response,
                    Err(TransportError::Timeout) => {
                        let failure_capture = if opts.snapshot_mode == SnapshotMode::OnError {
                            take_snapshot(transport, sink, session_id.as_deref())
                                .await
                                .ok()
                                .and_then(|snapshot| bounded_failure_capture(&snapshot))
                        } else {
                            None
                        };
                        let error = format!(
                            "per-step watchdog fired after {}ms; supervisor.execute_step did not return",
                            step_watchdog_ms
                        );
                        let report_context = build_failure_context(
                            &workflow.report_workflow,
                            Path::new(&opts.workflow_path),
                            &executor_step,
                            idx,
                            &error,
                        );
                        return Err(anyhow!(WorkflowRunFailure {
                            classification_message: format!(
                                "step {} ({}) timed out after {}ms",
                                step_id, step_type, step_watchdog_ms
                            ),
                            message: format!(
                                "step {} ({}) timed out after {}ms",
                                step_id, step_type, step_watchdog_ms
                            ),
                            report_context,
                            failing_step_index: idx,
                            failure_capture,
                        }));
                    }
                    Err(TransportError::Call(err)) => {
                        let error = err.to_string();
                        let report_context = build_failure_context(
                            &workflow.report_workflow,
                            Path::new(&opts.workflow_path),
                            &executor_step,
                            idx,
                            &error,
                        );
                        return Err(anyhow!(WorkflowRunFailure {
                            message: error.clone(),
                            report_context,
                            failing_step_index: idx,
                            failure_capture: None,
                            classification_message: error,
                        }));
                    }
                };
                let success = response_success(&response);

                if success {
                    sink.on_step_response(step_id, &step_type, &response);
                    record_step_output(step_id, &response, &mut step_outputs, &mut final_payload);
                    stop_reason = response_stop_reason(&response);
                    break;
                }

                let err_str = response.get("error").and_then(|v| v.as_str()).unwrap_or("");
                let transient = is_transient_step_error(err_str);
                if transient && !step_writes_externally && tokio::time::Instant::now() < deadline {
                    tokio::time::sleep(Duration::from_millis(350)).await;
                    continue;
                }

                sink.on_step_response(step_id, &step_type, &response);
                record_step_output(step_id, &response, &mut step_outputs, &mut final_payload);

                let failure_capture = if opts.snapshot_mode == SnapshotMode::OnError {
                    take_snapshot(transport, sink, session_id.as_deref())
                        .await
                        .ok()
                        .and_then(|snapshot| bounded_failure_capture(&snapshot))
                } else {
                    None
                };
                let error = response_error_message(&response).unwrap_or("unknown failure");
                let report_context = build_failure_context(
                    &workflow.report_workflow,
                    Path::new(&opts.workflow_path),
                    &executor_step,
                    idx,
                    error,
                );
                return Err(anyhow!(WorkflowRunFailure {
                    message: format!("step {} ({}) failed", step_id, step_type),
                    report_context,
                    failing_step_index: idx,
                    failure_capture,
                    classification_message: error.to_string(),
                }));
            }

            if opts.snapshot_mode == SnapshotMode::AfterStep {
                let _ = take_snapshot(transport, sink, session_id.as_deref()).await;
            }

            if let Some(reason) = stop_reason {
                sink.on_stop(step_id, &step_type, &reason);
                break;
            }
        }

        final_payload = selected_or_fallback_output(
            workflow.runtime_context.as_ref(),
            &step_outputs,
            final_payload.take(),
        )
        .map(|payload| build_cli_run_result(workflow.runtime_context.as_ref(), payload));
        if let Some(run_result) = final_payload.as_ref() {
            sink.on_result(run_result);
        }

        Ok(())
    }
    .await;

    if session_id.is_some() {
        let mut close_payload = with_session(session_id.as_deref(), json!({}));
        if opts.session.origin.as_deref() == Some("fleet") {
            close_payload["outcome"] = Value::String(
                if result.is_ok() {
                    "succeeded"
                } else {
                    "failed"
                }
                .to_string(),
            );
        }
        let _ = transport
            .call("browser.session_close", close_payload, 0)
            .await;
    }
    result.map(|_| final_payload)
}

fn session_open_payload(opts: &RunOptions) -> Value {
    let mut payload = with_browser_target(json!({}), opts.session.browser_target.as_ref());
    let Some(origin) = opts.session.origin.as_deref() else {
        return payload;
    };
    payload["origin"] = Value::String(origin.to_string());
    payload["run_id"] = Value::String(opts.run_id.clone());
    if let Some(job_id) = opts.session.job_id.as_deref() {
        payload["job_id"] = Value::String(job_id.to_string());
    }
    payload
}

async fn take_snapshot(
    transport: &dyn StepTransport,
    sink: &dyn RunEventSink,
    session_id: Option<&str>,
) -> Result<Value> {
    let response = transport
        .call("browser.snapshot", with_session(session_id, json!({})), 0)
        .await
        .map_err(TransportError::into_anyhow)?;
    let hash = response
        .get("dom_hash")
        .and_then(|value| value.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            response
                .pointer("/result/dom_hash")
                .and_then(|value| value.as_str())
                .map(|s| s.to_string())
        });
    sink.on_snapshot(hash.as_deref());
    Ok(response)
}

fn bounded_failure_capture(snapshot: &Value) -> Option<Value> {
    const MAX_DOM_BYTES: usize = 4 * 1024;
    const MAX_SCREENSHOT_BYTES: usize = 2 * 1024 * 1024;

    let mut capture = Map::new();
    if let Some(screenshot) = find_string_field(snapshot, "screenshot_b64") {
        capture.insert(
            "screenshot_b64".into(),
            Value::String(truncate_utf8(screenshot, MAX_SCREENSHOT_BYTES)),
        );
    }
    if let Some(excerpt) = find_string_field(snapshot, "dom_excerpt") {
        capture.insert(
            "dom_excerpt".into(),
            Value::String(truncate_utf8(excerpt, MAX_DOM_BYTES)),
        );
    } else if let Some(dom_snapshot) = find_field(snapshot, "dom_snapshot") {
        if let Ok(serialized) = serde_json::to_string(dom_snapshot) {
            capture.insert(
                "dom_excerpt".into(),
                Value::String(truncate_utf8(&serialized, MAX_DOM_BYTES)),
            );
        }
    }
    (!capture.is_empty()).then_some(Value::Object(capture))
}

fn find_field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map
            .get(name)
            .or_else(|| map.values().find_map(|value| find_field(value, name))),
        Value::Array(values) => values.iter().find_map(|value| find_field(value, name)),
        _ => None,
    }
}

fn find_string_field<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    find_field(value, name).and_then(Value::as_str)
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

// ---------------------------------------------------------------------------
// Workflow model + loading (manifest-v2 and legacy).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct WorkflowRuntimeContext {
    pub(crate) workflow_id: String,
    workflow_version: String,
    system: String,
    capability: String,
    declared_side_effects: Vec<String>,
    enforce_side_effects: bool,
    output_selector_step_id: Option<String>,
    output_selector_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LoadedWorkflow {
    pub(crate) report_workflow: Value,
    pub(crate) steps: Vec<RuntimeStep>,
    pub(crate) prefer_current_tab: bool,
    pub(crate) runtime_context: Option<WorkflowRuntimeContext>,
}

#[derive(Debug, Clone)]
pub(crate) enum RuntimeStep {
    Legacy(Value),
    Manifest {
        step: StepV2,
        params: HashMap<String, String>,
    },
}

impl RuntimeStep {
    fn id(&self) -> &str {
        match self {
            Self::Legacy(step) => step.get("id").and_then(Value::as_str).unwrap_or("step"),
            Self::Manifest { step, .. } => step.id.as_str(),
        }
    }

    fn step_type(&self) -> String {
        match self {
            Self::Legacy(step) => step
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            Self::Manifest { step, .. } => step
                .action
                .kind
                .engine_step_type()
                .or(step.action.custom_kind.as_deref())
                .unwrap_or("custom")
                .to_string(),
        }
    }

    fn timeout_ms(&self) -> u64 {
        match self {
            Self::Legacy(step) => step
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .or_else(|| step.get("timeoutMs").and_then(Value::as_u64))
                .unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS)
                .max(1),
            Self::Manifest { step, .. } => {
                step.timeout_ms.unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS).max(1)
            }
        }
    }

    fn executor_step(&self) -> Value {
        match self {
            Self::Legacy(step) => step.clone(),
            Self::Manifest { step, params } => {
                let mut step = manifest_step_to_executor_step(step);
                inject_script_params(&mut step, params);
                step
            }
        }
    }
}

fn should_handle_step_locally(step_type: &str) -> bool {
    step_type == "wait_for_timeout"
}

/// True when the executor step declares an `external_write` side effect, either
/// at the top level (manifest steps) or nested under `action` (legacy steps).
fn step_has_external_write(step: &Value) -> bool {
    let contains_external_write = |value: Option<&Value>| {
        value
            .and_then(Value::as_array)
            .map(|classes| {
                classes
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|class| class.eq_ignore_ascii_case("external_write"))
            })
            .unwrap_or(false)
    };
    contains_external_write(step.get("side_effects"))
        || contains_external_write(
            step.get("action")
                .and_then(|action| action.get("side_effects")),
        )
}

fn is_transient_step_error(err_str: &str) -> bool {
    let lower = err_str.to_ascii_lowercase();
    lower.contains("receiving end does not exist")
        || lower.contains("could not establish connection")
        || lower.contains("native host timeout")
        || lower.contains("extension timeout")
        || lower.contains("native_host_disconnected")
        || lower.contains("native host disconnected")
        || lower.contains("native-host bridge response channel closed")
        || lower.contains("native-host extension bridge timeout")
        || lower.contains("broker_watchdog_timeout")
}

pub(crate) fn validate_steps(steps: &[RuntimeStep]) -> Result<()> {
    for (index, step) in steps.iter().enumerate() {
        let executor_step = step.executor_step();
        if let Err(err) = dsl::validate_action_value(&executor_step) {
            return Err(anyhow!(
                "Step {} failed schema validation: {}",
                index + 1,
                err
            ));
        }
    }
    Ok(())
}

/// Parse a workflow file (manifest-v2 or legacy) into a [`LoadedWorkflow`] ready
/// for [`execute_workflow`]. This is the only public constructor for the type.
pub fn load_workflow_for_run(
    path: &str,
    params: &HashMap<String, String>,
) -> Result<LoadedWorkflow> {
    let content = std::fs::read_to_string(path).with_context(|| format!("Read {}", path))?;
    let value: Value = serde_json::from_str(&content).with_context(|| "Invalid JSON workflow")?;

    match validate_manifest_value(&value) {
        Ok(manifest) => {
            return load_manifest_workflow_for_run(Path::new(path), value, manifest, params)
        }
        Err(issues) if is_manifest_value(&value) => {
            return Err(anyhow!(
                "Invalid manifest: {}",
                format_contract_issues(issues)
            ));
        }
        Err(_) => {}
    }

    let workflow_value = apply_parameters(value, params);
    validate_required_params(&workflow_value, params)?;
    loaded_legacy_workflow(workflow_value, load_runtime_context_for_workflow(path)?)
}

fn load_manifest_workflow_for_run(
    manifest_path: &Path,
    manifest_value: Value,
    manifest: WorkflowManifestV2,
    params: &HashMap<String, String>,
) -> Result<LoadedWorkflow> {
    let normalized_params = normalize_manifest_params(&manifest, params)?;
    let runtime_context = Some(runtime_context_from_manifest(manifest.clone()));

    if manifest.steps.is_empty() {
        let root = workflows_root_for_path(manifest_path).ok_or_else(|| {
            anyhow!(
                "cannot resolve workflows root for {}",
                manifest_path.display()
            )
        })?;
        let runtime_path = manifest_runtime_workflow_path(&root, &manifest).ok_or_else(|| {
            anyhow!(
                "Manifest {} has no steps[] and no runtime workflow pointer",
                manifest_path.display()
            )
        })?;
        let content = std::fs::read_to_string(&runtime_path)
            .with_context(|| format!("Read {}", runtime_path.display()))?;
        let runtime_value: Value =
            serde_json::from_str(&content).with_context(|| "Invalid JSON workflow")?;
        let runtime_value = apply_parameters(runtime_value, &normalized_params);
        let mut loaded = loaded_legacy_workflow(runtime_value, runtime_context)?;
        loaded.prefer_current_tab =
            loaded.prefer_current_tab || manifest.runtime.requires_existing_session;
        return Ok(loaded);
    }

    let executable_value = apply_parameters(manifest_value, &normalized_params);
    let executable_manifest = validate_manifest_value(&executable_value).map_err(|issues| {
        anyhow!(
            "Invalid manifest after parameter substitution: {:?}",
            issues
        )
    })?;
    let steps = executable_manifest
        .steps
        .iter()
        .cloned()
        .map(|step| RuntimeStep::Manifest {
            step,
            params: normalized_params.clone(),
        })
        .collect::<Vec<_>>();

    Ok(LoadedWorkflow {
        report_workflow: executable_value,
        steps,
        prefer_current_tab: executable_manifest.runtime.requires_existing_session,
        runtime_context,
    })
}

fn normalize_manifest_params(
    manifest: &WorkflowManifestV2,
    params: &HashMap<String, String>,
) -> Result<HashMap<String, String>> {
    let mut input = Map::new();
    for (key, value) in params {
        let value = match manifest.params.properties.get(key) {
            Some(def) => cli_manifest_param_value(key, def, value)?,
            None => Value::String(value.clone()),
        };
        input.insert(key.clone(), value);
    }
    let normalized = manifest
        .params
        .normalize(&Value::Object(input))
        .map_err(|issues| {
            let messages = issues
                .into_iter()
                .map(|issue| {
                    if issue.field.is_empty() {
                        issue.message
                    } else {
                        format!("{}: {}", issue.field, issue.message)
                    }
                })
                .collect::<Vec<_>>();
            anyhow!("Invalid workflow parameters: {}", messages.join(", "))
        })?;

    Ok(normalized
        .into_iter()
        .map(|(key, value)| {
            let text = match value {
                Value::String(value) => value,
                other => other.to_string(),
            };
            (key, text)
        })
        .collect())
}

fn cli_manifest_param_value(field: &str, def: &ParamDefV2, raw: &str) -> Result<Value> {
    match def.kind {
        ParamKindV2::Array => cli_manifest_array_param_value(field, raw),
        ParamKindV2::Object => cli_manifest_object_param_value(field, raw),
        _ => Ok(Value::String(raw.to_string())),
    }
}

fn cli_manifest_array_param_value(field: &str, raw: &str) -> Result<Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Value::Array(Vec::new()));
    }

    if trimmed.starts_with('[') {
        let parsed: Value = serde_json::from_str(trimmed)
            .with_context(|| format!("{field}: invalid JSON array parameter"))?;
        if parsed.is_array() {
            return Ok(parsed);
        }
        return Err(anyhow!("{field}: expected JSON array parameter"));
    }

    let values = if trimmed.contains(',') {
        trimmed
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| Value::String(value.to_string()))
            .collect()
    } else {
        vec![Value::String(trimmed.to_string())]
    };
    Ok(Value::Array(values))
}

fn cli_manifest_object_param_value(field: &str, raw: &str) -> Result<Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let parsed: Value = serde_json::from_str(trimmed)
        .with_context(|| format!("{field}: invalid JSON object parameter"))?;
    if parsed.is_object() {
        Ok(parsed)
    } else {
        Err(anyhow!("{field}: expected JSON object parameter"))
    }
}

fn loaded_legacy_workflow(
    workflow_value: Value,
    runtime_context: Option<WorkflowRuntimeContext>,
) -> Result<LoadedWorkflow> {
    let steps = extract_steps(&workflow_value)?
        .into_iter()
        .map(RuntimeStep::Legacy)
        .collect::<Vec<_>>();
    let prefer_current_tab = workflow_prefers_current_tab(&workflow_value);
    Ok(LoadedWorkflow {
        report_workflow: workflow_value,
        steps,
        prefer_current_tab,
        runtime_context,
    })
}

fn load_workflow_value(path: &str) -> Result<Value> {
    let content = std::fs::read_to_string(path).with_context(|| format!("Read {}", path))?;
    let value: Value = serde_json::from_str(&content).with_context(|| "Invalid JSON workflow")?;
    match validate_manifest_value(&value) {
        Ok(manifest) => return workflow_value_for_manifest(Path::new(path), &manifest),
        Err(issues) if is_manifest_value(&value) => {
            return Err(anyhow!(
                "Invalid manifest: {}",
                format_contract_issues(issues)
            ));
        }
        Err(_) => {}
    }
    Ok(value)
}

fn is_manifest_value(value: &Value) -> bool {
    value.get("schema_version").and_then(Value::as_str) == Some("rzn.workflow_manifest")
}

fn format_contract_issues(issues: Vec<rzn_contracts::v2::ContractValidationIssueV2>) -> String {
    issues
        .into_iter()
        .map(|issue| {
            if issue.field.is_empty() {
                issue.message
            } else {
                format!("{}: {}", issue.field, issue.message)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn load_runtime_context_for_workflow(
    workflow_path: &str,
) -> Result<Option<WorkflowRuntimeContext>> {
    let workflow_path = PathBuf::from(workflow_path);
    if let Ok(content) = fs::read_to_string(&workflow_path) {
        if let Ok(value) = serde_json::from_str::<Value>(&content) {
            if let Ok(manifest) = validate_manifest_value(&value) {
                return Ok(Some(runtime_context_from_manifest(manifest)));
            }
        }
    }

    let Some(workflows_root) = workflows_root_for_path(&workflow_path) else {
        return Ok(None);
    };

    for manifest_path in manifest_candidates(&workflows_root) {
        let Ok(content) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&content) else {
            continue;
        };
        let Ok(manifest) = validate_manifest_value(&value) else {
            continue;
        };
        let Some(runtime_path) = manifest_runtime_workflow_path(&workflows_root, &manifest) else {
            continue;
        };
        if !paths_match(&runtime_path, &workflow_path) {
            continue;
        }

        return Ok(Some(runtime_context_from_manifest(manifest)));
    }

    Ok(None)
}

fn workflow_value_for_manifest(
    manifest_path: &Path,
    manifest: &WorkflowManifestV2,
) -> Result<Value> {
    if manifest.steps.is_empty() {
        let root = workflows_root_for_path(manifest_path).ok_or_else(|| {
            anyhow!(
                "cannot resolve workflows root for {}",
                manifest_path.display()
            )
        })?;
        let runtime_path = manifest_runtime_workflow_path(&root, manifest).ok_or_else(|| {
            anyhow!(
                "Manifest {} has no steps[] and no runtime workflow pointer",
                manifest_path.display()
            )
        })?;
        return load_workflow_value(&runtime_path.to_string_lossy());
    }

    Ok(workflow_value_from_manifest_steps(manifest))
}

fn workflow_value_from_manifest_steps(manifest: &WorkflowManifestV2) -> Value {
    let mut required_variables = Vec::new();
    let mut optional_variables = Vec::new();
    for (name, def) in &manifest.params.properties {
        let variable = json!({
            "name": name,
            "description": def.description.clone().unwrap_or_default(),
            "sensitive": def.sensitive
        });
        if def.required {
            required_variables.push(variable);
        } else {
            optional_variables.push(variable);
        }
    }

    let steps = manifest
        .steps
        .iter()
        .map(manifest_step_to_executor_step)
        .collect::<Vec<_>>();

    json!({
        "system_id": manifest.system,
        "id": manifest.id,
        "name": manifest.name,
        "description": manifest.description.as_deref().or(manifest.summary.as_deref()).unwrap_or(""),
        "version": manifest.version,
        "browser_automation": {
            "use_current_tab": manifest.runtime.requires_existing_session,
            "description": manifest.description.as_deref().or(manifest.summary.as_deref()).unwrap_or(""),
            "sequences": [{
                "name": manifest.id.replace(['/', '.'], "_"),
                "description": manifest.description.as_deref().or(manifest.summary.as_deref()).unwrap_or(""),
                "required_variables": required_variables,
                "optional_variables": optional_variables,
                "steps": steps
            }]
        }
    })
}

fn manifest_step_to_executor_step(step: &StepV2) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("id".to_string(), Value::String(step.id.clone()));
    if let Some(name) = &step.name {
        map.insert("name".to_string(), Value::String(name.clone()));
    }
    let step_type = step
        .action
        .kind
        .engine_step_type()
        .or(step.action.custom_kind.as_deref())
        .unwrap_or("custom")
        .to_string();
    map.insert("type".to_string(), Value::String(step_type));
    if let Some(timeout_ms) = step.timeout_ms {
        map.insert(
            "timeout_ms".to_string(),
            Value::Number(serde_json::Number::from(timeout_ms)),
        );
    }
    if step.continue_on_error {
        map.insert("continue_on_error".to_string(), Value::Bool(true));
    }
    if !step.action.side_effects.is_empty() {
        map.insert(
            "side_effects".to_string(),
            Value::Array(
                step.action
                    .side_effects
                    .iter()
                    .map(|class| Value::String(class.as_str().to_string()))
                    .collect(),
            ),
        );
    }

    if let Some(target) = &step.action.target {
        insert_optional_string(&mut map, "encoded_id", target.encoded_id.as_deref());
        insert_optional_string(&mut map, "selector", target.selector.as_deref());
        insert_optional_string(&mut map, "text", target.text.as_deref());
        insert_optional_string(&mut map, "role", target.role.as_deref());
        insert_optional_string(&mut map, "frame_id", target.frame_id.as_deref());
    }
    for (key, value) in &step.action.inputs {
        map.insert(key.clone(), value.clone());
    }
    for (key, value) in &step.action.options {
        map.insert(key.clone(), value.clone());
    }
    Value::Object(map)
}

fn insert_optional_string(
    map: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        map.insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn runtime_context_from_manifest(manifest: WorkflowManifestV2) -> WorkflowRuntimeContext {
    let output_selector = manifest.result.output_selector.clone();
    WorkflowRuntimeContext {
        workflow_id: manifest.id,
        workflow_version: manifest.version,
        system: manifest.system,
        capability: manifest.capability,
        declared_side_effects: manifest
            .side_effects
            .iter()
            .map(|effect| effect.class.as_str().to_string())
            .collect(),
        enforce_side_effects: true,
        output_selector_step_id: output_selector
            .as_ref()
            .map(|selector| selector.step_id.clone()),
        output_selector_path: output_selector.and_then(|selector| selector.path),
    }
}

fn workflows_root_for_path(workflow_path: &Path) -> Option<PathBuf> {
    let absolute = if workflow_path.is_absolute() {
        workflow_path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(workflow_path)
    };

    for ancestor in absolute.ancestors() {
        if ancestor.file_name().and_then(|value| value.to_str()) == Some("workflows") {
            return Some(ancestor.to_path_buf());
        }
    }

    absolute.parent().map(Path::to_path_buf)
}

fn manifest_candidates(root: &Path) -> Vec<PathBuf> {
    fn visit(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, out);
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if file_name.ends_with(".json") {
                out.push(path);
            }
        }
    }

    let mut candidates = Vec::new();
    visit(root, &mut candidates);
    candidates
}

fn manifest_runtime_workflow_path(root: &Path, manifest: &WorkflowManifestV2) -> Option<PathBuf> {
    manifest
        .runtime
        .workflow_ref
        .as_deref()
        .and_then(|workflow_ref| resolve_runtime_workflow_ref(root, workflow_ref))
        .or_else(|| {
            manifest
                .runtime
                .workflow_path
                .as_deref()
                .map(|workflow_path| {
                    let path = PathBuf::from(workflow_path);
                    if path.is_absolute() {
                        path
                    } else {
                        root.join(path)
                    }
                })
        })
}

fn resolve_runtime_workflow_ref(root: &Path, workflow_ref: &str) -> Option<PathBuf> {
    let normalized = normalize_workflow_ref(workflow_ref);
    let parts = normalized.split('/').collect::<Vec<_>>();
    if parts.len() == 2 {
        let system = slugify_ref_part(parts[0]);
        let workflow = slugify_ref_part(parts[1]);
        if !system.is_empty() && !workflow.is_empty() {
            let candidates = [
                root.join(&system).join(format!("{workflow}.json")),
                root.join(&system).join(format!("{system}-{workflow}.json")),
                root.join(&system).join(format!("{system}_{workflow}.json")),
            ];
            for candidate in candidates {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
            return Some(root.join(&system).join(format!("{workflow}.json")));
        }
    }

    let path = PathBuf::from(workflow_ref);
    Some(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

fn normalize_workflow_ref(input: &str) -> String {
    input
        .trim()
        .replace('\\', "/")
        .trim_matches('/')
        .to_ascii_lowercase()
}

fn slugify_ref_part(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn paths_match(left: &Path, right: &Path) -> bool {
    canonicalize_lossy(left) == canonicalize_lossy(right)
}

fn canonicalize_lossy(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn validate_required_params(workflow: &Value, params: &HashMap<String, String>) -> Result<()> {
    let required = workflow
        .pointer("/browser_automation/sequences/0/required_variables")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    let mut missing = Vec::new();
    for var in required {
        if let Some(name) = var.get("name").and_then(|value| value.as_str()) {
            if !params.contains_key(name) {
                missing.push(name.to_string());
            }
        }
    }
    if !missing.is_empty() {
        return Err(anyhow!(
            "Missing required parameters: {}",
            missing.join(", ")
        ));
    }
    Ok(())
}

fn extract_steps(workflow: &Value) -> Result<Vec<Value>> {
    let steps = workflow
        .pointer("/browser_automation/sequences/0/steps")
        .and_then(|value| value.as_array())
        .cloned()
        .ok_or_else(|| anyhow!("Workflow missing browser_automation.sequences[0].steps"))?;
    Ok(steps)
}

fn workflow_prefers_current_tab(workflow: &Value) -> bool {
    workflow
        .pointer("/browser_automation/use_current_tab")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || workflow
            .pointer("/browser_automation/use_active_tab")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Step payload assembly.
// ---------------------------------------------------------------------------

fn step_execution_payload(
    session_id: Option<&str>,
    step: &Value,
    prefer_current_tab: bool,
    runtime_context: Option<&WorkflowRuntimeContext>,
) -> Value {
    let effective_step = apply_runtime_step_overrides(step);
    let mut payload = with_session(
        session_id,
        json!({
            "step": effective_step
        }),
    );

    if prefer_current_tab {
        payload["use_current_tab"] = Value::Bool(true);
    }

    if let Some(context) = runtime_context {
        inject_runtime_context(&mut payload, context);
    }

    payload
}

fn inject_runtime_context(payload: &mut Value, context: &WorkflowRuntimeContext) {
    let Some(map) = payload.as_object_mut() else {
        return;
    };
    map.insert(
        "workflow_id".to_string(),
        Value::String(context.workflow_id.clone()),
    );
    map.insert(
        "workflow_version".to_string(),
        Value::String(context.workflow_version.clone()),
    );
    map.insert("system".to_string(), Value::String(context.system.clone()));
    map.insert(
        "capability".to_string(),
        Value::String(context.capability.clone()),
    );
    map.insert(
        "side_effect_policy".to_string(),
        json!({
            "enforce": context.enforce_side_effects,
            "declared_side_effects": context.declared_side_effects
        }),
    );
}

fn apply_runtime_step_overrides(step: &Value) -> Value {
    let mut effective_step = step.clone();
    if step.get("type").and_then(|value| value.as_str()) != Some("request_user_intervention") {
        return effective_step;
    }

    if let Some(step_obj) = effective_step.as_object_mut() {
        if let Some(mode) = approval_mode_override_from_env() {
            step_obj.insert("approval_mode".to_string(), Value::String(mode.to_string()));
        }

        if let Some(continue_on_timeout) = continue_on_timeout_override_from_env() {
            step_obj.insert(
                "continue_on_timeout".to_string(),
                Value::Bool(continue_on_timeout),
            );
        }
    }

    effective_step
}

fn approval_mode_override_from_env() -> Option<&'static str> {
    let raw = std::env::var("RZN_APPROVAL_MODE")
        .ok()
        .or_else(|| std::env::var("RZN_INTERVENTION_POLICY").ok())?;
    let normalized = raw.trim().to_ascii_lowercase().replace(['-', ' '], "_");

    match normalized.as_str() {
        "ask_user" | "ask" | "prompt" => Some("ask_user"),
        "notify" | "notification" | "system_notify" => Some("notify"),
        "auto_continue" | "auto" | "continue" | "yolo" => Some("auto_continue"),
        "noop" | "none" | "stop" | "do_nothing" => Some("noop"),
        _ => None,
    }
}

fn continue_on_timeout_override_from_env() -> Option<bool> {
    parse_env_bool("RZN_CONTINUE_ON_TIMEOUT")
        .or_else(|| parse_env_bool("RZN_APPROVAL_CONTINUE_ON_TIMEOUT"))
}

pub(crate) fn parse_env_bool(name: &str) -> Option<bool> {
    let raw = std::env::var(name).ok()?;
    let normalized = raw.trim().to_ascii_lowercase();

    match normalized.as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn extract_session_id(response: &Value) -> Option<String> {
    response
        .get("session_id")
        .and_then(|value| value.as_str())
        .map(|text| text.to_string())
        .or_else(|| {
            response
                .pointer("/result/session_id")
                .and_then(|value| value.as_str())
                .map(|text| text.to_string())
        })
        .or_else(|| {
            response
                .pointer("/result/sessionId")
                .and_then(|value| value.as_str())
                .map(|text| text.to_string())
        })
}

fn with_session(session_id: Option<&str>, mut payload: Value) -> Value {
    if let Some(session) = session_id {
        if let Value::Object(map) = &mut payload {
            map.insert("session_id".to_string(), Value::String(session.to_string()));
        }
    }
    payload
}

pub(crate) fn with_browser_target(mut payload: Value, browser_target: Option<&Value>) -> Value {
    let Some(browser_target) = browser_target else {
        return payload;
    };
    if let Value::Object(map) = &mut payload {
        map.entry("browser_target".to_string())
            .or_insert_with(|| browser_target.clone());
    }
    payload
}

fn with_timeout(mut payload: Value, timeout_ms: u64) -> Value {
    if let Value::Object(map) = &mut payload {
        map.insert(
            "timeout_ms".to_string(),
            Value::Number(serde_json::Number::from(timeout_ms)),
        );
    }
    payload
}

// ---------------------------------------------------------------------------
// Response inspection + output selection + result assembly.
// ---------------------------------------------------------------------------

pub(crate) fn response_success(response: &Value) -> bool {
    if let Some(status) = response
        .get("run_result")
        .filter(|value| is_run_result_v2(value))
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .or_else(|| {
            is_run_result_v2(response)
                .then(|| response.get("status").and_then(Value::as_str))
                .flatten()
        })
    {
        return status == "succeeded";
    }

    let top_level = response
        .get("success")
        .and_then(|value| value.as_bool())
        .or_else(|| response.get("ok").and_then(|value| value.as_bool()));

    let nested = response
        .pointer("/result/success")
        .and_then(|value| value.as_bool())
        .or_else(|| {
            response
                .pointer("/result/ok")
                .and_then(|value| value.as_bool())
        })
        .or_else(|| {
            response
                .pointer("/result/result/success")
                .and_then(|value| value.as_bool())
        })
        .or_else(|| {
            response
                .pointer("/result/result/ok")
                .and_then(|value| value.as_bool())
        });

    if let Some(nested_success) = nested {
        return nested_success;
    }

    if response_error_message(response).is_some() || response.get("error_code").is_some() {
        return false;
    }

    top_level.unwrap_or(true)
}

fn response_stop_reason(response: &Value) -> Option<String> {
    let stop_requested = response
        .pointer("/result/stop_workflow")
        .and_then(|value| value.as_bool())
        .or_else(|| {
            response
                .pointer("/result/result/stop_workflow")
                .and_then(|value| value.as_bool())
        })
        .unwrap_or(false);

    if !stop_requested {
        return None;
    }

    response
        .pointer("/result/stop_reason")
        .and_then(|value| value.as_str())
        .or_else(|| {
            response
                .pointer("/result/result/stop_reason")
                .and_then(|value| value.as_str())
        })
        .map(|value| value.to_string())
        .or_else(|| Some("stop_requested".to_string()))
}

pub(crate) fn response_error_message(response: &Value) -> Option<&str> {
    response
        .get("error")
        .and_then(|value| value.as_str())
        .or_else(|| response.get("error_msg").and_then(|value| value.as_str()))
        .or_else(|| response.get("message").and_then(|value| value.as_str()))
        .or_else(|| {
            response
                .pointer("/result/error")
                .and_then(|value| value.as_str())
        })
        .or_else(|| {
            response
                .pointer("/result/error_msg")
                .and_then(|value| value.as_str())
        })
        .or_else(|| {
            response
                .pointer("/result/message")
                .and_then(|value| value.as_str())
        })
        .or_else(|| {
            response
                .pointer("/result/result/error")
                .and_then(|value| value.as_str())
        })
        .or_else(|| {
            response
                .pointer("/result/result/error_msg")
                .and_then(|value| value.as_str())
        })
        .or_else(|| {
            response
                .pointer("/result/result/message")
                .and_then(|value| value.as_str())
        })
}

fn record_step_output(
    step_id: &str,
    response: &Value,
    step_outputs: &mut HashMap<String, Value>,
    final_payload: &mut Option<Value>,
) {
    let output = extract_payload_for_output(response);
    if let Some(output) = output {
        step_outputs.insert(step_id.to_string(), output);
    }

    if let Some(run_result) = extract_run_result_for_output(response) {
        *final_payload = Some(run_result);
    } else if let Some(output) = extract_payload_for_output(response) {
        *final_payload = Some(output);
    }
}

fn extract_run_result_for_output(response: &Value) -> Option<Value> {
    response
        .get("run_result")
        .filter(|value| is_run_result_v2(value))
        .cloned()
        .or_else(|| is_run_result_v2(response).then(|| response.clone()))
}

fn extract_payload_for_output(response: &Value) -> Option<Value> {
    if let Some(output) = response
        .get("run_result")
        .filter(|value| is_run_result_v2(value))
        .and_then(|value| value.get("output"))
    {
        if !matches!(output, Value::Null | Value::Bool(_)) {
            return Some(output.clone());
        }
    }

    if response.get("version").and_then(|value| value.as_str()) == Some(RUN_RESULT_VERSION) {
        return response.get("output").cloned();
    }

    let primary = response
        .get("result")
        .cloned()
        .or_else(|| response.get("data").cloned())?;

    if matches!(primary, Value::Null | Value::Bool(_)) {
        return None;
    }

    if let Value::Object(map) = &primary {
        if let Some(inner) = map.get("result") {
            if !matches!(inner, Value::Null | Value::Bool(_)) {
                return Some(inner.clone());
            }
        }
        if let Some(inner) = map.get("data") {
            if !matches!(inner, Value::Null | Value::Bool(_)) {
                return Some(inner.clone());
            }
        }
    }

    Some(primary)
}

fn build_cli_run_result(runtime_context: Option<&WorkflowRuntimeContext>, output: Value) -> Value {
    if is_run_result_v2(&output) {
        return output;
    }

    let workflow_id = runtime_context
        .map(|context| context.workflow_id.clone())
        .unwrap_or_else(|| "rzn.legacy.workflow".to_string());

    json!({
        "version": RUN_RESULT_VERSION,
        "run_id": format!("local-{}", Uuid::new_v4()),
        "workflow_id": workflow_id,
        "status": RunStatusV2::Succeeded,
        "output": output,
        "artifacts": [],
        "warnings": [],
        "steps": []
    })
}

fn select_workflow_output(
    runtime_context: Option<&WorkflowRuntimeContext>,
    step_outputs: &HashMap<String, Value>,
    fallback: Option<Value>,
) -> Option<Value> {
    let context = runtime_context?;
    let step_id = context.output_selector_step_id.as_deref()?.trim();
    if step_id.is_empty() {
        return fallback;
    }

    let selected = step_outputs.get(step_id)?;
    let path = context.output_selector_path.as_deref().unwrap_or("$");
    select_json_path(selected, path).or_else(|| Some(selected.clone()))
}

fn selected_or_fallback_output(
    runtime_context: Option<&WorkflowRuntimeContext>,
    step_outputs: &HashMap<String, Value>,
    fallback: Option<Value>,
) -> Option<Value> {
    select_workflow_output(runtime_context, step_outputs, fallback.clone()).or(fallback)
}

fn select_json_path(value: &Value, path: &str) -> Option<Value> {
    let path = path.trim();
    if path.is_empty() || path == "$" {
        return Some(value.clone());
    }
    let mut current = value;
    let mut rest = path.strip_prefix('$')?;
    while !rest.is_empty() {
        if let Some(after_dot) = rest.strip_prefix('.') {
            let end = after_dot.find(['.', '[']).unwrap_or(after_dot.len());
            let key = &after_dot[..end];
            if key.is_empty() {
                return None;
            }
            current = current.get(key)?;
            rest = &after_dot[end..];
        } else if let Some(after_bracket) = rest.strip_prefix('[') {
            let end = after_bracket.find(']')?;
            let index = after_bracket[..end].parse::<usize>().ok()?;
            current = current.get(index)?;
            rest = &after_bracket[end + 1..];
        } else {
            return None;
        }
    }
    Some(current.clone())
}

fn is_run_result_v2(value: &Value) -> bool {
    value.get("version").and_then(|value| value.as_str()) == Some(RUN_RESULT_VERSION)
}

#[cfg(test)]
mod tests;
