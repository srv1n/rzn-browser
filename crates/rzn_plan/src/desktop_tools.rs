use crate::broker_client::{BrokerClient, DomSnapshot};
use crate::{PlanError, PlanResult};
use rzn_core::{Step, StepKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopErrorCode {
    Timeout,
    TargetNotFound,
    PolicyBlocked,
    TransportError,
    ExtensionError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopToolError {
    pub code: DesktopErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dom_hash: Option<String>,
}

impl std::fmt::Display for DesktopToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for DesktopToolError {}

pub type DesktopToolResult<T> = Result<T, DesktopToolError>;

pub(crate) fn map_plan_error(err: PlanError) -> DesktopToolError {
    match err {
        PlanError::PolicyBlocked(reason) => DesktopToolError {
            code: DesktopErrorCode::PolicyBlocked,
            message: reason,
            error_code: Some("POLICY_BLOCKED".to_string()),
            current_url: None,
            dom_hash: None,
        },
        other => DesktopToolError {
            code: DesktopErrorCode::TransportError,
            message: other.to_string(),
            error_code: None,
            current_url: None,
            dom_hash: None,
        },
    }
}

fn classify_extension_error_code(code: &str) -> DesktopErrorCode {
    match code {
        "TIMEOUT" | "NAVIGATION_TIMEOUT" | "WORKFLOW_TIMEOUT" | "TASK_TIMEOUT" => {
            DesktopErrorCode::Timeout
        }
        "SELECTOR_NOT_FOUND" | "TARGET_NOT_FOUND" | "NO_MATCH" => DesktopErrorCode::TargetNotFound,
        "POLICY_BLOCKED" => DesktopErrorCode::PolicyBlocked,
        _ => DesktopErrorCode::ExtensionError,
    }
}

fn error_from_response(resp: &Value) -> DesktopToolError {
    let error_code = resp
        .get("error_code")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let message = resp
        .get("error_msg")
        .or(resp.get("error"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Unknown error".to_string());

    let mut code = error_code
        .as_deref()
        .map(classify_extension_error_code)
        .unwrap_or(DesktopErrorCode::ExtensionError);

    // Fallback classification by message if no explicit error_code
    if error_code.is_none() {
        let msg_lower = message.to_lowercase();
        if msg_lower.contains("timed out") {
            code = DesktopErrorCode::Timeout;
        }
        if msg_lower.contains("selector_not_found")
            || msg_lower.contains("not found")
            || msg_lower.contains("no element")
        {
            code = DesktopErrorCode::TargetNotFound;
        }
    }

    DesktopToolError {
        code,
        message,
        error_code,
        current_url: resp
            .get("current_url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        dom_hash: resp
            .get("dom_hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    }
}

fn ensure_success(resp: Value) -> DesktopToolResult<Value> {
    match resp.get("success").and_then(|v| v.as_bool()) {
        Some(true) => Ok(resp),
        _ => Err(error_from_response(&resp)),
    }
}

impl BrokerClient {
    /// Desktop-facing primitive: fetch the enhanced DOM snapshot with structured errors.
    pub async fn get_dom_snapshot_desktop(&mut self) -> DesktopToolResult<DomSnapshot> {
        let resp = self.get_dom_snapshot().await.map_err(map_plan_error)?;
        let resp = ensure_success(resp)?;

        let snapshot_value = resp
            .get("dom_snapshot")
            .cloned()
            .ok_or_else(|| DesktopToolError {
                code: DesktopErrorCode::ExtensionError,
                message: "Missing dom_snapshot in response".to_string(),
                error_code: None,
                current_url: resp
                    .get("current_url")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                dom_hash: resp
                    .get("dom_hash")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            })?;

        serde_json::from_value::<DomSnapshot>(snapshot_value).map_err(|e| DesktopToolError {
            code: DesktopErrorCode::ExtensionError,
            message: format!("Failed to parse dom_snapshot: {}", e),
            error_code: None,
            current_url: resp
                .get("current_url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            dom_hash: resp
                .get("dom_hash")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })
    }

    /// Desktop-facing primitive: observe page and return selector candidates.
    pub async fn observe_desktop(
        &mut self,
        instruction: &str,
        scope_selector: Option<&str>,
        max_items: Option<u32>,
    ) -> DesktopToolResult<Value> {
        let resp = self
            .observe(instruction, scope_selector, max_items)
            .await
            .map_err(map_plan_error)?;
        ensure_success(resp)
    }

    /// Desktop-facing primitive: deterministic extraction via validated plan (no arbitrary JS).
    pub async fn execute_extraction_plan_desktop(
        &mut self,
        plan: Value,
    ) -> DesktopToolResult<Value> {
        let resp = self
            .execute_extraction_plan(plan)
            .await
            .map_err(map_plan_error)?;
        ensure_success(resp)
    }

    /// Desktop-facing primitive: execute a single step and return the raw extension response
    /// (dom_snapshot/dom_hash/current_url preserved when provided by the extension).
    ///
    /// Note: for navigation/page-source steps we route through the workflow runner path to
    /// ensure tab-level actions are handled in the background script.
    pub async fn act_step_desktop(&mut self, step: &Step) -> DesktopToolResult<Value> {
        match &step.kind {
            StepKind::NavigateToUrl { .. } | StepKind::GetPageSource => {
                let (resp, _dom) = self
                    .execute_step_and_get_dom(step)
                    .await
                    .map_err(map_plan_error)?;
                ensure_success(resp)
            }
            _ => {
                let payload = serde_json::to_value(step).map_err(|e| DesktopToolError {
                    code: DesktopErrorCode::TransportError,
                    message: format!("Failed to serialize step: {}", e),
                    error_code: None,
                    current_url: None,
                    dom_hash: None,
                })?;
                let resp = self
                    .execute_raw_step(payload)
                    .await
                    .map_err(map_plan_error)?;
                ensure_success(resp)
            }
        }
    }

    /// Desktop-facing primitive: execute many steps sequentially, collecting responses.
    ///
    /// This avoids relying on a single workflow response shape and preserves per-step
    /// dom_hash/current_url for the caller.
    pub async fn execute_steps_desktop(&mut self, steps: &[Step]) -> DesktopToolResult<Value> {
        let mut out: Vec<Value> = Vec::with_capacity(steps.len());
        for step in steps {
            let resp = self.act_step_desktop(step).await?;
            out.push(resp);
        }
        Ok(json!({ "success": true, "steps": out }))
    }
}

// Internal helpers for callers that still want PlanResult.
pub(crate) fn ensure_success_plan(resp: Value) -> PlanResult<Value> {
    match resp.get("success").and_then(|v| v.as_bool()) {
        Some(true) => Ok(resp),
        _ => Err(PlanError::ExecutionError(
            resp.get("error_msg")
                .or(resp.get("error"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error")
                .to_string(),
        )),
    }
}
