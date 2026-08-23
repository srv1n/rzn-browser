//! Shared workflow run loop extracted from `native_runner`.
//!
//! `native_runner` is the CLI glue: it owns argument handling, the local-socket
//! JSON-RPC client (a [`StepTransport`] impl), a [`RunEventSink`] that prints the
//! `[OK]/[ERR]/[STOP]` progress lines, and post-processing. Everything about
//! *running the workflow itself* — loading/parsing canonical workflow manifests,
//! param normalization/injection, the per-step execution loop (transient
//! retry, external-write guard, per-step watchdog), stop_workflow handling, output
//! selection and `RunResult` assembly — lives here so both the CLI and the
//! supervisor's in-process fleet loop can drive it.
//!
//! The loop never prints and never dials a socket directly: it talks to the
//! browser session layer through [`StepTransport`] and reports progress through
//! [`RunEventSink`].

use crate::workflow_failure_report::{build_failure_context, WorkflowRunFailure};
use crate::workflow_params::{apply_parameters, inject_script_params};
use anyhow::{anyhow, Context, Result};
use rzn_contracts::workflow::{
    validate_manifest_value, DebugBundle, ParamDef, ParamKind, RunError, RunResult, RunStatus,
    WorkflowManifest, WorkflowStep, RUN_RESULT_CONTRACT,
};
use rzn_core::dsl;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::Path;
use tokio::time::Duration;
use uuid::Uuid;

const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 30000;
const DEFAULT_NATIVE_STEP_RPC_GRACE_MS: u64 = 5000;
/// A short bridge-recovery window is useful after an extension restart, but a
/// deadline-sized retry loop turns an upstream throttle into request hammering.
const MAX_TRANSIENT_STEP_RETRIES: usize = 2;
const TRANSIENT_STEP_RETRY_DELAY_MS: u64 = 350;

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
    /// Exact browser tab to reuse for this run, scoped by browser instance.
    pub tab_ref: Option<String>,
    /// Release session ownership without closing the dedicated tab.
    pub retain_tab_on_close: bool,
    /// Optional run lifecycle metadata forwarded to the browser session layer.
    /// Absent metadata preserves the local-session protocol.
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

/// Run a fully-loaded workflow to completion and return a typed `RunResult`.
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
) -> RunResult {
    let workflow_id = workflow
        .runtime_context
        .as_ref()
        .map(|context| context.workflow_id.clone())
        .unwrap_or_else(|| "rzn.workflow".to_string());

    match run_workflow(transport, sink, &workflow, &opts).await {
        Ok(Some(value)) => run_result_from_output_value(value, &opts.run_id, &workflow_id),
        Ok(None) => run_result_shell(RunStatus::Succeeded, None, &opts.run_id, &workflow_id, None),
        Err(err) => {
            let mut result = run_result_shell(
                RunStatus::Failed,
                None,
                &opts.run_id,
                &workflow_id,
                Some(RunError {
                    code: err
                        .downcast_ref::<WorkflowRunFailure>()
                        .and_then(|failure| failure.error_code.clone())
                        .unwrap_or_else(|| "step_failed".to_string()),
                    message: err.to_string(),
                    step_id: None,
                    retry_hint: None,
                }),
            );
            enrich_failure_result(
                &mut result,
                &err,
                opts.workflow_hash.as_deref().unwrap_or(""),
            );
            result
        }
    }
}

pub(crate) fn run_result_from_output_value(
    value: Value,
    run_id: &str,
    workflow_id: &str,
) -> RunResult {
    if let Ok(result) = serde_json::from_value::<RunResult>(value.clone()) {
        return result;
    }
    run_result_shell(RunStatus::Succeeded, Some(value), run_id, workflow_id, None)
}

pub(crate) fn run_result_shell(
    status: RunStatus,
    output: Option<Value>,
    run_id: &str,
    workflow_id: &str,
    error: Option<RunError>,
) -> RunResult {
    RunResult {
        version: RUN_RESULT_CONTRACT.to_string(),
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
    result: &mut RunResult,
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
        result.debug = Some(DebugBundle {
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
            let payload = with_tab_ref(payload, opts.session.tab_ref.as_deref());
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
            let mut transient_retries = 0usize;
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
                            take_snapshot(
                                transport,
                                sink,
                                session_id.as_deref(),
                                opts.session.tab_ref.as_deref(),
                            )
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
                            error_code: None,
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
                            error_code: None,
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

                let err_str = response_error_message(&response).unwrap_or("");
                let error_code = response_error_code(&response);
                let transient = is_transient_step_error(err_str);
                if transient
                    && !is_rate_limited_step_error(err_str, error_code)
                    && !step_writes_externally
                    && transient_retries < MAX_TRANSIENT_STEP_RETRIES
                    && tokio::time::Instant::now() < deadline
                {
                    transient_retries += 1;
                    tokio::time::sleep(Duration::from_millis(TRANSIENT_STEP_RETRY_DELAY_MS)).await;
                    continue;
                }

                sink.on_step_response(step_id, &step_type, &response);
                record_step_output(step_id, &response, &mut step_outputs, &mut final_payload);

                let failure_capture = if opts.snapshot_mode == SnapshotMode::OnError {
                    take_snapshot(
                        transport,
                        sink,
                        session_id.as_deref(),
                        opts.session.tab_ref.as_deref(),
                    )
                        .await
                        .ok()
                        .and_then(|snapshot| bounded_failure_capture(&snapshot))
                } else {
                    None
                };
                let error = response_error_message(&response).unwrap_or("unknown failure");
                let error_code = response_error_code(&response).map(str::to_string);
                let report_context = build_failure_context(
                    &workflow.report_workflow,
                    Path::new(&opts.workflow_path),
                    &executor_step,
                    idx,
                    error,
                );
                return Err(anyhow!(WorkflowRunFailure {
                    message: format!("step {} ({}) failed", step_id, step_type),
                    error_code: error_code.clone(),
                    report_context,
                    failing_step_index: idx,
                    failure_capture,
                    classification_message: error_code
                        .map(|code| format!("{}: {}", code, error))
                        .unwrap_or_else(|| error.to_string()),
                }));
            }

            if opts.snapshot_mode == SnapshotMode::AfterStep {
                let _ = take_snapshot(
                    transport,
                    sink,
                    session_id.as_deref(),
                    opts.session.tab_ref.as_deref(),
                )
                .await;
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
        let mut close_payload = with_tab_ref(
            with_session(session_id.as_deref(), json!({})),
            opts.session.tab_ref.as_deref(),
        );
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
        if opts.session.retain_tab_on_close {
            close_payload["keep_tab"] = Value::Bool(true);
        }
        let _ = transport
            .call("browser.session_close", close_payload, 0)
            .await;
    }
    result.map(|_| final_payload)
}

fn session_open_payload(opts: &RunOptions) -> Value {
    let mut payload = with_browser_target(json!({}), opts.session.browser_target.as_ref());
    payload = with_tab_ref(payload, opts.session.tab_ref.as_deref());
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

fn with_tab_ref(mut payload: Value, tab_ref: Option<&str>) -> Value {
    if let Some(tab_ref) = tab_ref.map(str::trim).filter(|value| !value.is_empty()) {
        if let Value::Object(map) = &mut payload {
            map.insert("tab_ref".to_string(), Value::String(tab_ref.to_string()));
        }
    }
    payload
}

async fn take_snapshot(
    transport: &dyn StepTransport,
    sink: &dyn RunEventSink,
    session_id: Option<&str>,
    tab_ref: Option<&str>,
) -> Result<Value> {
    let response = transport
        .call(
            "browser.snapshot",
            with_tab_ref(with_session(session_id, json!({})), tab_ref),
            0,
        )
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
// Workflow model + loading.
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
    Manifest {
        step: WorkflowStep,
        params: HashMap<String, String>,
    },
}

impl RuntimeStep {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Manifest { step, .. } => step.id.as_str(),
        }
    }

    pub(crate) fn step_type(&self) -> String {
        match self {
            Self::Manifest { step, .. } => step
                .action
                .kind
                .engine_step_type()
                .or(step.action.custom_kind.as_deref())
                .unwrap_or("custom")
                .to_string(),
        }
    }

    pub(crate) fn timeout_ms(&self) -> u64 {
        match self {
            Self::Manifest { step, .. } => {
                step.timeout_ms.unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS).max(1)
            }
        }
    }

    pub(crate) fn executor_step(&self) -> Value {
        match self {
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
/// at the top level on canonical manifest steps.
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

/// Rate limits must be surfaced to the caller immediately. Retrying them in the
/// generic bridge-recovery loop only extends the cooldown and multiplies load.
fn is_rate_limited_step_error(err_str: &str, error_code: Option<&str>) -> bool {
    let contains_rate_limit = |value: &str| {
        let lower = value.to_ascii_lowercase();
        lower.contains("429")
            || lower.contains("rate limit")
            || lower.contains("rate-limit")
            || lower.contains("rate_limit")
            || lower.contains("ratelimit")
            || lower.contains("too many requests")
    };
    contains_rate_limit(err_str) || error_code.is_some_and(contains_rate_limit)
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

/// Parse a canonical workflow manifest into a [`LoadedWorkflow`] ready
/// for [`execute_workflow`]. This is the only public constructor for the type.
pub fn load_workflow_for_run(
    path: &str,
    params: &HashMap<String, String>,
) -> Result<LoadedWorkflow> {
    let content = std::fs::read_to_string(path).with_context(|| format!("Read {}", path))?;
    let value: Value = serde_json::from_str(&content).with_context(|| "Invalid JSON workflow")?;

    let manifest = validate_manifest_value(&value).map_err(|issues| {
        anyhow!(
            "Invalid workflow manifest: {}",
            format_contract_issues(issues)
        )
    })?;
    load_manifest_workflow_for_run(Path::new(path), value, manifest, params)
}

pub(crate) fn load_workflow_value_for_run(
    value: Value,
    params: &HashMap<String, String>,
) -> Result<LoadedWorkflow> {
    let manifest = validate_manifest_value(&value).map_err(|issues| {
        anyhow!(
            "Invalid workflow manifest: {}",
            format_contract_issues(issues)
        )
    })?;
    load_manifest_workflow_for_run(Path::new("<cloud-request>"), value, manifest, params)
}

fn load_manifest_workflow_for_run(
    manifest_path: &Path,
    manifest_value: Value,
    manifest: WorkflowManifest,
    params: &HashMap<String, String>,
) -> Result<LoadedWorkflow> {
    let normalized_params = normalize_manifest_params(&manifest, params)?;
    let runtime_context = Some(runtime_context_from_manifest(manifest.clone()));

    if manifest.steps.is_empty() {
        return Err(anyhow!(
            "Workflow manifest {} must declare at least one step",
            manifest_path.display()
        ));
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
    manifest: &WorkflowManifest,
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

fn cli_manifest_param_value(field: &str, def: &ParamDef, raw: &str) -> Result<Value> {
    match def.kind {
        ParamKind::Array => cli_manifest_array_param_value(field, raw),
        ParamKind::Object => cli_manifest_object_param_value(field, raw),
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

fn load_workflow_value(path: &str) -> Result<Value> {
    let content = std::fs::read_to_string(path).with_context(|| format!("Read {}", path))?;
    let value: Value = serde_json::from_str(&content).with_context(|| "Invalid JSON workflow")?;
    validate_manifest_value(&value).map_err(|issues| {
        anyhow!(
            "Invalid workflow manifest: {}",
            format_contract_issues(issues)
        )
    })?;
    Ok(value)
}

fn format_contract_issues(issues: Vec<rzn_contracts::workflow::ContractValidationIssue>) -> String {
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

pub(crate) fn manifest_step_to_executor_step(step: &WorkflowStep) -> Value {
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

fn runtime_context_from_manifest(manifest: WorkflowManifest) -> WorkflowRuntimeContext {
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

// ---------------------------------------------------------------------------
// Step payload assembly.
// ---------------------------------------------------------------------------

pub(crate) fn step_execution_payload(
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
        .filter(|value| is_run_result(value))
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .or_else(|| {
            is_run_result(response)
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

pub(crate) fn response_error_code(response: &Value) -> Option<&str> {
    response
        .get("error_code")
        .and_then(Value::as_str)
        .or_else(|| response.pointer("/error/code").and_then(Value::as_str))
        .or_else(|| {
            response
                .pointer("/result/error_code")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            response
                .pointer("/result/error/code")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            response
                .pointer("/result/result/error_code")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            response
                .pointer("/result/result/error/code")
                .and_then(Value::as_str)
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
        .filter(|value| is_run_result(value))
        .cloned()
        .or_else(|| is_run_result(response).then(|| response.clone()))
}

fn extract_payload_for_output(response: &Value) -> Option<Value> {
    if let Some(output) = response
        .get("run_result")
        .filter(|value| is_run_result(value))
        .and_then(|value| value.get("output"))
    {
        if !matches!(output, Value::Null | Value::Bool(_)) {
            return Some(output.clone());
        }
    }

    if response.get("version").and_then(|value| value.as_str()) == Some(RUN_RESULT_CONTRACT) {
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
    if is_run_result(&output) {
        return output;
    }

    let workflow_id = runtime_context
        .map(|context| context.workflow_id.clone())
        .unwrap_or_else(|| "rzn.workflow".to_string());

    json!({
        "version": RUN_RESULT_CONTRACT,
        "run_id": format!("local-{}", Uuid::new_v4()),
        "workflow_id": workflow_id,
        "status": RunStatus::Succeeded,
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
        } else {
            let after_bracket = rest.strip_prefix('[')?;
            let end = after_bracket.find(']')?;
            let index = after_bracket[..end].parse::<usize>().ok()?;
            current = current.get(index)?;
            rest = &after_bracket[end + 1..];
        }
    }
    Some(current.clone())
}

fn is_run_result(value: &Value) -> bool {
    value.get("version").and_then(|value| value.as_str()) == Some(RUN_RESULT_CONTRACT)
}

#[cfg(test)]
mod tests;
