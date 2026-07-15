//! CLI glue around the shared [`workflow_runner`] loop.
//!
//! This module owns the CLI-facing entry point (`run_supervisor_workflow`), the
//! local-socket JSON-RPC transport ([`CliStepTransport`]) that implements
//! [`StepTransport`] over `supervisor::call`, a [`CliEventSink`] that prints the
//! exact `[OK]/[ERR]/[STOP]` progress lines the CLI has always emitted, and the
//! supervisor bootstrap + run-readiness preflight. The workflow loading, param
//! normalization, per-step execution loop, retry/timeout semantics, output
//! selection and `RunResultV2` assembly all live in [`crate::workflow_runner`].

use crate::run_store::{AppendRun, RunStore};
use crate::supervisor;
use crate::workflow_runner::{
    load_workflow_for_run, parse_env_bool, response_error_message, response_success, run_workflow,
    validate_steps, with_browser_target, RunEventSink, RunOptions, SessionSpec, StepTransport,
    TransportError,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rzn_contracts::v2::{RunErrorV1, RunStatusV2};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::Duration;
use uuid::Uuid;

pub use crate::workflow_runner::SnapshotMode;

#[derive(Debug, Clone)]
pub struct SupervisorRunConfig {
    pub workflow_path: String,
    pub params: HashMap<String, String>,
    pub snapshot_mode: SnapshotMode,
    pub app_base: Option<String>,
    pub browser_target: Option<Value>,
}

pub async fn run_supervisor_workflow(config: SupervisorRunConfig) -> Result<Option<Value>> {
    let workflow = load_workflow_for_run(&config.workflow_path, &config.params)?;
    let workflow_id = workflow
        .runtime_context
        .as_ref()
        .map(|c| c.workflow_id.clone())
        .unwrap_or_else(|| "rzn.legacy.workflow".into());
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

    let transport = CliStepTransport {
        config: supervisor_config,
    };
    let sink = CliEventSink;
    let opts = RunOptions {
        run_id: format!("local-{}", Uuid::new_v4()),
        workflow_hash: workflow_hash(&config.workflow_path).ok(),
        params: config.params.clone(),
        deadline: None,
        session: SessionSpec {
            browser_target: config.browser_target.clone(),
            ..SessionSpec::default()
        },
        snapshot_mode: config.snapshot_mode,
        workflow_path: config.workflow_path.clone(),
    };

    let started = epoch_ms();
    let outcome = run_workflow(&transport, &sink, &workflow, &opts).await;
    let mut result = match &outcome {
        Ok(Some(v)) => crate::workflow_runner::run_result_from_output_value(
            v.clone(),
            &opts.run_id,
            &workflow_id,
        ),
        Ok(None) => crate::workflow_runner::run_result_shell(
            RunStatusV2::Succeeded,
            None,
            &opts.run_id,
            &workflow_id,
            None,
        ),
        Err(e) => crate::workflow_runner::run_result_shell(
            RunStatusV2::Failed,
            None,
            &opts.run_id,
            &workflow_id,
            Some(RunErrorV1 {
                code: "workflow_execution_error".into(),
                message: e.to_string(),
                step_id: None,
                retry_hint: None,
            }),
        ),
    };
    if let Err(error) = &outcome {
        crate::workflow_runner::enrich_failure_result(
            &mut result,
            error,
            &workflow_hash(&config.workflow_path).unwrap_or_default(),
        );
    }
    if result.status != RunStatusV2::Succeeded && result.failure_summary.is_none() {
        let error = result.error.as_ref();
        let code = error.map_or("unknown", |e| e.code.as_str());
        let message = error.map_or("workflow failed", |e| e.message.as_str());
        result.failure_summary = Some(crate::workflow_health::failure_summary(
            &workflow_hash(&config.workflow_path).unwrap_or_default(),
            None,
            code,
            message,
        ));
    }
    let params = serde_json::to_value(&config.params)?;
    let base = config
        .app_base
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(rzn_core::runtime_paths::default_app_base_dir);
    let workflow_hash = workflow_hash(&config.workflow_path).ok();
    RunStore::open(base)?.append(AppendRun {
        origin: "local_cli",
        workflow_hash: workflow_hash.as_deref(),
        started_at: started,
        ended_at: epoch_ms(),
        params: &params,
        result: &result,
    })?;
    outcome
}

fn workflow_hash(path: &str) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(hex::encode(Sha256::digest(std::fs::read(path)?)))
}

fn epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// [`StepTransport`] over the existing local-socket JSON-RPC client.
///
/// `timeout_ms == 0` means "no client-side watchdog, await the call directly" —
/// exactly how the CLI has always issued session open/close and snapshot calls.
/// A non-zero `timeout_ms` reproduces the per-step watchdog: the call is bounded
/// by `tokio::time::timeout`, and elapsing surfaces as [`TransportError::Timeout`].
struct CliStepTransport {
    config: supervisor::SupervisorConfig,
}

#[async_trait]
impl StepTransport for CliStepTransport {
    async fn call(
        &self,
        method: &str,
        params: Value,
        timeout_ms: u64,
    ) -> Result<Value, TransportError> {
        let call = supervisor::call(self.config.clone(), method, params);
        if timeout_ms == 0 {
            return call.await.map_err(TransportError::Call);
        }
        match tokio::time::timeout(Duration::from_millis(timeout_ms), call).await {
            Ok(result) => result.map_err(TransportError::Call),
            Err(_) => Err(TransportError::Timeout),
        }
    }
}

/// Prints the exact CLI progress lines the run loop used to emit inline.
struct CliEventSink;

impl RunEventSink for CliEventSink {
    fn on_session_open(&self, session_id: Option<&str>) {
        if let Some(session) = session_id {
            println!("[OK] Session opened: {}", session);
        } else {
            println!("[WARN] Session opened (no session_id returned)");
        }
    }

    fn on_step_start(&self, idx: usize, total: usize, step_id: &str, step_type: &str) {
        println!("[STEP] {}/{} {} ({})", idx + 1, total, step_id, step_type);
    }

    fn on_step_response(&self, step_id: &str, step_type: &str, response: &Value) {
        log_step_response(step_id, step_type, response);
    }

    fn on_snapshot(&self, dom_hash: Option<&str>) {
        if let Some(hash) = dom_hash {
            println!("[SNAPSHOT] dom_hash={}", hash);
        } else {
            println!("[SNAPSHOT] ok");
        }
    }

    fn on_stop(&self, step_id: &str, step_type: &str, reason: &str) {
        println!(
            "[STOP] Workflow halted after {} ({}): {}",
            step_id, step_type, reason
        );
    }

    fn on_result(&self, run_result: &Value) {
        if let Ok(pretty) = serde_json::to_string_pretty(run_result) {
            println!("{}", pretty);
        } else {
            println!("{}", run_result);
        }
    }
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

fn emit_runtime_status(message: String) {
    if parse_env_bool("RZN_BROWSER_MCP_STDIO").unwrap_or(false) {
        eprintln!("{}", message);
    } else {
        println!("{}", message);
    }
}

#[cfg(test)]
mod tests {
    use super::{ensure_supervisor_run_ready, should_auto_heal_run_readiness};
    use serde_json::{json, Value};
    use std::collections::VecDeque;
    use std::future;

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
}
