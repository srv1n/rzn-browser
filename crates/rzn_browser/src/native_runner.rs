use crate::supervisor;
use crate::workflow_catalog::default_runtime_dir;
use crate::workflow_failure_report::{build_failure_context, WorkflowRunFailure};
use anyhow::{anyhow, Context, Result};
use interprocess::local_socket::traits::tokio::Stream as _;
use interprocess::local_socket::{
    tokio::Stream as LocalSocketStream, GenericFilePath, GenericNamespaced, ToFsName, ToNsName,
};
use rzn_broker_endpoint::{
    endpoint_pid_is_live, prune_stale_broker_endpoint, BrokerEndpointPruneReport,
};
use rzn_contracts::v2::{
    validate_manifest_value, ParamDefV2, ParamKindV2, RunStatusV2, StepV2, WorkflowManifestV2,
    RUN_RESULT_VERSION,
};
use rzn_core::dsl;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::{Duration as StdDuration, SystemTime};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::time::{timeout, Duration};
use uuid::Uuid;

const DEFAULT_ATTACH_TIMEOUT_MS: u64 = 4000;
const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 30000;
const MAX_FRAME_SIZE: usize = 25 * 1024 * 1024;
const DEFAULT_SPAWN_ENDPOINT_TIMEOUT_MS: u64 = 12000;
const DEFAULT_NATIVE_HOST_WAIT_MS: u64 = 45000;
const DEFAULT_DESKTOP_STEP_RPC_GRACE_MS: u64 = 5000;
const DEFAULT_NATIVE_STEP_RPC_GRACE_MS: u64 = 5000;
const DEFAULT_SPAWN_LOCK_WAIT_MS: u64 = 15000;
const STALE_SPAWN_LOCK_AGE_SECS: u64 = 60;
const DEFAULT_NATIVE_SELF_HEAL_ATTEMPTS: usize = 1;
const BROWSER_WORKER_SPAWN_LOCK_FILENAME: &str = "browser_worker_spawn.lock";
const LIVE_WORKER_ATTACH_RETRY_MS: u64 = 3000;
const WORKER_HEALTHCHECK_TIMEOUT_MS: u64 = 1500;
const BROKER_ENDPOINT_FILENAME: &str = "broker_endpoint_v1.json";
const SECURE_DIRNAME: &str = "secure";

fn should_handle_step_locally(step_type: &str) -> bool {
    step_type == "wait_for_timeout"
}

fn is_transient_step_error(err_str: &str) -> bool {
    err_str.contains("Receiving end does not exist")
        || err_str.contains("Could not establish connection")
        || err_str.contains("Native host timeout")
        || err_str.contains("Extension timeout")
}

#[derive(Debug, Clone)]
pub struct DesktopRunConfig {
    pub workflow_path: String,
    pub params: HashMap<String, String>,
    pub app_base: Option<String>,
    pub endpoint_path: Option<String>,
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeRunMode {
    Auto,
    Attach,
    Spawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotMode {
    None,
    AfterStep,
    OnError,
}

#[derive(Debug, Clone)]
pub struct NativeRunConfig {
    pub workflow_path: String,
    pub params: HashMap<String, String>,
    pub mode: NativeRunMode,
    pub snapshot_mode: SnapshotMode,
    pub app_base: Option<String>,
    pub endpoint_path: Option<String>,
    pub worker_cmd: Option<String>,
    pub worker_args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SupervisorRunConfig {
    pub workflow_path: String,
    pub params: HashMap<String, String>,
    pub mode: NativeRunMode,
    pub snapshot_mode: SnapshotMode,
    pub app_base: Option<String>,
    pub endpoint_path: Option<String>,
    pub worker_cmd: Option<String>,
    pub worker_args: Vec<String>,
    pub allow_legacy_worker_fallback: bool,
}

#[derive(Debug, Clone)]
pub struct NativeHealConfig {
    pub app_base: Option<String>,
    pub endpoint_path: Option<String>,
    pub restart_native_host: bool,
    pub reset_worker: bool,
    pub spawn_worker: bool,
    pub worker_cmd: Option<String>,
    pub worker_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeHealReport {
    pub endpoint_reports: Vec<BrokerEndpointPruneReport>,
    pub restarted_native_hosts: Vec<String>,
    pub reset_worker_endpoints: Vec<String>,
    pub spawned_worker: bool,
    pub worker_health: Option<Value>,
    pub supervisor: Option<Value>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
struct WorkflowRuntimeContext {
    workflow_id: String,
    workflow_version: String,
    system: String,
    capability: String,
    declared_side_effects: Vec<String>,
    enforce_side_effects: bool,
    output_selector_step_id: Option<String>,
    output_selector_path: Option<String>,
}

#[derive(Debug, Clone)]
struct LoadedWorkflow {
    report_workflow: Value,
    steps: Vec<RuntimeStep>,
    prefer_current_tab: bool,
    runtime_context: Option<WorkflowRuntimeContext>,
}

#[derive(Debug, Clone)]
enum RuntimeStep {
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

#[derive(Debug, Clone)]
enum EndpointTransport {
    Tcp { host: String, port: u16 },
    Pipe { path: String, namespaced: bool },
    Stdio { command: String, args: Vec<String> },
}

#[derive(Debug, Clone)]
struct EndpointSpec {
    transport: EndpointTransport,
    token_path: Option<String>,
}

#[derive(Debug)]
struct NativeRunPreflightFailure {
    message: String,
}

impl NativeRunPreflightFailure {
    fn new(err: impl fmt::Display) -> Self {
        Self {
            message: err.to_string(),
        }
    }
}

impl fmt::Display for NativeRunPreflightFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "native runtime preflight failed: {}", self.message)
    }
}

impl std::error::Error for NativeRunPreflightFailure {}

pub async fn run_desktop_workflow(config: DesktopRunConfig) -> Result<Option<Value>> {
    let workflow = load_workflow_for_run(&config.workflow_path, &config.params)?;
    validate_steps(&workflow.steps)?;

    let endpoint_path = resolve_desktop_endpoint_path(&config)?;
    let (socket_path, token_path, profile) = load_desktop_broker_endpoint(&endpoint_path, &config)?;

    println!(
        "[INFO] Desktop broker endpoint: {}",
        endpoint_path.to_string_lossy()
    );
    println!("[INFO] Desktop broker socket: {}", socket_path.display());

    let mut client = DesktopBrokerClient::connect(&socket_path, &token_path, &profile).await?;
    let mut final_payload: Option<Value> = None;
    let mut step_outputs: HashMap<String, Value> = HashMap::new();

    // The desktop broker tool surface (`rzn.browser.session`) expects `session_id` in payload for
    // stateful automation commands like execute_step/snapshot.
    let session_id = Uuid::new_v4().to_string();

    let wait_ms = std::env::var("RZN_WAIT_DESKTOP_NATIVE_HOST_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_NATIVE_HOST_WAIT_MS);
    if wait_ms > 0 {
        wait_for_desktop_native_host(&mut client, wait_ms).await?;
    }

    for (idx, step) in workflow.steps.iter().enumerate() {
        let step_id = step.id();
        let step_type = step.step_type();
        let executor_step = step.executor_step();

        println!(
            "[STEP] {}/{} {} ({})",
            idx + 1,
            workflow.steps.len(),
            step_id,
            step_type
        );

        let timeout_ms = step.timeout_ms();
        let rpc_grace_ms = std::env::var("RZN_DESKTOP_STEP_RPC_GRACE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_DESKTOP_STEP_RPC_GRACE_MS);
        let rpc_timeout_ms = timeout_ms.saturating_add(rpc_grace_ms).max(timeout_ms);

        // Some desktop extension builds route every step through a content script; immediately after
        // navigation, that script may not be ready yet. Two pragmatic mitigations:
        // 1) handle simple sleeps locally for wait_for_timeout
        // 2) retry transient "Receiving end does not exist" failures briefly
        if should_handle_step_locally(&step_type) {
            tokio::time::sleep(Duration::from_millis(timeout_ms)).await;
            let response = json!({ "ok": true, "success": true, "waited_ms": timeout_ms });
            log_step_response(step_id, &step_type, &response);
            continue;
        }

        let payload = step_execution_payload(
            Some(&session_id),
            &executor_step,
            workflow.prefer_current_tab,
            workflow.runtime_context.as_ref(),
        );
        let deadline = tokio::time::Instant::now() + Duration::from_millis(rpc_timeout_ms);
        let stop_reason: Option<String>;
        loop {
            let response = client
                .browser_session("execute_step", payload.clone(), rpc_timeout_ms)
                .await?;

            let success = response_success(&response);
            if success {
                log_step_response(step_id, &step_type, &response);
                record_step_output(step_id, &response, &mut step_outputs, &mut final_payload);
                stop_reason = response_stop_reason(&response);
                break;
            }

            let err_str = response.get("error").and_then(|v| v.as_str()).unwrap_or("");
            let transient = is_transient_step_error(err_str);
            if transient && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(350)).await;
                continue;
            }

            log_step_response(step_id, &step_type, &response);
            let error = response_error_message(&response).unwrap_or("unknown failure");
            let report_context = build_failure_context(
                &workflow.report_workflow,
                Path::new(&config.workflow_path),
                &executor_step,
                idx,
                error,
            );
            return Err(anyhow!(WorkflowRunFailure {
                message: format!("step {} ({}) failed", step_id, step_type),
                report_context,
            }));
        }

        if let Some(reason) = stop_reason {
            println!(
                "[STOP] Workflow halted after {} ({}): {}",
                step_id, step_type, reason
            );
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
        if let Ok(pretty) = serde_json::to_string_pretty(run_result) {
            println!("{}", pretty);
        } else {
            println!("{}", run_result);
        }
    }

    Ok(final_payload)
}

pub async fn run_native_workflow(config: NativeRunConfig) -> Result<Option<Value>> {
    let workflow = load_workflow_for_run(&config.workflow_path, &config.params)?;
    validate_steps(&workflow.steps)?;

    let max_self_heal_attempts = native_self_heal_attempts(config.mode);
    let mut attempt = 0usize;
    loop {
        match run_native_workflow_once(&config, &workflow).await {
            Ok(payload) => return Ok(payload),
            Err(err) if attempt < max_self_heal_attempts && is_preflight_native_error(&err) => {
                attempt += 1;
                println!(
                    "[HEAL] Native runtime preflight failed; resetting worker/native-host and retrying ({}/{})...",
                    attempt, max_self_heal_attempts
                );
                self_heal_native_runtime(&config).await;
                continue;
            }
            Err(err) => return Err(err),
        }
    }
}

pub async fn run_supervisor_workflow(config: SupervisorRunConfig) -> Result<Option<Value>> {
    let workflow = load_workflow_for_run(&config.workflow_path, &config.params)?;
    validate_steps(&workflow.steps)?;

    let supervisor_config = supervisor::SupervisorConfig {
        app_base: config.app_base.as_ref().map(PathBuf::from),
        endpoint_path: config.endpoint_path.clone(),
        mode: config.mode,
        worker_cmd: config.worker_cmd.clone(),
        worker_args: config.worker_args.clone(),
        allow_legacy_worker_fallback: config.allow_legacy_worker_fallback,
    };
    supervisor::ensure_running(supervisor_config.clone()).await?;
    let readiness =
        supervisor::call(supervisor_config.clone(), "runtime.ensure_ready", json!({})).await?;
    if readiness.get("ok").and_then(|value| value.as_bool()) != Some(true) {
        let message = readiness
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("supervisor runtime is not ready");
        anyhow::bail!("{}", message);
    }

    let mut session_id: Option<String> = None;
    let mut final_payload: Option<Value> = None;
    let mut step_outputs: HashMap<String, Value> = HashMap::new();

    let result: Result<()> = async {
        let session_resp =
            supervisor::call(supervisor_config.clone(), "browser.session_open", json!({})).await?;
        session_id = extract_session_id(&session_resp);
        if let Some(session) = session_id.as_ref() {
            println!("[OK] Session opened: {}", session);
        } else {
            println!("[WARN] Session opened (no session_id returned)");
        }

        for (idx, step) in workflow.steps.iter().enumerate() {
            let step_id = step.id();
            let step_type = step.step_type();
            let executor_step = step.executor_step();

            println!(
                "[STEP] {}/{} {} ({})",
                idx + 1,
                workflow.steps.len(),
                step_id,
                step_type
            );

            let timeout_ms = step.timeout_ms();
            let rpc_grace_ms = std::env::var("RZN_SUPERVISOR_STEP_RPC_GRACE_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(DEFAULT_NATIVE_STEP_RPC_GRACE_MS);
            let rpc_timeout_ms = timeout_ms.saturating_add(rpc_grace_ms).max(timeout_ms);

            if should_handle_step_locally(&step_type) {
                tokio::time::sleep(Duration::from_millis(timeout_ms)).await;
                let response = json!({ "ok": true, "success": true, "waited_ms": timeout_ms });
                log_step_response(step_id, &step_type, &response);
                continue;
            }

            let payload = step_execution_payload(
                session_id.as_deref(),
                &executor_step,
                workflow.prefer_current_tab,
                workflow.runtime_context.as_ref(),
            );
            let deadline = tokio::time::Instant::now() + Duration::from_millis(rpc_timeout_ms);
            let stop_reason: Option<String>;
            loop {
                let response = supervisor::call(
                    supervisor_config.clone(),
                    "browser.execute_step",
                    with_timeout(payload.clone(), rpc_timeout_ms),
                )
                .await?;
                let success = response_success(&response);

                if success {
                    log_step_response(step_id, &step_type, &response);
                    record_step_output(step_id, &response, &mut step_outputs, &mut final_payload);
                    stop_reason = response_stop_reason(&response);
                    break;
                }

                let err_str = response.get("error").and_then(|v| v.as_str()).unwrap_or("");
                let transient = is_transient_step_error(err_str);
                if transient && tokio::time::Instant::now() < deadline {
                    tokio::time::sleep(Duration::from_millis(350)).await;
                    continue;
                }

                log_step_response(step_id, &step_type, &response);
                record_step_output(step_id, &response, &mut step_outputs, &mut final_payload);

                if config.snapshot_mode == SnapshotMode::OnError {
                    let _ =
                        take_supervisor_snapshot(&supervisor_config, session_id.as_deref()).await;
                }
                let error = response_error_message(&response).unwrap_or("unknown failure");
                let report_context = build_failure_context(
                    &workflow.report_workflow,
                    Path::new(&config.workflow_path),
                    &executor_step,
                    idx,
                    error,
                );
                return Err(anyhow!(WorkflowRunFailure {
                    message: format!("step {} ({}) failed", step_id, step_type),
                    report_context,
                }));
            }

            if config.snapshot_mode == SnapshotMode::AfterStep {
                let _ = take_supervisor_snapshot(&supervisor_config, session_id.as_deref()).await;
            }

            if let Some(reason) = stop_reason {
                println!(
                    "[STOP] Workflow halted after {} ({}): {}",
                    step_id, step_type, reason
                );
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
            if let Ok(pretty) = serde_json::to_string_pretty(run_result) {
                println!("{}", pretty);
            } else {
                println!("{}", run_result);
            }
        }

        Ok(())
    }
    .await;

    if session_id.is_some() {
        let _ = supervisor::call(
            supervisor_config,
            "browser.session_close",
            with_session(session_id.as_deref(), json!({})),
        )
        .await;
    }
    result.map(|_| final_payload)
}

pub async fn heal_native_runtime(config: NativeHealConfig) -> Result<NativeHealReport> {
    let mut report = NativeHealReport {
        endpoint_reports: Vec::new(),
        restarted_native_hosts: Vec::new(),
        reset_worker_endpoints: Vec::new(),
        spawned_worker: false,
        worker_health: None,
        supervisor: None,
        notes: Vec::new(),
    };

    let endpoint_paths = native_heal_endpoint_paths(&config);
    if endpoint_paths.is_empty() {
        report
            .notes
            .push("No runtime endpoint paths were found or derivable".to_string());
    }

    for endpoint_path in &endpoint_paths {
        if let Some(app_base) = app_base_from_endpoint_path(endpoint_path) {
            if let Ok(prune_report) = prune_stale_broker_endpoint(&app_base) {
                report.endpoint_reports.push(prune_report);
            }
        }

        if config.restart_native_host {
            if let Ok(Some(native_host_path)) = query_native_host_path(endpoint_path).await {
                restart_native_host(&native_host_path).await?;
                if !report
                    .restarted_native_hosts
                    .iter()
                    .any(|existing| existing == &native_host_path)
                {
                    report.restarted_native_hosts.push(native_host_path);
                }
            }
        }

        if config.reset_worker {
            let _lock = match app_base_from_endpoint_path(endpoint_path) {
                Some(app_base) => acquire_spawn_lock(&app_base).await.ok(),
                None => None,
            };
            terminate_browser_worker_at_endpoint(endpoint_path).await?;
            remove_browser_worker_socket_artifacts(endpoint_path)?;
            if let Some(app_base) = app_base_from_endpoint_path(endpoint_path) {
                if let Ok(prune_report) = prune_stale_broker_endpoint(&app_base) {
                    report.endpoint_reports.push(prune_report);
                }
            }
            report
                .reset_worker_endpoints
                .push(endpoint_path.to_string_lossy().to_string());
        }
    }

    if config.spawn_worker {
        let run_config = NativeRunConfig {
            workflow_path: String::new(),
            params: HashMap::new(),
            mode: NativeRunMode::Spawn,
            snapshot_mode: SnapshotMode::OnError,
            app_base: config.app_base.clone(),
            endpoint_path: config.endpoint_path.clone(),
            worker_cmd: config.worker_cmd.clone(),
            worker_args: config.worker_args.clone(),
        };
        let mut client = spawn_worker(&run_config).await?;
        report.spawned_worker = true;
        report.worker_health = client
            .send_request("rzn.worker.health", json!({}))
            .await
            .ok();
        client.shutdown().await;
    }

    let supervisor_config = supervisor::SupervisorConfig {
        app_base: config.app_base.as_ref().map(PathBuf::from),
        endpoint_path: config.endpoint_path.clone(),
        mode: NativeRunMode::Auto,
        worker_cmd: config.worker_cmd.clone(),
        worker_args: config.worker_args.clone(),
        allow_legacy_worker_fallback: false,
    };
    match supervisor::ensure_running(supervisor_config.clone()).await {
        Ok(_) => match supervisor::call(supervisor_config, "runtime.heal", json!({})).await {
            Ok(value) => {
                report.supervisor = Some(value);
            }
            Err(err) => {
                report
                    .notes
                    .push(format!("Supervisor heal failed after startup: {}", err));
            }
        },
        Err(err) => {
            report
                .notes
                .push(format!("Supervisor startup failed during heal: {}", err));
        }
    }

    Ok(report)
}

async fn run_native_workflow_once(
    config: &NativeRunConfig,
    workflow: &LoadedWorkflow,
) -> Result<Option<Value>> {
    let mut client = connect_native(config)
        .await
        .map_err(NativeRunPreflightFailure::new)?;
    let mut session_id: Option<String> = None;
    let mut final_payload: Option<Value> = None;
    let mut step_outputs: HashMap<String, Value> = HashMap::new();

    let result: Result<()> = async {
        let session_resp = client
            .send_request("browser.session_open", json!({}))
            .await
            .map_err(NativeRunPreflightFailure::new)?;
        session_id = extract_session_id(&session_resp);

        if let Some(session) = session_id.as_ref() {
            println!("[OK] Session opened: {}", session);
        } else {
            println!("[WARN] Session opened (no session_id returned)");
        }

        // When attaching/spawning a desktop-built worker, it may come up before the browser extension
        // launches the native host and connects to the worker bridge. Wait briefly so the first step
        // doesn't fail with "No native host connected".
        let wait_ms = std::env::var("RZN_WAIT_NATIVE_HOST_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_NATIVE_HOST_WAIT_MS);
        if wait_ms > 0 {
            wait_for_native_host(&mut client, wait_ms)
                .await
                .map_err(NativeRunPreflightFailure::new)?;
        }

        for (idx, step) in workflow.steps.iter().enumerate() {
            let step_id = step.id();
            let step_type = step.step_type();
            let executor_step = step.executor_step();

            println!(
                "[STEP] {}/{} {} ({})",
                idx + 1,
                workflow.steps.len(),
                step_id,
                step_type
            );

            let timeout_ms = step.timeout_ms();
            let rpc_grace_ms = std::env::var("RZN_NATIVE_STEP_RPC_GRACE_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(DEFAULT_NATIVE_STEP_RPC_GRACE_MS);
            let rpc_timeout_ms = timeout_ms.saturating_add(rpc_grace_ms).max(timeout_ms);

            // Native-run can hit the same content-script race as desktop-run immediately after
            // navigation. Keep the mitigation consistent across both paths:
            // 1) handle pure waits locally
            // 2) retry transient messaging failures briefly
            if should_handle_step_locally(&step_type) {
                tokio::time::sleep(Duration::from_millis(timeout_ms)).await;
                let response = json!({ "ok": true, "success": true, "waited_ms": timeout_ms });
                log_step_response(step_id, &step_type, &response);
                continue;
            }

            let payload = step_execution_payload(
                session_id.as_deref(),
                &executor_step,
                workflow.prefer_current_tab,
                workflow.runtime_context.as_ref(),
            );
            let deadline = tokio::time::Instant::now() + Duration::from_millis(rpc_timeout_ms);
            let stop_reason: Option<String>;
            loop {
                let response = client
                    .send_request_with_timeout(
                        "browser.execute_step",
                        payload.clone(),
                        rpc_timeout_ms,
                    )
                    .await?;
                let success = response_success(&response);

                if success {
                    log_step_response(step_id, &step_type, &response);
                    record_step_output(step_id, &response, &mut step_outputs, &mut final_payload);
                    stop_reason = response_stop_reason(&response);
                    break;
                }

                let err_str = response.get("error").and_then(|v| v.as_str()).unwrap_or("");
                let transient = is_transient_step_error(err_str);
                if transient && tokio::time::Instant::now() < deadline {
                    tokio::time::sleep(Duration::from_millis(350)).await;
                    continue;
                }

                log_step_response(step_id, &step_type, &response);
                record_step_output(step_id, &response, &mut step_outputs, &mut final_payload);

                if config.snapshot_mode == SnapshotMode::OnError {
                    let _ = take_snapshot(&mut client, session_id.as_deref()).await;
                }
                let error = response_error_message(&response).unwrap_or("unknown failure");
                let report_context = build_failure_context(
                    &workflow.report_workflow,
                    Path::new(&config.workflow_path),
                    &executor_step,
                    idx,
                    error,
                );
                return Err(anyhow!(WorkflowRunFailure {
                    message: format!("step {} ({}) failed", step_id, step_type),
                    report_context,
                }));
            }

            if config.snapshot_mode == SnapshotMode::AfterStep {
                let _ = take_snapshot(&mut client, session_id.as_deref()).await;
            }

            if let Some(reason) = stop_reason {
                println!(
                    "[STOP] Workflow halted after {} ({}): {}",
                    step_id, step_type, reason
                );
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
            if let Ok(pretty) = serde_json::to_string_pretty(run_result) {
                println!("{}", pretty);
            } else {
                println!("{}", run_result);
            }
        }

        Ok(())
    }
    .await;

    if session_id.is_some() {
        let _ = client
            .send_request(
                "browser.session_close",
                with_session(session_id.as_deref(), json!({})),
            )
            .await;
    }
    client.shutdown().await;
    result.map(|_| final_payload)
}

async fn wait_for_native_host(client: &mut NativeClient, timeout_ms: u64) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    let mut printed_banner = false;
    let mut restart_attempted = false;
    let mut worker_reset_attempted = false;
    let should_restart = restart_native_host_enabled();
    loop {
        if tokio::time::Instant::now() > deadline {
            // Best-effort: include remediation text if the worker exposes it.
            if let Ok(health) = client.send_request("rzn.worker.health", json!({})).await {
                print_worker_health_summary(&health);
            }
            return Err(anyhow!(
                "Timed out waiting for native host connection ({}ms). If Chrome is already open, reload the RZN extension to restart the native host and re-try.",
                timeout_ms
            ));
        }

        let health = match client.send_request("rzn.worker.health", json!({})).await {
            Ok(v) => v,
            Err(err) => {
                return Err(anyhow!("browser worker health check failed: {}", err));
            }
        };

        let ok = health.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if ok {
            return Ok(());
        }

        if !printed_banner {
            printed_banner = true;
            println!("[WAIT] Waiting for native host/extension connection...");
            print_worker_health_summary(&health);
        }

        if should_restart && !restart_attempted {
            // Best-effort dev convenience: if a native host is already running, it may be connected
            // to the broker instead of the newly-started browser-bridge. Restarting it forces a
            // reconnect, allowing it to pick up the bridge endpoint.
            if let Some(path) = health
                .pointer("/details/native_host_path")
                .and_then(|v| v.as_str())
                .filter(|v| !v.trim().is_empty())
            {
                restart_attempted = true;
                println!("[WAIT] Restarting native host to pick up browser-bridge endpoint...");
                let _ = restart_native_host(path).await;
            }
        }

        if should_restart
            && restart_attempted
            && !worker_reset_attempted
            && health_indicates_stale_worker_handshake(&health)
        {
            worker_reset_attempted = true;
            println!(
                "[HEAL] Browser worker accepted a stale native-host bridge; resetting worker..."
            );
            if reset_browser_worker_for_client(client).await? {
                return Err(anyhow!(
                    "Native runtime self-healed by resetting a stale browser worker; retry required"
                ));
            }
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn restart_native_host_enabled() -> bool {
    parse_env_bool("RZN_RESTART_NATIVE_HOST")
        .or_else(|| parse_env_bool("RZN_DISABLE_NATIVE_HOST_RESTART").map(|v| !v))
        .unwrap_or(true)
}

async fn restart_native_host(native_host_path: &str) -> Result<()> {
    // Chrome owns native-host launch. Terminating the host is the least invasive way to make the
    // extension reconnect to the freshest browser-bridge endpoint.
    if cfg!(unix) {
        let _ = Command::new("pkill")
            .arg("-TERM")
            .arg("-f")
            .arg(native_host_path)
            .status()
            .await;
        tokio::time::sleep(Duration::from_millis(750)).await;
    }
    Ok(())
}

fn health_bool(health: &Value, key: &str) -> bool {
    health
        .pointer(&format!("/details/{}", key))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn health_u64(health: &Value, key: &str) -> u64 {
    health
        .pointer(&format!("/details/{}", key))
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

fn health_indicates_stale_worker_handshake(health: &Value) -> bool {
    health_bool(health, "bridge_connected")
        && !health_bool(health, "native_host_connected")
        && !health_bool(health, "extension_connected")
        && health_u64(health, "bridge_host_count") > 0
}

async fn reset_browser_worker_for_client(client: &NativeClient) -> Result<bool> {
    let Some(endpoint_path) = client.reconnect_endpoint_path.as_ref() else {
        return Ok(false);
    };
    terminate_browser_worker_at_endpoint(endpoint_path).await?;
    remove_browser_worker_socket_artifacts(endpoint_path)?;
    Ok(true)
}

async fn self_heal_native_runtime(config: &NativeRunConfig) {
    for endpoint_path in native_self_heal_endpoint_paths(config) {
        let _ = prune_stale_endpoint_path(&endpoint_path);
        let _lock = match app_base_from_endpoint_path(&endpoint_path) {
            Some(app_base) => acquire_spawn_lock(&app_base).await.ok(),
            None => None,
        };
        if let Ok(Some(native_host_path)) = query_native_host_path(&endpoint_path).await {
            let _ = restart_native_host(&native_host_path).await;
        }
        let _ = terminate_browser_worker_at_endpoint(&endpoint_path).await;
        let _ = remove_browser_worker_socket_artifacts(&endpoint_path);
        let _ = prune_stale_endpoint_path(&endpoint_path);
    }
}

fn native_self_heal_endpoint_paths(config: &NativeRunConfig) -> Vec<PathBuf> {
    if let Some(path) = endpoint_path_arg(config.endpoint_path.as_ref()) {
        return vec![path];
    }
    if config.app_base.is_some()
        || env_app_base(&["APP_BASE", "RZN_APP_BASE", "RZN_NATIVE_APP_BASE"]).is_some()
    {
        return native_attach_endpoint_paths(config);
    }
    if matches!(config.mode, NativeRunMode::Auto | NativeRunMode::Spawn) {
        if let Ok(app_base) = resolve_native_spawn_app_base_dir(config) {
            return vec![resolve_native_spawn_endpoint_path(config, &app_base)];
        }
    }
    Vec::new()
}

fn native_heal_endpoint_paths(config: &NativeHealConfig) -> Vec<PathBuf> {
    let run_config = NativeRunConfig {
        workflow_path: String::new(),
        params: HashMap::new(),
        mode: NativeRunMode::Auto,
        snapshot_mode: SnapshotMode::OnError,
        app_base: config.app_base.clone(),
        endpoint_path: config.endpoint_path.clone(),
        worker_cmd: config.worker_cmd.clone(),
        worker_args: config.worker_args.clone(),
    };

    if let Some(path) = endpoint_path_arg(config.endpoint_path.as_ref()) {
        return vec![path];
    }

    let mut paths = native_attach_endpoint_paths(&run_config);
    if let Ok(app_base) = resolve_native_spawn_app_base_dir(&run_config) {
        paths.push(resolve_native_spawn_endpoint_path(&run_config, &app_base));
    }
    dedupe_paths(paths)
}

async fn query_native_host_path(endpoint_path: &Path) -> Result<Option<String>> {
    let Some(mut client) =
        try_attach_existing_browser_worker_with_preference(endpoint_path, None).await?
    else {
        return Ok(None);
    };
    let health = client.send_request("rzn.worker.health", json!({})).await?;
    Ok(health
        .pointer("/details/native_host_path")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .filter(|v| !v.trim().is_empty()))
}

fn native_self_heal_attempts(mode: NativeRunMode) -> usize {
    if parse_env_bool("RZN_DISABLE_NATIVE_SELF_HEAL").unwrap_or(false) {
        return 0;
    }
    if let Ok(value) = std::env::var("RZN_NATIVE_SELF_HEAL_ATTEMPTS") {
        if let Ok(parsed) = value.trim().parse::<usize>() {
            return parsed;
        }
    }
    match mode {
        NativeRunMode::Attach => 0,
        NativeRunMode::Auto | NativeRunMode::Spawn => DEFAULT_NATIVE_SELF_HEAL_ATTEMPTS,
    }
}

fn is_preflight_native_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<NativeRunPreflightFailure>().is_some()
}

fn print_worker_health_summary(health: &Value) {
    let details = health.get("details").and_then(|v| v.as_object());
    let bridge_connected = details
        .and_then(|d| d.get("bridge_connected"))
        .and_then(|v| v.as_bool());
    let native_host_connected = details
        .and_then(|d| d.get("native_host_connected"))
        .and_then(|v| v.as_bool());
    let native_host_path = details
        .and_then(|d| d.get("native_host_path"))
        .and_then(|v| v.as_str());
    let bridge_socket = details
        .and_then(|d| d.get("browser_bridge_socket_path"))
        .and_then(|v| v.as_str());
    let extension_connected = details
        .and_then(|d| d.get("extension_connected"))
        .and_then(|v| v.as_bool());
    let bridge_host_count = details
        .and_then(|d| d.get("bridge_host_count"))
        .and_then(|v| v.as_u64());
    let browser_session_count = details
        .and_then(|d| d.get("browser_session_count"))
        .and_then(|v| v.as_u64());
    let ping_duration_ms = details
        .and_then(|d| d.get("ping_duration_ms"))
        .and_then(|v| v.as_u64());

    println!(
        "[HEALTH] bridge_connected={:?} native_host_connected={:?} extension_connected={:?} bridge_hosts={:?} browser_sessions={:?} ping_ms={:?} native_host_path={:?} bridge_socket={:?}",
        bridge_connected,
        native_host_connected,
        extension_connected,
        bridge_host_count,
        browser_session_count,
        ping_duration_ms,
        native_host_path,
        bridge_socket
    );

    if let Some(remediation) = details
        .and_then(|d| d.get("remediation"))
        .and_then(|v| v.as_array())
    {
        if !remediation.is_empty() {
            println!("[HEALTH] Remediation:");
            for item in remediation.iter().filter_map(|v| v.as_str()) {
                println!("  - {}", item);
            }
        }
    }
}

fn validate_steps(steps: &[RuntimeStep]) -> Result<()> {
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

struct DesktopBrokerClient {
    reader: Box<dyn AsyncRead + Unpin + Send>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
}

impl DesktopBrokerClient {
    async fn connect(socket_path: &Path, token_path: &Path, profile: &str) -> Result<Self> {
        let token = read_token_file(token_path)?;

        let name = socket_path
            .to_path_buf()
            .to_fs_name::<GenericFilePath>()
            .with_context(|| format!("Invalid socket path {}", socket_path.display()))?;

        let stream = timeout(
            Duration::from_millis(DEFAULT_ATTACH_TIMEOUT_MS),
            LocalSocketStream::connect(name),
        )
        .await
        .context("Desktop broker connect timeout")?
        .with_context(|| {
            format!(
                "Failed to connect to desktop broker {}",
                socket_path.display()
            )
        })?;

        let (reader, mut writer) = tokio::io::split(stream);
        let mut client = Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
        };

        // Broker handshake (matches desktop app native-host expectations).
        let handshake = json!({
            "type": "rzn_broker_handshake",
            "v": 1,
            "token": token,
            "client": {
                "name": "rzn-browser",
                "kind": "cli",
                "pid": std::process::id(),
                "version": env!("CARGO_PKG_VERSION")
            },
            "profile": profile
        });
        let bytes = serde_json::to_vec(&handshake)?;
        send_frame(&mut client.writer, &bytes).await?;

        let resp = timeout(
            Duration::from_millis(DEFAULT_ATTACH_TIMEOUT_MS),
            read_frame(&mut client.reader),
        )
        .await
        .context("Desktop broker handshake timeout")??;
        let value: Value = serde_json::from_slice(&resp)?;
        let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if !ok {
            return Err(anyhow!("Desktop broker handshake failed: {}", value));
        }

        client.initialize_mcp().await?;
        client.ensure_browser_session_tool().await?;

        Ok(client)
    }

    async fn browser_session(
        &mut self,
        cmd: &str,
        payload: Value,
        timeout_ms: u64,
    ) -> Result<Value> {
        let args = json!({
            "cmd": cmd,
            "payload": payload,
            "req_id": Uuid::new_v4().to_string(),
            "timeout_ms": timeout_ms
        });
        self.mcp_tools_call("rzn.browser.session", args, timeout_ms)
            .await
    }

    async fn initialize_mcp(&mut self) -> Result<()> {
        let req_id = format!("init-{}", Uuid::new_v4());
        let request = json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "clientInfo": { "name": "rzn-browser" },
                "capabilities": {}
            }
        });
        let bytes = serde_json::to_vec(&request)?;
        send_frame(&mut self.writer, &bytes).await?;

        let _resp = timeout(
            Duration::from_millis(DEFAULT_REQUEST_TIMEOUT_MS),
            read_matching_jsonrpc_frame(&mut self.reader, &req_id),
        )
        .await
        .context("Desktop broker initialize timeout")??;
        Ok(())
    }

    async fn ensure_browser_session_tool(&mut self) -> Result<()> {
        let req_id = format!("tools-list-{}", Uuid::new_v4());
        let request = json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "method": "tools/list",
            "params": {}
        });
        let bytes = serde_json::to_vec(&request)?;
        send_frame(&mut self.writer, &bytes).await?;

        let resp = timeout(
            Duration::from_millis(DEFAULT_REQUEST_TIMEOUT_MS),
            read_matching_jsonrpc_frame(&mut self.reader, &req_id),
        )
        .await
        .context("Desktop broker tools/list timeout")??;

        let tools = resp
            .pointer("/result/tools")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("Desktop broker tools/list missing result.tools"))?;

        let ok = tools.iter().any(|t| {
            t.get("name")
                .and_then(|v| v.as_str())
                .map(|name| name == "rzn.browser.session")
                .unwrap_or(false)
        });
        if !ok {
            return Err(anyhow!(
                "Desktop broker does not expose rzn.browser.session (profile/tool allowlist mismatch)"
            ));
        }
        Ok(())
    }

    async fn mcp_tools_call(
        &mut self,
        tool_name: &str,
        arguments: Value,
        timeout_ms: u64,
    ) -> Result<Value> {
        let id = format!("req-{}", Uuid::new_v4());
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": tool_name, "arguments": arguments }
        });
        let bytes = serde_json::to_vec(&request)?;
        send_frame(&mut self.writer, &bytes).await?;

        let response = timeout(
            Duration::from_millis(timeout_ms.max(DEFAULT_REQUEST_TIMEOUT_MS)),
            read_matching_jsonrpc_frame(&mut self.reader, &id),
        )
        .await
        .context("Desktop broker tools/call timeout")??;

        if let Some(error) = response.get("error") {
            return Err(anyhow!("Desktop broker JSON-RPC error: {}", error));
        }

        // Prefer structured content when present.
        if let Some(structured) = response
            .pointer("/result/structuredContent")
            .or_else(|| response.pointer("/result/structured_content"))
        {
            return Ok(structured.clone());
        }

        // Fallback: try to parse the first text content as JSON.
        if let Some(text) = response
            .pointer("/result/content")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter()
                    .find_map(|c| c.get("text").and_then(|t| t.as_str()))
            })
        {
            if let Ok(v) = serde_json::from_str::<Value>(text) {
                return Ok(v);
            }
        }

        Ok(response
            .get("result")
            .cloned()
            .unwrap_or_else(|| response.clone()))
    }
}

async fn wait_for_desktop_native_host(
    client: &mut DesktopBrokerClient,
    wait_ms: u64,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms.max(1));
    let mut printed_banner = false;
    loop {
        if tokio::time::Instant::now() > deadline {
            return Err(anyhow!(
                "Timed out waiting for native host/extension connection (desktop broker)"
            ));
        }

        match client.browser_session("ping", json!({}), 2000).await {
            Ok(_) => return Ok(()),
            Err(_) => {
                if !printed_banner {
                    printed_banner = true;
                    println!(
                        "[WAIT] Waiting for native host/extension connection (desktop broker)..."
                    );
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(350)).await;
    }
}

pub(crate) async fn connect_native(config: &NativeRunConfig) -> Result<NativeClient> {
    match config.mode {
        NativeRunMode::Attach => try_attach(config).await,
        NativeRunMode::Spawn => spawn_worker(config).await,
        NativeRunMode::Auto => {
            if let Ok(client) = try_attach(config).await {
                Ok(client)
            } else {
                spawn_worker(config).await
            }
        }
    }
}

fn resolve_desktop_endpoint_path(config: &DesktopRunConfig) -> Result<PathBuf> {
    if let Some(path) = endpoint_path_arg(config.endpoint_path.as_ref()) {
        return Ok(path);
    }
    if let Some(app_base) = app_base_arg_path(config.app_base.as_ref())
        .or_else(|| env_app_base(&["APP_BASE", "RZN_APP_BASE"]))
    {
        return Ok(endpoint_path_for_app_base(&app_base));
    }

    desktop_attach_endpoint_paths()
        .into_iter()
        .next()
        .ok_or_else(|| {
            anyhow!(
                "Desktop broker endpoint not found in common runtime locations. Pass --app-base or --endpoint-path if you need a specific install."
            )
        })
}

fn endpoint_path_arg(value: Option<&String>) -> Option<PathBuf> {
    value
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn app_base_arg_path(value: Option<&String>) -> Option<PathBuf> {
    value
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn env_trimmed(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_app_base(keys: &[&str]) -> Option<PathBuf> {
    keys.iter()
        .find_map(|key| env_trimmed(key).map(PathBuf::from))
}

fn endpoint_path_for_app_base(app_base: &Path) -> PathBuf {
    app_base.join(SECURE_DIRNAME).join(BROKER_ENDPOINT_FILENAME)
}

fn app_base_from_endpoint_path(endpoint_path: &Path) -> Option<PathBuf> {
    if endpoint_path.file_name()?.to_str()? != BROKER_ENDPOINT_FILENAME {
        return None;
    }
    let secure_dir = endpoint_path.parent()?;
    if secure_dir.file_name().and_then(|value| value.to_str()) != Some(SECURE_DIRNAME) {
        return None;
    }
    secure_dir.parent().map(Path::to_path_buf)
}

fn prune_stale_endpoint_path(endpoint_path: &Path) -> Option<BrokerEndpointPruneReport> {
    let app_base = app_base_from_endpoint_path(endpoint_path)?;
    prune_stale_broker_endpoint(&app_base).ok()
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut deduped = Vec::new();
    for path in paths {
        if !deduped.iter().any(|existing| existing == &path) {
            deduped.push(path);
        }
    }
    deduped
}

fn data_roots() -> Vec<PathBuf> {
    dedupe_paths(
        [dirs::data_local_dir(), dirs::data_dir()]
            .into_iter()
            .flatten()
            .collect(),
    )
}

fn runtime_root_candidates() -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(runtime_dir) = env_trimmed("RZN_RUNTIME_DIR").map(PathBuf::from) {
        bases.push(runtime_dir);
    }
    bases.push(default_runtime_dir());
    for root in data_roots() {
        bases.push(root.join("RZN"));
    }
    dedupe_paths(bases)
}

fn sorted_existing_endpoint_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut existing: Vec<(PathBuf, Option<SystemTime>)> = paths
        .into_iter()
        .filter_map(|path| {
            let _ = prune_stale_endpoint_path(&path);
            let metadata = std::fs::metadata(&path).ok()?;
            let modified = metadata.modified().ok();
            Some((path, modified))
        })
        .collect();

    existing.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    existing.into_iter().map(|(path, _)| path).collect()
}

fn desktop_attach_endpoint_paths() -> Vec<PathBuf> {
    let mut bases = runtime_root_candidates();
    for root in data_roots() {
        bases.push(root.join("rzn"));
        bases.push(root.join("rzn_debug"));
    }
    let paths = dedupe_paths(bases)
        .into_iter()
        .map(|base| endpoint_path_for_app_base(&base))
        .collect();
    sorted_existing_endpoint_paths(paths)
}

fn native_attach_endpoint_paths(config: &NativeRunConfig) -> Vec<PathBuf> {
    if let Some(path) = endpoint_path_arg(config.endpoint_path.as_ref()) {
        return vec![path];
    }
    if let Some(app_base) = app_base_arg_path(config.app_base.as_ref())
        .or_else(|| env_app_base(&["APP_BASE", "RZN_APP_BASE", "RZN_NATIVE_APP_BASE"]))
    {
        return vec![endpoint_path_for_app_base(&app_base)];
    }

    let mut bases = runtime_root_candidates();
    for root in data_roots() {
        bases.push(root.join("rzn-browser"));
        bases.push(root.join("rzn"));
        bases.push(root.join("rzn_debug"));
    }
    let paths = dedupe_paths(bases)
        .into_iter()
        .map(|base| endpoint_path_for_app_base(&base))
        .collect();
    sorted_existing_endpoint_paths(paths)
}

fn default_native_spawn_app_base_dir() -> PathBuf {
    if let Some(base) = env_app_base(&["RZN_NATIVE_APP_BASE"]) {
        return base;
    }
    if let Some(root) = data_roots().into_iter().next() {
        return root.join("rzn-browser");
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".rzn-browser");
    }
    PathBuf::from(".rzn-browser")
}

fn resolve_native_spawn_app_base_dir(config: &NativeRunConfig) -> Result<PathBuf> {
    if let Some(app_base) = app_base_arg_path(config.app_base.as_ref()) {
        return Ok(app_base);
    }
    if let Some(app_base) = env_app_base(&["APP_BASE", "RZN_APP_BASE", "RZN_NATIVE_APP_BASE"]) {
        return Ok(app_base);
    }
    if let Some(endpoint_path) = endpoint_path_arg(config.endpoint_path.as_ref()) {
        return app_base_from_endpoint_path(&endpoint_path).ok_or_else(|| {
            anyhow!(
                "--endpoint-path must point to <APP_BASE>/{}/{} when spawn mode is used",
                SECURE_DIRNAME,
                BROKER_ENDPOINT_FILENAME
            )
        });
    }
    Ok(default_native_spawn_app_base_dir())
}

fn resolve_native_spawn_endpoint_path(config: &NativeRunConfig, app_base: &Path) -> PathBuf {
    endpoint_path_arg(config.endpoint_path.as_ref())
        .unwrap_or_else(|| endpoint_path_for_app_base(app_base))
}

fn load_desktop_broker_endpoint(
    endpoint_path: &Path,
    config: &DesktopRunConfig,
) -> Result<(PathBuf, PathBuf, String)> {
    let _ = prune_stale_endpoint_path(endpoint_path);
    let contents = std::fs::read_to_string(endpoint_path)
        .with_context(|| format!("Read endpoint {}", endpoint_path.display()))?;
    let value: Value = serde_json::from_str(&contents)
        .with_context(|| format!("Parse endpoint {}", endpoint_path.display()))?;

    let broker = value
        .get("broker")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow!("Endpoint missing broker section"))?;
    let socket = broker
        .get("socket")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Endpoint broker.socket missing"))?;
    let token_path = broker
        .get("token_path")
        .or_else(|| broker.get("tokenPath"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Endpoint broker.token_path missing"))?;

    let profile = config
        .profile
        .clone()
        .or_else(|| {
            broker
                .get("profile")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "minimal".to_string());

    Ok((PathBuf::from(socket), PathBuf::from(token_path), profile))
}

async fn try_attach(config: &NativeRunConfig) -> Result<NativeClient> {
    let endpoint_paths = native_attach_endpoint_paths(config);
    let preferred_worker_binary = preferred_worker_binary_path(config);
    if endpoint_paths.is_empty() {
        return Err(anyhow!(
            "No native endpoint found in common runtime locations"
        ));
    }

    let mut failures = Vec::new();
    for endpoint_path in endpoint_paths {
        if !existing_worker_matches_preferred(&endpoint_path, preferred_worker_binary.as_deref())? {
            failures.push(format!(
                "{} (existing worker binary does not match preferred worker)",
                endpoint_path.display()
            ));
            continue;
        }
        match try_attach_endpoint(&endpoint_path).await {
            Ok(client) => {
                emit_runtime_status(format!(
                    "[INFO] Attach endpoint: {}",
                    endpoint_path.to_string_lossy()
                ));
                return Ok(client);
            }
            Err(err) => {
                failures.push(format!("{} ({})", endpoint_path.display(), err));
            }
        }
    }

    Err(anyhow!(
        "Failed to attach to any discovered native endpoint: {}",
        failures.join("; ")
    ))
}

async fn try_attach_endpoint(endpoint_path: &Path) -> Result<NativeClient> {
    let endpoint = load_browser_worker_endpoint(endpoint_path)
        .with_context(|| format!("Failed to read endpoint: {}", endpoint_path.display()))?
        .ok_or_else(|| anyhow!("Endpoint does not contain a live browser_worker section"))?;

    match endpoint.transport {
        EndpointTransport::Tcp { host, port } => {
            let addr = format!("{}:{}", host, port);
            let stream = timeout(
                Duration::from_millis(DEFAULT_ATTACH_TIMEOUT_MS),
                TcpStream::connect(addr.clone()),
            )
            .await
            .context("Attach TCP timeout")?
            .with_context(|| format!("Failed to connect TCP {}", addr))?;
            Ok(NativeClient::from_tcp(stream))
        }
        EndpointTransport::Pipe { path, namespaced } => {
            let stream = timeout(
                Duration::from_millis(DEFAULT_ATTACH_TIMEOUT_MS),
                connect_local_socket(&path, namespaced),
            )
            .await
            .context("Attach pipe timeout")??;
            let token_path = endpoint
                .token_path
                .clone()
                .ok_or_else(|| anyhow!("Endpoint missing token_path"))?;
            let mut client = NativeClient::connect_pipe(stream, Path::new(&token_path)).await?;
            if !worker_control_plane_responds(&mut client).await {
                return Err(anyhow!("Attached worker is unresponsive"));
            }
            client.reconnect_pipe = Some(PipeReconnectInfo {
                path,
                namespaced,
                token_path: PathBuf::from(token_path),
            });
            client.reconnect_endpoint_path = Some(endpoint_path.to_path_buf());
            Ok(client)
        }
        EndpointTransport::Stdio { command, args } => Err(anyhow!(
            "Endpoint requires stdio spawn ({} {:?})",
            command,
            args
        )),
    }
}

async fn spawn_worker(config: &NativeRunConfig) -> Result<NativeClient> {
    let app_base = resolve_native_spawn_app_base_dir(config)?;
    let endpoint_path = resolve_native_spawn_endpoint_path(config, &app_base);
    let _spawn_lock = acquire_spawn_lock(&app_base).await?;
    let preferred_worker_binary = preferred_worker_binary_path(config);
    let _ = prune_stale_broker_endpoint(&app_base);

    if let Some(client) = try_attach_existing_browser_worker_with_preference(
        &endpoint_path,
        preferred_worker_binary.as_deref(),
    )
    .await?
    {
        emit_runtime_status(format!(
            "[INFO] Reusing live browser worker at {}",
            endpoint_path.to_string_lossy()
        ));
        return Ok(client);
    }

    if browser_worker_pid_is_live(&endpoint_path)? {
        let deadline =
            tokio::time::Instant::now() + Duration::from_millis(LIVE_WORKER_ATTACH_RETRY_MS);
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if let Some(client) = try_attach_existing_browser_worker_with_preference(
                &endpoint_path,
                preferred_worker_binary.as_deref(),
            )
            .await?
            {
                emit_runtime_status(format!(
                    "[INFO] Reusing live browser worker after retry at {}",
                    endpoint_path.to_string_lossy()
                ));
                return Ok(client);
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            if !browser_worker_pid_is_live(&endpoint_path)? {
                break;
            }
        }

        if browser_worker_pid_is_live(&endpoint_path)? {
            terminate_stale_browser_worker(&endpoint_path).await?;
        }
    }

    let command = config
        .worker_cmd
        .clone()
        .or_else(|| std::env::var("RZN_BROWSER_WORKER_CMD").ok())
        .or_else(resolve_default_worker_command)
        .unwrap_or_else(|| "rzn-browser-worker".to_string());
    let args = if !config.worker_args.is_empty() {
        config.worker_args.clone()
    } else if let Ok(env_args) = std::env::var("RZN_BROWSER_WORKER_ARGS") {
        env_args
            .split_whitespace()
            .filter(|segment| !segment.is_empty())
            .map(|segment| segment.to_string())
            .collect()
    } else {
        Vec::new()
    };

    emit_runtime_status(format!("[INFO] Spawning worker: {} {:?}", command, args));

    let child = Command::new(&command)
        .args(&args)
        .env("RZN_APP_BASE_DIR", app_base.to_string_lossy().to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .with_context(|| format!("Failed to spawn {}", command))?;

    let expected_pid = child.id();
    let endpoint = wait_for_browser_worker_endpoint(&endpoint_path, expected_pid).await?;
    let (transport, token_path) = (endpoint.transport, endpoint.token_path);
    let (path, namespaced) = match transport {
        EndpointTransport::Pipe { path, namespaced } => (path, namespaced),
        other => {
            return Err(anyhow!(
                "Spawned worker returned non-pipe endpoint: {:?}",
                other
            ))
        }
    };
    let stream = timeout(
        Duration::from_millis(DEFAULT_ATTACH_TIMEOUT_MS),
        connect_local_socket(&path, namespaced),
    )
    .await
    .context("Attach pipe timeout")??;

    let token_path = token_path.ok_or_else(|| anyhow!("Endpoint missing token_path"))?;
    let mut client = NativeClient::connect_pipe(stream, Path::new(&token_path)).await?;
    client.child = Some(child);
    client.reconnect_pipe = Some(PipeReconnectInfo {
        path,
        namespaced,
        token_path: PathBuf::from(token_path),
    });
    client.reconnect_endpoint_path = Some(endpoint_path.clone());
    Ok(client)
}

async fn try_attach_existing_browser_worker_with_preference(
    endpoint_path: &Path,
    preferred_worker_binary: Option<&Path>,
) -> Result<Option<NativeClient>> {
    if !existing_worker_matches_preferred(endpoint_path, preferred_worker_binary)? {
        return Ok(None);
    }

    let Some(endpoint) = load_browser_worker_endpoint(endpoint_path)? else {
        return Ok(None);
    };

    let (path, namespaced) = match endpoint.transport {
        EndpointTransport::Pipe { path, namespaced } => (path, namespaced),
        _ => return Ok(None),
    };

    let stream = match timeout(
        Duration::from_millis(DEFAULT_ATTACH_TIMEOUT_MS),
        connect_local_socket(&path, namespaced),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(_)) | Err(_) => return Ok(None),
    };

    let Some(token_path) = endpoint.token_path else {
        return Ok(None);
    };

    match NativeClient::connect_pipe(stream, Path::new(&token_path)).await {
        Ok(mut client) => {
            if !worker_control_plane_responds(&mut client).await {
                return Ok(None);
            }
            client.reconnect_pipe = Some(PipeReconnectInfo {
                path,
                namespaced,
                token_path: PathBuf::from(token_path),
            });
            client.reconnect_endpoint_path = Some(endpoint_path.to_path_buf());
            Ok(Some(client))
        }
        Err(_) => Ok(None),
    }
}

fn load_browser_worker_endpoint(path: &Path) -> Result<Option<EndpointSpec>> {
    let _ = prune_stale_endpoint_path(path);
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let value: Value = serde_json::from_str(&contents)?;
    let Some(obj) = value.as_object() else {
        return Ok(None);
    };

    for key in ["browser_worker", "browser_worker_v1"] {
        if let Some(entry) = obj.get(key) {
            if let Some(pid) = entry.get("pid").and_then(|v| v.as_u64()).map(|v| v as u32) {
                if !pid_looks_alive(pid) {
                    continue;
                }
            }
            if let Some(spec) = parse_endpoint(entry) {
                if !endpoint_spec_is_usable(&spec) {
                    continue;
                }
                return Ok(Some(spec));
            }
        }
    }

    Ok(None)
}

fn endpoint_spec_is_usable(endpoint: &EndpointSpec) -> bool {
    if let Some(token_path) = endpoint.token_path.as_ref() {
        if !Path::new(token_path).exists() {
            return false;
        }
    }

    match &endpoint.transport {
        EndpointTransport::Pipe { path, .. } => Path::new(path).exists(),
        EndpointTransport::Tcp { .. } | EndpointTransport::Stdio { .. } => true,
    }
}

fn browser_worker_pid_is_live(path: &Path) -> Result<bool> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    let value: Value = serde_json::from_str(&contents)?;
    let Some(obj) = value.as_object() else {
        return Ok(false);
    };

    for key in ["browser_worker", "browser_worker_v1"] {
        if let Some(entry) = obj.get(key) {
            if let Some(pid) = entry.get("pid").and_then(|v| v.as_u64()).map(|v| v as u32) {
                return Ok(pid_looks_alive(pid));
            }
        }
    }

    Ok(false)
}

fn browser_worker_pid(path: &Path) -> Result<Option<u32>> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let value: Value = serde_json::from_str(&contents)?;
    let Some(obj) = value.as_object() else {
        return Ok(None);
    };

    for key in ["browser_worker", "browser_worker_v1"] {
        if let Some(entry) = obj.get(key) {
            if let Some(pid) = entry.get("pid").and_then(|v| v.as_u64()).map(|v| v as u32) {
                return Ok(Some(pid));
            }
        }
    }

    Ok(None)
}

fn preferred_worker_binary_path(config: &NativeRunConfig) -> Option<PathBuf> {
    config
        .worker_cmd
        .clone()
        .or_else(|| std::env::var("RZN_BROWSER_WORKER_CMD").ok())
        .or_else(resolve_default_worker_command)
        .map(PathBuf::from)
        .filter(|path| path.exists())
}

fn existing_worker_matches_preferred(
    endpoint_path: &Path,
    preferred_worker_binary: Option<&Path>,
) -> Result<bool> {
    let Some(preferred) = preferred_worker_binary else {
        return Ok(true);
    };
    let Some(pid) = browser_worker_pid(endpoint_path)? else {
        return Ok(true);
    };
    let Some(existing) = pid_executable_path(pid) else {
        return Ok(true);
    };

    let preferred = canonicalize_lossy(preferred);
    let existing = canonicalize_lossy(&existing);
    Ok(existing == preferred)
}

fn canonicalize_lossy(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn pid_looks_alive(pid: u32) -> bool {
    endpoint_pid_is_live(pid)
}

async fn terminate_stale_browser_worker(endpoint_path: &Path) -> Result<()> {
    let Some(pid) = browser_worker_pid(endpoint_path)? else {
        return Ok(());
    };
    if !pid_looks_alive(pid) {
        return Ok(());
    }

    #[cfg(unix)]
    {
        let Ok(pid_i32) = i32::try_from(pid) else {
            return Ok(());
        };
        if pid_i32 > 0 {
            emit_runtime_status(format!(
                "[WARN] Existing browser worker {} is unresponsive; terminating it before spawn",
                pid
            ));
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(pid_i32.to_string())
                .status()
                .await;
            tokio::time::sleep(Duration::from_millis(800)).await;
        }
    }

    #[cfg(not(unix))]
    {
        let _ = pid;
    }

    Ok(())
}

async fn terminate_browser_worker_at_endpoint(endpoint_path: &Path) -> Result<()> {
    terminate_stale_browser_worker(endpoint_path).await
}

fn remove_browser_worker_socket_artifacts(endpoint_path: &Path) -> Result<()> {
    let contents = match std::fs::read_to_string(endpoint_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    let value: Value = serde_json::from_str(&contents)?;

    for key in [
        "browser_worker",
        "browser_worker_v1",
        "browser_bridge",
        "browser_bridge_v1",
    ] {
        if let Some(socket) = value
            .get(key)
            .and_then(|entry| {
                entry
                    .get("socket")
                    .or_else(|| entry.get("socket_path"))
                    .or_else(|| entry.get("pipe_path"))
                    .or_else(|| entry.get("path"))
            })
            .and_then(|socket| socket.as_str())
            .filter(|socket| !socket.trim().is_empty())
        {
            let _ = std::fs::remove_file(socket);
        }
    }

    if let Some(app_base) = app_base_from_endpoint_path(endpoint_path) {
        let _ = std::fs::remove_file(
            app_base
                .join(SECURE_DIRNAME)
                .join(BROWSER_WORKER_SPAWN_LOCK_FILENAME),
        );
    }

    Ok(())
}

fn pid_executable_path(pid: u32) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        return std::fs::read_link(format!("/proc/{}/exe", pid)).ok();
    }

    #[cfg(not(target_os = "linux"))]
    {
        let output = StdCommand::new("lsof")
            .arg("-p")
            .arg(pid.to_string())
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let mut parts = line.split_whitespace();
            let _command = parts.next();
            let _pid = parts.next();
            let _user = parts.next();
            let fd = parts.next();
            if fd != Some("txt") {
                continue;
            }
            let _type = parts.next();
            let _device = parts.next();
            let _size = parts.next();
            let _node = parts.next();
            let path = parts.collect::<Vec<_>>().join(" ");
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
        None
    }
}

fn resolve_default_worker_command() -> Option<String> {
    detect_sibling_worker_binary().or_else(detect_installed_worker_binary)
}

fn detect_sibling_worker_binary() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = if cfg!(windows) {
        dir.join("rzn-browser-worker.exe")
    } else {
        dir.join("rzn-browser-worker")
    };
    candidate
        .exists()
        .then(|| candidate.to_string_lossy().to_string())
}

fn detect_installed_worker_binary() -> Option<String> {
    let file_name = if cfg!(windows) {
        "rzn-browser-worker.exe"
    } else {
        "rzn-browser-worker"
    };

    for root in data_roots() {
        for candidate in [
            root.join("rzn").join("bin").join(file_name),
            root.join("RZN").join("bin").join(file_name),
        ] {
            if candidate.exists() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }

    None
}

async fn wait_for_browser_worker_endpoint(
    endpoint_path: &Path,
    expected_pid: Option<u32>,
) -> Result<EndpointSpec> {
    let deadline =
        tokio::time::Instant::now() + Duration::from_millis(DEFAULT_SPAWN_ENDPOINT_TIMEOUT_MS);
    loop {
        if tokio::time::Instant::now() > deadline {
            return Err(anyhow!(
                "Timed out waiting for browser_worker endpoint at {}",
                endpoint_path.display()
            ));
        }

        if let Ok(contents) = std::fs::read_to_string(endpoint_path) {
            if let Ok(value) = serde_json::from_str::<Value>(&contents) {
                if let Some(obj) = value.as_object() {
                    for key in ["browser_worker", "browser_worker_v1"] {
                        if let Some(entry) = obj.get(key) {
                            // If expected_pid is set, require pid match when present.
                            if let (Some(expected), Some(actual)) = (
                                expected_pid,
                                entry.get("pid").and_then(|v| v.as_u64()).map(|v| v as u32),
                            ) {
                                if expected != actual {
                                    continue;
                                }
                            }
                            if let Some(spec) = parse_endpoint(entry) {
                                return Ok(spec);
                            }
                        }
                    }
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

struct SpawnLockGuard {
    path: PathBuf,
}

impl Drop for SpawnLockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn acquire_spawn_lock(app_base: &Path) -> Result<SpawnLockGuard> {
    let secure_dir = app_base.join("secure");
    std::fs::create_dir_all(&secure_dir)
        .with_context(|| format!("Failed to create secure dir {}", secure_dir.display()))?;
    let path = secure_dir.join(BROWSER_WORKER_SPAWN_LOCK_FILENAME);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(DEFAULT_SPAWN_LOCK_WAIT_MS);

    loop {
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                let _ = writeln!(
                    file,
                    "{{\"pid\":{},\"created_at_ms\":{}}}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                );
                return Ok(SpawnLockGuard { path });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if spawn_lock_is_stale(&path) {
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                if tokio::time::Instant::now() > deadline {
                    return Err(anyhow!(
                        "Timed out waiting for browser worker spawn lock at {}",
                        path.display()
                    ));
                }
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("Failed to create spawn lock {}", path.display()));
            }
        }
    }
}

fn spawn_lock_is_stale(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    let Ok(age) = SystemTime::now().duration_since(modified) else {
        return false;
    };
    age >= StdDuration::from_secs(STALE_SPAWN_LOCK_AGE_SECS)
}

fn parse_endpoint(value: &Value) -> Option<EndpointSpec> {
    if let Some(s) = value.as_str() {
        return parse_endpoint_string(s).map(|transport| EndpointSpec {
            transport,
            token_path: None,
        });
    }

    let obj = value.as_object()?;
    if let Some(v) = obj.get("endpoint") {
        if let Some(parsed) = parse_endpoint(v) {
            return Some(parsed);
        }
    }

    let token_path = obj
        .get("token_path")
        .or_else(|| obj.get("tokenPath"))
        .and_then(|value| value.as_str())
        .map(|text| text.to_string());

    let transport = obj
        .get("transport")
        .or_else(|| obj.get("type"))
        .or_else(|| obj.get("protocol"))
        .or_else(|| obj.get("kind"))
        .and_then(|value| value.as_str())
        .map(|text| text.to_lowercase());

    let command = obj
        .get("command")
        .or_else(|| obj.get("cmd"))
        .and_then(|value| value.as_str())
        .map(|text| text.to_string());
    let args = obj
        .get("args")
        .or_else(|| obj.get("argv"))
        .and_then(|value| value.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str().map(|text| text.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let host = obj
        .get("host")
        .or_else(|| obj.get("hostname"))
        .or_else(|| obj.get("addr"))
        .and_then(|value| value.as_str())
        .map(|text| text.to_string());
    let port = obj
        .get("port")
        .and_then(|value| value.as_u64())
        .map(|port_value| port_value as u16);

    if let Some(url) = obj.get("url").and_then(|value| value.as_str()) {
        if let Some(parsed) = parse_endpoint_string(url) {
            return Some(EndpointSpec {
                transport: parsed,
                token_path,
            });
        }
    }

    let socket_path = obj
        .get("socket")
        .or_else(|| obj.get("socket_path"))
        .or_else(|| obj.get("pipe_path"))
        .or_else(|| obj.get("path"))
        .and_then(|value| value.as_str())
        .map(|text| text.to_string());

    match transport.as_deref() {
        Some("tcp") | Some("mcp") | Some("http") => {
            let host = host?;
            let port = port?;
            return Some(EndpointSpec {
                transport: EndpointTransport::Tcp { host, port },
                token_path,
            });
        }
        Some("pipe") | Some("unix") => {
            let path = socket_path?;
            return Some(EndpointSpec {
                transport: EndpointTransport::Pipe {
                    path,
                    namespaced: false,
                },
                token_path,
            });
        }
        Some("ns") | Some("namespace") | Some("namespaced") => {
            let path = socket_path?;
            return Some(EndpointSpec {
                transport: EndpointTransport::Pipe {
                    path,
                    namespaced: true,
                },
                token_path,
            });
        }
        Some("stdio") => {
            let command = command?;
            return Some(EndpointSpec {
                transport: EndpointTransport::Stdio { command, args },
                token_path,
            });
        }
        _ => {}
    }

    if let (Some(host), Some(port)) = (host, port) {
        return Some(EndpointSpec {
            transport: EndpointTransport::Tcp { host, port },
            token_path,
        });
    }
    if let Some(path) = socket_path {
        return Some(EndpointSpec {
            transport: EndpointTransport::Pipe {
                path,
                namespaced: false,
            },
            token_path,
        });
    }
    if let Some(command) = command {
        return Some(EndpointSpec {
            transport: EndpointTransport::Stdio { command, args },
            token_path,
        });
    }
    None
}

fn parse_endpoint_string(value: &str) -> Option<EndpointTransport> {
    if let Some(stripped) = value.strip_prefix("tcp://") {
        return parse_host_port(stripped);
    }
    if let Some(stripped) = value.strip_prefix("unix://") {
        return Some(EndpointTransport::Pipe {
            path: stripped.to_string(),
            namespaced: false,
        });
    }
    if let Some(stripped) = value.strip_prefix("pipe://") {
        return Some(EndpointTransport::Pipe {
            path: stripped.to_string(),
            namespaced: false,
        });
    }
    if value.contains(':') {
        if let Some(parsed) = parse_host_port(value) {
            return Some(parsed);
        }
    }
    Some(EndpointTransport::Pipe {
        path: value.to_string(),
        namespaced: false,
    })
}

fn parse_host_port(value: &str) -> Option<EndpointTransport> {
    let mut parts = value.rsplitn(2, ':');
    let port_str = parts.next()?;
    let host = parts.next()?.to_string();
    let port: u16 = port_str.parse().ok()?;
    Some(EndpointTransport::Tcp { host, port })
}

async fn connect_local_socket(path: &str, namespaced: bool) -> Result<LocalSocketStream> {
    if namespaced {
        let name = path.to_ns_name::<GenericNamespaced>()?;
        Ok(LocalSocketStream::connect(name).await?)
    } else {
        let name = path.to_fs_name::<GenericFilePath>()?;
        Ok(LocalSocketStream::connect(name).await?)
    }
}

fn load_workflow_for_run(path: &str, params: &HashMap<String, String>) -> Result<LoadedWorkflow> {
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
                "name": manifest.id.replace('/', "_").replace('.', "_"),
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

fn apply_parameters(mut value: Value, params: &HashMap<String, String>) -> Value {
    substitute_value(&mut value, params);
    inject_script_params(&mut value, params);
    value
}

fn substitute_value(value: &mut Value, params: &HashMap<String, String>) {
    match value {
        Value::String(s) => {
            let mut out = s.clone();
            for (key, val) in params {
                out = out.replace(&format!("{{{}}}", key), val);
            }
            *s = out;
        }
        Value::Array(items) => {
            for item in items {
                substitute_value(item, params);
            }
        }
        Value::Object(map) => {
            for value in map.values_mut() {
                substitute_value(value, params);
            }
        }
        _ => {}
    }
}

fn inject_script_params(value: &mut Value, params: &HashMap<String, String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                inject_script_params(item, params);
            }
        }
        Value::Object(map) => {
            let is_script_step = map
                .get("type")
                .and_then(|value| value.as_str())
                .map(|step_type| {
                    matches!(
                        step_type,
                        "execute_javascript" | "eval_main_world" | "eval_isolated_world"
                    )
                })
                .unwrap_or(false);
            if is_script_step {
                let params_value = params
                    .iter()
                    .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                    .collect();
                map.insert("params".to_string(), Value::Object(params_value));
            }
            for value in map.values_mut() {
                inject_script_params(value, params);
            }
        }
        _ => {}
    }
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
            let end = after_dot
                .find(|ch| ch == '.' || ch == '[')
                .unwrap_or(after_dot.len());
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
    let normalized = raw
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .replace(' ', "_");

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

fn parse_env_bool(name: &str) -> Option<bool> {
    let raw = std::env::var(name).ok()?;
    let normalized = raw.trim().to_ascii_lowercase();

    match normalized.as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn emit_runtime_status(message: String) {
    if parse_env_bool("RZN_BROWSER_MCP_STDIO").unwrap_or(false) {
        eprintln!("{}", message);
    } else {
        println!("{}", message);
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

fn with_timeout(mut payload: Value, timeout_ms: u64) -> Value {
    if let Value::Object(map) = &mut payload {
        map.insert(
            "timeout_ms".to_string(),
            Value::Number(serde_json::Number::from(timeout_ms)),
        );
    }
    payload
}

async fn take_supervisor_snapshot(
    config: &supervisor::SupervisorConfig,
    session_id: Option<&str>,
) -> Result<()> {
    let response = supervisor::call(
        config.clone(),
        "browser.snapshot",
        with_session(session_id, json!({})),
    )
    .await?;
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
    if let Some(hash) = hash {
        println!("[SNAPSHOT] dom_hash={}", hash);
    } else {
        println!("[SNAPSHOT] ok");
    }
    Ok(())
}

async fn take_snapshot(client: &mut NativeClient, session_id: Option<&str>) -> Result<()> {
    let response = client
        .send_request("browser.snapshot", with_session(session_id, json!({})))
        .await?;
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
    if let Some(hash) = hash {
        println!("[SNAPSHOT] dom_hash={}", hash);
    } else {
        println!("[SNAPSHOT] ok");
    }
    Ok(())
}

fn response_success(response: &Value) -> bool {
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

fn response_error_message<'a>(response: &'a Value) -> Option<&'a str> {
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

fn debug_raw_step_response_enabled() -> bool {
    std::env::var("RZN_DEBUG_NATIVE_STEP_RESPONSES")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn log_step_response(step_id: &str, step_type: &str, response: &Value) {
    if response_success(response) {
        println!("[OK] {} ({})", step_id, step_type);
    } else {
        let err = response_error_message(response).unwrap_or("unknown error");
        println!("[ERR] {} ({}) {}", step_id, step_type, err);
    }

    if debug_raw_step_response_enabled() {
        if let Ok(pretty) = serde_json::to_string_pretty(response) {
            println!("   raw_response: {}", pretty.replace('\n', "\n   "));
        } else {
            println!("   raw_response: {}", response);
        }
    }

    if let Some(result) = response.get("result") {
        summarize_result(result);
    } else if let Some(result) = response.get("data") {
        summarize_result(result);
    }
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

fn summarize_result(value: &Value) {
    if let Some(items) = value.as_array() {
        println!("   result: {} items", items.len());
        let mut printed = 0usize;
        for item in items.iter() {
            if let Some(obj) = item.as_object() {
                if let Some(title) = obj.get("title").and_then(|value| value.as_str()) {
                    println!("   - title: {}", title);
                    printed += 1;
                } else if let Some(text) = obj.get("text").and_then(|value| value.as_str()) {
                    println!("   - text: {}", text);
                    printed += 1;
                }
            }
            if printed >= 5 {
                break;
            }
        }
    } else if let Some(obj) = value.as_object() {
        if let Some(inner) = obj.get("result") {
            if !matches!(inner, Value::Null | Value::Bool(_)) {
                summarize_result(inner);
                return;
            }
        }
        if let Some(items) = obj.get("items").and_then(|value| value.as_array()) {
            println!("   result.items: {}", items.len());
        } else if let Some(url) = obj.get("url").and_then(|value| value.as_str()) {
            println!("   result.url: {}", url);
        } else {
            let mut parts: Vec<String> = Vec::new();
            for key in [
                "clicked",
                "opened",
                "found",
                "success",
                "url",
                "selector",
                "target_text",
                "target_href",
                "force_same_tab",
                "count",
                "text",
                "value",
                "href",
                "approval_mode",
                "continued_by",
                "stop_workflow",
                "stop_reason",
                "notification_sent",
            ] {
                if let Some(v) = obj.get(key) {
                    let rendered = match v {
                        Value::String(s) => s.clone(),
                        Value::Bool(b) => b.to_string(),
                        Value::Number(n) => n.to_string(),
                        _ => continue,
                    };
                    parts.push(format!("{}={}", key, rendered));
                }
            }

            if !parts.is_empty() {
                println!("   result: {}", parts.join(" "));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        app_base_from_endpoint_path, apply_parameters, endpoint_path_for_app_base,
        extract_payload_for_output, health_indicates_stale_worker_handshake,
        is_transient_step_error, keep_browser_worker_on_exit, load_browser_worker_endpoint,
        load_runtime_context_for_workflow, load_workflow_for_run, load_workflow_value,
        native_attach_endpoint_paths, native_self_heal_attempts, native_self_heal_endpoint_paths,
        record_step_output, resolve_native_spawn_app_base_dir, response_success,
        restart_native_host_enabled, select_workflow_output, should_handle_step_locally,
        step_execution_payload, NativeRunConfig, NativeRunMode, RuntimeStep, SnapshotMode,
        WorkflowRuntimeContext,
    };
    use serde_json::json;
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use uuid::Uuid;

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
        assert!(!is_transient_step_error("Selector not found"));
    }

    #[test]
    fn load_browser_worker_endpoint_ignores_non_worker_entries() {
        let path = unique_temp_path("native-run-endpoint.json");
        fs::write(
            &path,
            r#"{
              "browser_bridge": {
                "transport": "pipe",
                "path": "/tmp/bridge.sock",
                "token_path": "/tmp/bridge.token"
              }
            }"#,
        )
        .unwrap();

        let endpoint = load_browser_worker_endpoint(&path).unwrap();
        assert!(endpoint.is_none());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn load_browser_worker_endpoint_prefers_worker_entry() {
        let path = unique_temp_path("native-run-endpoint.json");
        let live_pid = std::process::id();
        let worker_socket = unique_temp_path("worker.sock");
        let worker_token = unique_temp_path("worker.token");
        write_marker(&worker_socket);
        write_marker(&worker_token);
        let contents = serde_json::json!({
            "browser_bridge": {
                "transport": "pipe",
                "path": "/tmp/bridge.sock",
                "token_path": "/tmp/bridge.token"
            },
            "browser_worker": {
                "transport": "pipe",
                "path": worker_socket,
                "token_path": worker_token,
                "pid": live_pid
            }
        });
        fs::write(&path, serde_json::to_string(&contents).unwrap()).unwrap();

        let endpoint = load_browser_worker_endpoint(&path).unwrap().unwrap();
        match endpoint.transport {
            super::EndpointTransport::Pipe { path, namespaced } => {
                assert!(path.ends_with("worker.sock"));
                assert!(!namespaced);
            }
            other => panic!("unexpected endpoint transport: {:?}", other),
        }
        assert!(endpoint
            .token_path
            .as_deref()
            .is_some_and(|path| path.ends_with("worker.token")));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn load_browser_worker_endpoint_skips_dead_worker_pid() {
        let path = unique_temp_path("native-run-endpoint-dead.json");
        fs::write(
            &path,
            r#"{
              "browser_worker": {
                "transport": "pipe",
                "path": "/tmp/dead-worker.sock",
                "token_path": "/tmp/dead-worker.token",
                "pid": 4294967295
              }
            }"#,
        )
        .unwrap();

        let endpoint = load_browser_worker_endpoint(&path).unwrap();
        assert!(endpoint.is_none());

        let _ = fs::remove_file(&path);
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
            }"#,
        )
        .unwrap();

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
    fn keep_browser_worker_defaults_to_true_for_shared_runtime() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var("RZN_KEEP_BROWSER_WORKER");
        std::env::remove_var("RZN_KILL_BROWSER_WORKER_ON_EXIT");
        assert!(keep_browser_worker_on_exit());
    }

    #[test]
    fn kill_browser_worker_flag_can_force_old_shutdown_behavior() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var("RZN_KEEP_BROWSER_WORKER");
        std::env::set_var("RZN_KILL_BROWSER_WORKER_ON_EXIT", "1");
        assert!(!keep_browser_worker_on_exit());
        std::env::remove_var("RZN_KILL_BROWSER_WORKER_ON_EXIT");
    }

    #[test]
    fn native_host_restart_defaults_on_for_distributed_runs() {
        let _guard = env_lock().lock().unwrap();
        clear_env(&["RZN_RESTART_NATIVE_HOST", "RZN_DISABLE_NATIVE_HOST_RESTART"]);

        assert!(restart_native_host_enabled());

        std::env::set_var("RZN_DISABLE_NATIVE_HOST_RESTART", "1");
        assert!(!restart_native_host_enabled());
        std::env::remove_var("RZN_DISABLE_NATIVE_HOST_RESTART");

        std::env::set_var("RZN_RESTART_NATIVE_HOST", "0");
        assert!(!restart_native_host_enabled());
        std::env::remove_var("RZN_RESTART_NATIVE_HOST");
    }

    #[test]
    fn native_self_heal_retries_only_auto_and_spawn_by_default() {
        let _guard = env_lock().lock().unwrap();
        clear_env(&[
            "RZN_DISABLE_NATIVE_SELF_HEAL",
            "RZN_NATIVE_SELF_HEAL_ATTEMPTS",
        ]);

        assert_eq!(native_self_heal_attempts(NativeRunMode::Auto), 1);
        assert_eq!(native_self_heal_attempts(NativeRunMode::Spawn), 1);
        assert_eq!(native_self_heal_attempts(NativeRunMode::Attach), 0);

        std::env::set_var("RZN_NATIVE_SELF_HEAL_ATTEMPTS", "3");
        assert_eq!(native_self_heal_attempts(NativeRunMode::Attach), 3);
        std::env::remove_var("RZN_NATIVE_SELF_HEAL_ATTEMPTS");
    }

    #[test]
    fn native_self_heal_scopes_default_to_standalone_runtime() {
        let _guard = env_lock().lock().unwrap();
        clear_env(&["APP_BASE", "RZN_APP_BASE", "RZN_NATIVE_APP_BASE"]);

        let config = sample_native_config();
        let paths = native_self_heal_endpoint_paths(&config);

        assert_eq!(paths.len(), 1);
        assert!(paths[0].to_string_lossy().contains("rzn-browser"));
    }

    #[test]
    fn native_self_heal_respects_explicit_endpoint_path() {
        let _guard = env_lock().lock().unwrap();
        clear_env(&["APP_BASE", "RZN_APP_BASE", "RZN_NATIVE_APP_BASE"]);

        let endpoint_path = unique_temp_path("explicit-endpoint.json");
        let mut config = sample_native_config();
        config.endpoint_path = Some(endpoint_path.to_string_lossy().to_string());

        assert_eq!(
            native_self_heal_endpoint_paths(&config),
            vec![endpoint_path]
        );
    }

    #[test]
    fn stale_worker_health_requires_accepted_bridge_without_live_extension() {
        let health = json!({
            "details": {
                "bridge_connected": true,
                "native_host_connected": false,
                "extension_connected": false,
                "bridge_host_count": 2
            }
        });
        assert!(health_indicates_stale_worker_handshake(&health));

        let healthy = json!({
            "details": {
                "bridge_connected": true,
                "native_host_connected": true,
                "extension_connected": true,
                "bridge_host_count": 1
            }
        });
        assert!(!health_indicates_stale_worker_handshake(&healthy));
    }

    #[test]
    fn spawn_mode_defaults_to_internal_standalone_app_base() {
        let _guard = env_lock().lock().unwrap();
        clear_env(&["APP_BASE", "RZN_APP_BASE", "RZN_NATIVE_APP_BASE"]);

        let config = sample_native_config();
        let app_base = resolve_native_spawn_app_base_dir(&config).unwrap();

        assert!(app_base.ends_with("rzn-browser"));
    }

    #[test]
    fn spawn_mode_can_derive_app_base_from_endpoint_path() {
        let _guard = env_lock().lock().unwrap();
        clear_env(&["APP_BASE", "RZN_APP_BASE", "RZN_NATIVE_APP_BASE"]);

        let app_base = unique_temp_path("native-app-base");
        let endpoint_path = endpoint_path_for_app_base(&app_base);
        let mut config = sample_native_config();
        config.endpoint_path = Some(endpoint_path.to_string_lossy().to_string());

        let resolved = resolve_native_spawn_app_base_dir(&config).unwrap();
        assert_eq!(resolved, app_base);
        assert_eq!(
            app_base_from_endpoint_path(&endpoint_path).unwrap(),
            app_base
        );
    }

    #[test]
    fn attach_mode_respects_explicit_app_base_without_env() {
        let _guard = env_lock().lock().unwrap();
        clear_env(&["APP_BASE", "RZN_APP_BASE", "RZN_NATIVE_APP_BASE"]);

        let app_base = unique_temp_path("native-attach-base");
        let mut config = sample_native_config();
        config.app_base = Some(app_base.to_string_lossy().to_string());

        let paths = native_attach_endpoint_paths(&config);
        assert_eq!(paths, vec![endpoint_path_for_app_base(&app_base)]);
    }

    fn unique_temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{}-{}", Uuid::new_v4(), name))
    }

    fn write_marker(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, "ok\n").unwrap();
    }

    fn sample_native_config() -> NativeRunConfig {
        NativeRunConfig {
            workflow_path: "workflows/google/google-search.json".to_string(),
            params: HashMap::new(),
            mode: NativeRunMode::Auto,
            snapshot_mode: SnapshotMode::OnError,
            app_base: None,
            endpoint_path: None,
            worker_cmd: None,
            worker_args: Vec::new(),
        }
    }

    fn clear_env(keys: &[&str]) {
        for key in keys {
            std::env::remove_var(key);
        }
    }

    fn env_lock() -> &'static Mutex<()> {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }
}

pub(crate) struct NativeClient {
    reader: Box<dyn AsyncRead + Unpin + Send>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
    child: Option<Child>,
    reconnect_pipe: Option<PipeReconnectInfo>,
    reconnect_endpoint_path: Option<PathBuf>,
}

#[derive(Clone)]
struct PipeReconnectInfo {
    path: String,
    namespaced: bool,
    token_path: PathBuf,
}

impl NativeClient {
    fn from_tcp(stream: TcpStream) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
            child: None,
            reconnect_pipe: None,
            reconnect_endpoint_path: None,
        }
    }

    fn from_pipe(stream: LocalSocketStream) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
            child: None,
            reconnect_pipe: None,
            reconnect_endpoint_path: None,
        }
    }

    async fn connect_pipe(stream: LocalSocketStream, token_path: &Path) -> Result<Self> {
        let mut client = Self::from_pipe(stream);
        client.handshake_worker(token_path).await?;
        client.initialize_mcp().await?;
        Ok(client)
    }

    async fn handshake_worker(&mut self, token_path: &Path) -> Result<()> {
        let token = read_token_file(token_path)?;
        let handshake = json!({
            "type": "rzn_browser_worker_handshake",
            "v": 1,
            "token": token,
            "client": {
                "name": "rzn-browser",
                "pid": std::process::id()
            }
        });
        let bytes = serde_json::to_vec(&handshake)?;
        send_frame(&mut self.writer, &bytes).await?;

        let resp = timeout(
            Duration::from_millis(DEFAULT_ATTACH_TIMEOUT_MS),
            read_frame(&mut self.reader),
        )
        .await
        .context("Handshake timeout")??;
        let value: Value = serde_json::from_slice(&resp)?;
        let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if !ok {
            return Err(anyhow!("Handshake failed: {}", value));
        }
        Ok(())
    }

    async fn initialize_mcp(&mut self) -> Result<()> {
        let req_id = format!("init-{}", Uuid::new_v4());
        let request = json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "clientInfo": { "name": "rzn-browser" },
                "capabilities": {}
            }
        });
        let bytes = serde_json::to_vec(&request)?;
        send_frame(&mut self.writer, &bytes).await?;

        let _resp = timeout(
            Duration::from_millis(DEFAULT_REQUEST_TIMEOUT_MS),
            read_matching_jsonrpc_frame(&mut self.reader, &req_id),
        )
        .await
        .context("Initialize timeout")??;
        Ok(())
    }

    pub(crate) async fn send_request(&mut self, cmd: &str, payload: Value) -> Result<Value> {
        self.send_request_with_timeout(cmd, payload, DEFAULT_REQUEST_TIMEOUT_MS)
            .await
    }

    pub(crate) async fn send_request_with_timeout(
        &mut self,
        cmd: &str,
        payload: Value,
        timeout_ms: u64,
    ) -> Result<Value> {
        let result = self
            .send_tool_call_result_with_timeout(cmd, payload, timeout_ms)
            .await?;

        // Return the structured tool payload (what rzn-browser-worker puts in structuredContent).
        if let Some(structured) = result
            .pointer("/structuredContent")
            .or_else(|| result.pointer("/structured_content"))
        {
            return Ok(structured.clone());
        }

        // Fallback: try to parse the first text content as JSON.
        if let Some(text) = result
            .get("content")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter()
                    .find_map(|c| c.get("text").and_then(|t| t.as_str()))
            })
        {
            if let Ok(v) = serde_json::from_str::<Value>(text) {
                return Ok(v);
            }
        }

        Ok(result)
    }

    async fn send_tool_call_result_with_timeout(
        &mut self,
        cmd: &str,
        payload: Value,
        timeout_ms: u64,
    ) -> Result<Value> {
        let req_id = format!("req-{}", Uuid::new_v4());
        let message = json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "method": "tools/call",
            "params": { "name": cmd, "arguments": payload }
        });
        let bytes = serde_json::to_vec(&message)?;
        if let Err(err) = send_frame(&mut self.writer, &bytes).await {
            if is_retryable_transport_error(&err) && self.try_reconnect().await? {
                send_frame(&mut self.writer, &bytes).await?;
            } else {
                return Err(err);
            }
        }

        let response = timeout(
            Duration::from_millis(timeout_ms.max(DEFAULT_REQUEST_TIMEOUT_MS)),
            read_matching_jsonrpc_frame(&mut self.reader, &req_id),
        )
        .await
        .context("Request timeout")??;

        if let Some(error) = response.get("error") {
            return Err(anyhow!("MCP error: {}", error));
        }

        if let Some(result) = response.get("result") {
            Ok(result.clone())
        } else {
            Ok(response)
        }
    }

    pub(crate) async fn shutdown(&mut self) {
        if let Some(child) = &mut self.child {
            let keep = keep_browser_worker_on_exit();
            if keep {
                return;
            }
            let _ = child.kill().await;
        }
    }

    async fn try_reconnect(&mut self) -> Result<bool> {
        if let Some(endpoint_path) = self.reconnect_endpoint_path.clone() {
            if let Some(endpoint) = load_browser_worker_endpoint(&endpoint_path)? {
                if let EndpointTransport::Pipe { path, namespaced } = endpoint.transport {
                    if let Some(token_path) = endpoint.token_path {
                        let stream = match timeout(
                            Duration::from_millis(DEFAULT_ATTACH_TIMEOUT_MS),
                            connect_local_socket(&path, namespaced),
                        )
                        .await
                        {
                            Ok(Ok(stream)) => stream,
                            Ok(Err(_)) | Err(_) => return self.try_reconnect_pipe_fallback().await,
                        };

                        let mut client =
                            NativeClient::connect_pipe(stream, Path::new(&token_path)).await?;
                        client.child = self.child.take();
                        client.reconnect_pipe = Some(PipeReconnectInfo {
                            path,
                            namespaced,
                            token_path: PathBuf::from(token_path),
                        });
                        client.reconnect_endpoint_path = Some(endpoint_path);
                        *self = client;
                        return Ok(true);
                    }
                }
            }
        }

        self.try_reconnect_pipe_fallback().await
    }

    async fn try_reconnect_pipe_fallback(&mut self) -> Result<bool> {
        let Some(info) = self.reconnect_pipe.clone() else {
            return Ok(false);
        };

        let stream = match timeout(
            Duration::from_millis(DEFAULT_ATTACH_TIMEOUT_MS),
            connect_local_socket(&info.path, info.namespaced),
        )
        .await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(_)) | Err(_) => return Ok(false),
        };

        let mut client = NativeClient::connect_pipe(stream, &info.token_path).await?;
        client.child = self.child.take();
        client.reconnect_pipe = Some(info);
        client.reconnect_endpoint_path = self.reconnect_endpoint_path.clone();
        *self = client;
        Ok(true)
    }
}

async fn worker_control_plane_responds(client: &mut NativeClient) -> bool {
    match client
        .send_request_with_timeout(
            "rzn.worker.health",
            json!({}),
            WORKER_HEALTHCHECK_TIMEOUT_MS,
        )
        .await
    {
        Ok(_) => true,
        Err(err) => {
            emit_runtime_status(format!(
                "[WARN] Existing browser worker control plane is unresponsive: {}",
                err
            ));
            false
        }
    }
}

fn keep_browser_worker_on_exit() -> bool {
    if let Ok(value) = std::env::var("RZN_KEEP_BROWSER_WORKER") {
        return value == "1" || value.eq_ignore_ascii_case("true");
    }
    if let Ok(value) = std::env::var("RZN_KILL_BROWSER_WORKER_ON_EXIT") {
        return !(value == "1" || value.eq_ignore_ascii_case("true"));
    }
    true
}

fn is_retryable_transport_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    msg.contains("Broken pipe")
        || msg.contains("Connection reset")
        || msg.contains("connection reset")
        || msg.contains("Not connected")
        || msg.contains("not connected")
}

async fn send_frame<W: AsyncWrite + Unpin>(writer: &mut W, bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_FRAME_SIZE {
        return Err(anyhow!("Frame too large: {}", bytes.len()));
    }
    let len = bytes.len() as u32;
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(bytes).await?;
    writer.flush().await?;
    Ok(())
}

fn read_token_file(path: &Path) -> Result<String> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Read token file {}", path.display()))?;
    let token = content.trim().to_string();
    if token.is_empty() {
        return Err(anyhow!("Token file is empty: {}", path.display()));
    }
    Ok(token)
}

async fn read_matching_jsonrpc_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    req_id: &str,
) -> Result<Value> {
    loop {
        let bytes = read_frame(reader).await?;
        let value: Value = serde_json::from_slice(&bytes)?;
        let resp_id = value.get("id").map(|value| match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            other => other.to_string(),
        });
        if resp_id.as_deref() == Some(req_id) {
            return Ok(value);
        }
    }
}

async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    if len == 0 {
        return Err(anyhow!("Empty frame"));
    }
    if len > MAX_FRAME_SIZE {
        return Err(anyhow!("Frame exceeds limit: {}", len));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}
