use crate::supervisor;
use crate::workflow_failure_report::{build_failure_context, WorkflowRunFailure};
use crate::workflow_params::{apply_parameters, inject_script_params};
use anyhow::{anyhow, Context, Result};
use rzn_contracts::v2::{
    validate_manifest_value, ParamDefV2, ParamKindV2, RunStatusV2, StepV2, WorkflowManifestV2,
    RUN_RESULT_VERSION,
};
use rzn_core::dsl;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use tokio::time::Duration;
use uuid::Uuid;

const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 30000;
const DEFAULT_NATIVE_STEP_RPC_GRACE_MS: u64 = 5000;

fn should_handle_step_locally(step_type: &str) -> bool {
    step_type == "wait_for_timeout"
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotMode {
    None,
    AfterStep,
    OnError,
}

#[derive(Debug, Clone)]
pub struct SupervisorRunConfig {
    pub workflow_path: String,
    pub params: HashMap<String, String>,
    pub snapshot_mode: SnapshotMode,
    pub app_base: Option<String>,
    pub browser_target: Option<Value>,
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

pub async fn run_supervisor_workflow(config: SupervisorRunConfig) -> Result<Option<Value>> {
    let workflow = load_workflow_for_run(&config.workflow_path, &config.params)?;
    validate_steps(&workflow.steps)?;

    let supervisor_config = supervisor::SupervisorConfig {
        app_base: config.app_base.as_ref().map(PathBuf::from),
    };
    supervisor::ensure_running(supervisor_config.clone()).await?;
    ensure_supervisor_run_ready(
        |method, params| {
            let supervisor_config = supervisor_config.clone();
            async move { supervisor::call(supervisor_config, method, params).await }
        },
        config.browser_target.as_ref(),
    )
    .await?;

    let mut session_id: Option<String> = None;
    let mut final_payload: Option<Value> = None;
    let mut step_outputs: HashMap<String, Value> = HashMap::new();

    let result: Result<()> = async {
        let session_resp = supervisor::call(
            supervisor_config.clone(),
            "browser.session_open",
            with_browser_target(json!({}), config.browser_target.as_ref()),
        )
        .await?;
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
            let payload = with_browser_target(payload, config.browser_target.as_ref());
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

async fn ensure_supervisor_run_ready<F, Fut>(
    mut supervisor_call: F,
    browser_target: Option<&Value>,
) -> Result<Value>
where
    F: FnMut(&'static str, Value) -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    let params = with_browser_target(json!({}), browser_target);
    let readiness = supervisor_call("runtime.ensure_ready", params.clone()).await?;
    if readiness_ok(&readiness) {
        return Ok(readiness);
    }

    if should_auto_heal_run_readiness(&readiness) {
        let healed = supervisor_call("runtime.heal", params).await?;
        if readiness_ok(&healed) {
            return Ok(healed);
        }
        return Err(anyhow!("{}", readiness_failure_message(&healed)));
    }

    Err(anyhow!("{}", readiness_failure_message(&readiness)))
}

fn readiness_ok(value: &Value) -> bool {
    value.get("ok").and_then(|value| value.as_bool()) == Some(true)
        || value.get("ready").and_then(|value| value.as_bool()) == Some(true)
}

fn should_auto_heal_run_readiness(readiness: &Value) -> bool {
    if readiness_ok(readiness) {
        return false;
    }

    let bridge_connected = readiness
        .pointer("/native_host_bridge/connected")
        .and_then(|value| value.as_bool())
        == Some(true);
    if !bridge_connected {
        return false;
    }

    let bridge_responsive = readiness
        .pointer("/native_host_bridge/responsive")
        .and_then(|value| value.as_bool());
    let probe_checked = readiness.pointer("/native_host_bridge/probe").is_some();
    (bridge_responsive == Some(false) || probe_checked)
        && !readiness_reports_stale_extension_contract(readiness)
        && !readiness_reports_browser_target_problem(readiness)
}

fn readiness_reports_browser_target_problem(readiness: &Value) -> bool {
    let cause = readiness
        .pointer("/diagnostic/cause")
        .and_then(Value::as_str)
        .or_else(|| {
            readiness
                .pointer("/readiness/diagnostic/cause")
                .and_then(Value::as_str)
        });
    if matches!(
        cause,
        Some("browser_target_unresolved" | "browser_target_mismatch")
    ) {
        return true;
    }

    if readiness
        .pointer("/native_host_bridge/probe/target_resolution_error/error_code")
        .and_then(Value::as_str)
        .is_some()
        || readiness
            .pointer("/readiness/native_host_bridge/probe/target_resolution_error/error_code")
            .and_then(Value::as_str)
            .is_some()
    {
        return true;
    }

    readiness
        .pointer("/native_host_bridge/probe/target_match_ok")
        .and_then(Value::as_bool)
        == Some(false)
        || readiness
            .pointer("/readiness/native_host_bridge/probe/target_match_ok")
            .and_then(Value::as_bool)
            == Some(false)
}

fn readiness_reports_stale_extension_contract(readiness: &Value) -> bool {
    if readiness
        .pointer("/diagnostic/cause")
        .and_then(|value| value.as_str())
        == Some("stale_extension_bundle")
    {
        return true;
    }

    if readiness
        .pointer("/native_host_bridge/probe/bridge_contract_version_ok")
        .and_then(|value| value.as_bool())
        == Some(false)
    {
        return true;
    }

    let keepalive_missing = readiness
        .pointer("/native_host_bridge/probe/required_capabilities/content_keepalive_port")
        .and_then(|value| value.as_bool())
        == Some(false);
    if keepalive_missing {
        return true;
    }

    readiness
        .pointer("/native_host_bridge/probe/error")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .contains("content_keepalive_port")
}

fn readiness_failure_message(value: &Value) -> String {
    value
        .pointer("/diagnostic/action_text")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            value
                .pointer("/readiness/diagnostic/action_text")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| {
            value
                .pointer("/diagnostic/message")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| {
            value
                .pointer("/readiness/diagnostic/message")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| {
            value
                .get("error")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| {
            value
                .pointer("/readiness/error")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| {
            value
                .pointer("/readiness/native_host_bridge/probe/error")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| {
            value
                .pointer("/native_host_bridge/probe/error")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or("supervisor runtime is not ready")
        .to_string()
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

fn with_browser_target(mut payload: Value, browser_target: Option<&Value>) -> Value {
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

fn response_error_message(response: &Value) -> Option<&str> {
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
        apply_parameters, ensure_supervisor_run_ready, extract_payload_for_output,
        is_transient_step_error, load_runtime_context_for_workflow, load_workflow_for_run,
        load_workflow_value, record_step_output, response_success, select_workflow_output,
        should_auto_heal_run_readiness, should_handle_step_locally, step_execution_payload,
        with_browser_target, RuntimeStep, WorkflowRuntimeContext,
    };
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::collections::VecDeque;
    use std::fs;
    use std::future;
    use std::path::PathBuf;
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
        assert!(is_transient_step_error("native_host_disconnected"));
        assert!(is_transient_step_error(
            "Native-host bridge response channel closed"
        ));
        assert!(is_transient_step_error(
            "Native-host extension bridge timeout after 40000ms"
        ));
        assert!(!is_transient_step_error("Selector not found"));
    }

    #[tokio::test]
    async fn run_preflight_heals_connected_unresponsive_bridge() {
        let mut calls = Vec::new();
        let mut responses = VecDeque::from([
            Ok(json!({
                "ok": false,
                "ready": false,
                "native_host_bridge": {
                    "connected": true,
                    "responsive": false,
                    "probe": {
                        "ok": false,
                        "error": "Native-host extension bridge timeout after 1500ms"
                    }
                },
                "error": "initial readiness failed"
            })),
            Ok(json!({
                "ok": true,
                "ready": true,
                "readiness": {
                    "ok": true,
                    "ready": true
                }
            })),
        ]);

        let result = ensure_supervisor_run_ready(
            |method, params| {
                calls.push((method, params));
                future::ready(responses.pop_front().expect("queued response"))
            },
            None,
        )
        .await
        .expect("heal success should allow run preflight");

        assert_eq!(result.get("ok"), Some(&json!(true)));
        assert_eq!(
            calls.iter().map(|(method, _)| *method).collect::<Vec<_>>(),
            vec!["runtime.ensure_ready", "runtime.heal"]
        );
    }

    #[tokio::test]
    async fn run_preflight_reports_post_heal_diagnostic() {
        let mut calls = Vec::new();
        let mut responses = VecDeque::from([
            Ok(json!({
                "ok": false,
                "ready": false,
                "native_host_bridge": {
                    "connected": true,
                    "responsive": false,
                    "probe": {
                        "ok": false,
                        "error": "initial timeout"
                    }
                },
                "error": "initial generic reload text"
            })),
            Ok(json!({
                "ok": false,
                "ready": false,
                "readiness": {
                    "ok": false,
                    "ready": false,
                    "diagnostic": {
                        "cause": "transport_timeout",
                        "action_text": "Run `rzn-browser heal --json`, then retry."
                    },
                    "error": "post-heal bridge still timed out"
                }
            })),
        ]);

        let error = ensure_supervisor_run_ready(
            |method, params| {
                calls.push((method, params));
                future::ready(responses.pop_front().expect("queued response"))
            },
            None,
        )
        .await
        .expect_err("failed heal should fail run preflight");

        assert!(error
            .to_string()
            .contains("Run `rzn-browser heal --json`, then retry."));
        assert!(!error.to_string().contains("initial generic reload text"));
        assert_eq!(
            calls.iter().map(|(method, _)| *method).collect::<Vec<_>>(),
            vec!["runtime.ensure_ready", "runtime.heal"]
        );
    }

    #[tokio::test]
    async fn run_preflight_passes_browser_target_to_readiness_and_heal() {
        let browser_target = json!({ "browser": "edge" });
        let mut calls = Vec::new();
        let mut responses = VecDeque::from([
            Ok(json!({
                "ok": false,
                "ready": false,
                "native_host_bridge": {
                    "connected": true,
                    "responsive": false,
                    "probe": { "ok": false, "error": "timeout" }
                }
            })),
            Ok(json!({ "ok": true, "ready": true })),
        ]);

        ensure_supervisor_run_ready(
            |method, params| {
                calls.push((method, params));
                future::ready(responses.pop_front().expect("queued response"))
            },
            Some(&browser_target),
        )
        .await
        .expect("targeted readiness should pass after heal");

        assert_eq!(
            calls.iter().map(|(method, _)| *method).collect::<Vec<_>>(),
            vec!["runtime.ensure_ready", "runtime.heal"]
        );
        assert!(calls.iter().all(|(_, params)| {
            params
                .pointer("/browser_target/browser")
                .and_then(Value::as_str)
                == Some("edge")
        }));
    }

    #[tokio::test]
    async fn run_preflight_does_not_heal_ready_or_stale_bundle_states() {
        let mut ready_calls = Vec::new();
        let ready = ensure_supervisor_run_ready(
            |method, params| {
                ready_calls.push((method, params));
                future::ready(Ok(json!({ "ok": true, "ready": true })))
            },
            None,
        )
        .await
        .expect("ready state should pass");
        assert_eq!(ready.get("ready"), Some(&json!(true)));
        assert_eq!(ready_calls.len(), 1);

        let stale = json!({
            "ok": false,
            "ready": false,
            "native_host_bridge": {
                "connected": true,
                "responsive": false,
                "probe": {
                    "ok": false,
                    "transport_ok": true,
                    "required_capabilities": {
                        "content_keepalive_port": false
                    },
                    "error": "loaded extension is missing content_keepalive_port capability"
                }
            },
            "diagnostic": {
                "cause": "stale_extension_bundle",
                "action_text": "Reload the RZN extension from the current extension/dist/chrome bundle, then retry."
            },
            "error": "reload the extension"
        });
        assert!(!should_auto_heal_run_readiness(&stale));

        let mut stale_calls = Vec::new();
        let error = ensure_supervisor_run_ready(
            |method, params| {
                stale_calls.push((method, params));
                future::ready(Ok(stale.clone()))
            },
            None,
        )
        .await
        .expect_err("stale bundle should fail without long heal");
        assert!(error.to_string().contains("extension/dist/chrome"));
        assert_eq!(stale_calls.len(), 1);

        let target_problem = json!({
            "ok": false,
            "ready": false,
            "native_host_bridge": {
                "connected": true,
                "responsive": false,
                "probe": {
                    "ok": false,
                    "transport_ok": true,
                    "target_match_ok": false,
                    "error": "readiness ping reached edge instead of chrome"
                }
            },
            "diagnostic": {
                "cause": "browser_target_mismatch",
                "action_text": "Run `rzn-browser browser targets --json` and select the reported browser/bridge."
            },
            "error": "wrong browser target"
        });
        assert!(!should_auto_heal_run_readiness(&target_problem));

        let mut target_calls = Vec::new();
        let error = ensure_supervisor_run_ready(
            |method, params| {
                target_calls.push((method, params));
                future::ready(Ok(target_problem.clone()))
            },
            None,
        )
        .await
        .expect_err("browser target mismatch should fail without heal");
        assert!(error.to_string().contains("browser targets"));
        assert_eq!(target_calls.len(), 1);
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

    fn unique_temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{}-{}", Uuid::new_v4(), name))
    }
}
