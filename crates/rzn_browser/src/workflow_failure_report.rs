use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

const DEFAULT_REPORT_BASE_URL: &str = "https://cloud.rzn.ai";
const FLOW_FAILURE_REPORT_PATH: &str = "/v1/flow-failure-reports";
const MAX_NOTE_CHARS: usize = 1000;

#[derive(Debug, Clone)]
pub struct WorkflowBrokenReportInput {
    pub product: String,
    pub flow_kind: String,
    pub system: String,
    pub workflow: String,
    pub version: String,
    pub step: String,
    pub error: String,
    pub app_version: String,
    pub platform: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FlowFailureReportDraft {
    pub schema_version: u8,
    pub source: &'static str,
    pub submission_mode: &'static str,
    pub product: String,
    pub flow_kind: String,
    pub surface: String,
    pub flow: String,
    pub flow_version: String,
    pub failed_stage: String,
    pub error: String,
    pub app_version: String,
    pub platform: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

pub type WorkflowFailureReportBody = FlowFailureReportDraft;

#[derive(Debug, Deserialize)]
pub struct WorkflowFailureReportResponse {
    pub ok: bool,
    pub report_id: Option<String>,
    pub group_id: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowFailureReportContext {
    pub product: String,
    pub flow_kind: String,
    pub system: String,
    pub workflow: String,
    pub version: String,
    pub step: String,
    pub error: String,
    pub app_version: String,
    pub platform: String,
}

#[derive(Debug, Clone)]
pub struct WorkflowRunFailure {
    pub message: String,
    pub report_context: WorkflowFailureReportContext,
}

impl fmt::Display for WorkflowRunFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for WorkflowRunFailure {}

pub fn workflow_report_url() -> String {
    resolve_workflow_report_url(
        std::env::var("RZN_FLOW_REPORT_URL").ok().as_deref(),
        std::env::var("RZN_REPORT_BASE_URL").ok().as_deref(),
    )
}

pub fn resolve_workflow_report_url(
    flow_report_url: Option<&str>,
    report_base_url: Option<&str>,
) -> String {
    if let Some(value) = flow_report_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return value.to_string();
    }

    let base = report_base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_REPORT_BASE_URL)
        .trim_end_matches('/');
    format!("{base}{FLOW_FAILURE_REPORT_PATH}")
}

pub fn platform_family() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

pub fn build_report_body(input: WorkflowBrokenReportInput) -> Result<WorkflowFailureReportBody> {
    build_report_body_with_source(input, "rzn-browser-cli", "manual_cli")
}

pub fn build_host_flow_failure_report_draft(
    ctx: &WorkflowFailureReportContext,
    note: Option<String>,
) -> Result<FlowFailureReportDraft> {
    build_report_body_with_source(
        WorkflowBrokenReportInput {
            product: ctx.product.clone(),
            flow_kind: ctx.flow_kind.clone(),
            system: ctx.system.clone(),
            workflow: ctx.workflow.clone(),
            version: ctx.version.clone(),
            step: ctx.step.clone(),
            error: ctx.error.clone(),
            app_version: ctx.app_version.clone(),
            platform: ctx.platform.clone(),
            note,
        },
        "rzn-browser-host",
        "host_auto",
    )
}

pub fn build_host_flow_failure_report_draft_from_failure(
    workflow: &Value,
    workflow_path: &Path,
    step: &Value,
    step_index: usize,
    raw_error: &str,
    note: Option<String>,
) -> Result<FlowFailureReportDraft> {
    let ctx = build_failure_context(workflow, workflow_path, step, step_index, raw_error);
    build_host_flow_failure_report_draft(&ctx, note)
}

fn build_report_body_with_source(
    input: WorkflowBrokenReportInput,
    source: &'static str,
    submission_mode: &'static str,
) -> Result<WorkflowFailureReportBody> {
    let note = input.note.and_then(trim_note);
    let body = FlowFailureReportDraft {
        schema_version: 1,
        source,
        submission_mode,
        product: input.product,
        flow_kind: input.flow_kind,
        surface: input.system,
        flow: input.workflow,
        flow_version: input.version,
        failed_stage: input.step,
        error: input.error,
        app_version: input.app_version,
        platform: input.platform,
        note,
    };
    validate_report_body(&body)?;
    Ok(body)
}

pub fn build_failure_context(
    workflow: &Value,
    workflow_path: &Path,
    step: &Value,
    step_index: usize,
    raw_error: &str,
) -> WorkflowFailureReportContext {
    let step_type = step.get("type").and_then(Value::as_str).unwrap_or("step");
    let system = derive_system(workflow, workflow_path);
    let workflow_id = derive_workflow_id(workflow, &system, workflow_path);
    let version = workflow
        .get("version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("sha256:{}", sha256_short(workflow)));
    let step = step
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(safe_slug)
        .unwrap_or_else(|| format!("step_{}_{}", step_index + 1, safe_slug(step_type)));

    WorkflowFailureReportContext {
        product: "rzn-browser".to_string(),
        flow_kind: "workflow".to_string(),
        system,
        workflow: workflow_id,
        version,
        step,
        error: normalize_workflow_error(raw_error, step_type),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        platform: platform_family().to_string(),
    }
}

pub fn build_failure_context_from_error(
    workflow_ref: &str,
    workflow_path: &Path,
    raw_error: &str,
) -> WorkflowFailureReportContext {
    let workflow = fs::read_to_string(workflow_path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .unwrap_or_else(|| {
            serde_json::json!({
                "id": workflow_ref,
                "version": "unversioned",
                "browser_automation": {"sequences": [{"steps": []}]}
            })
        });
    let (step, step_type) = parse_failed_step_from_error(raw_error);
    let synthetic_step = serde_json::json!({
        "id": step,
        "type": step_type.as_deref().unwrap_or("step")
    });
    build_failure_context(&workflow, workflow_path, &synthetic_step, 0, raw_error)
}

pub fn render_failure_report_block(ctx: &WorkflowFailureReportContext) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Workflow failed: {}", ctx.workflow));
    lines.push(format!("Failed at: {}", ctx.step));
    lines.push(format!("Reason: {}", ctx.error));
    lines.push(String::new());
    lines.push(
        "Reporting this helps us know what broke, group similar failures, and fix the workflow faster."
            .to_string(),
    );
    lines.push(String::new());
    lines.push("Report this broken workflow:".to_string());
    lines.extend(command_lines(ctx, None, 2));
    lines.push(String::new());
    lines.push("This command sends exactly the visible fields in the command.".to_string());
    lines
        .push("It does not read or send workflow inputs, search terms, prompts, URLs,".to_string());
    lines.push(
        "DOM/accessibility trees, screenshots, cookies, local storage, session storage,"
            .to_string(),
    );
    lines.push(
        "logs, stdout/stderr, run_id/trace_id, browser history, file paths, or page titles/text."
            .to_string(),
    );
    lines.push(String::new());
    lines.push("Optional, if you want to add context in your own words:".to_string());
    lines.extend(command_lines(ctx, Some("what happened?"), 2));
    lines.join("\n")
}

pub async fn submit_report(
    body: &WorkflowFailureReportBody,
) -> Result<WorkflowFailureReportResponse> {
    let url = workflow_report_url();
    let response = reqwest::Client::new().post(url).json(body).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("HTTP {}", status.as_u16()));
    }
    let payload: WorkflowFailureReportResponse = response.json().await?;
    if !payload.ok {
        return Err(anyhow!("server returned ok=false"));
    }
    Ok(payload)
}

pub fn report_success_output(response: &WorkflowFailureReportResponse) -> String {
    let verb = match response.status.as_str() {
        "counted" | "rate_limited_counted" => "counted",
        _ => "sent",
    };
    let id = if response.group_id.is_empty() {
        response.report_id.as_deref().unwrap_or("report accepted")
    } else {
        &response.group_id
    };
    format!(
        "Report {}: {}\nThis helps us see which workflows are broken and fix them faster.",
        verb, id
    )
}

pub fn normalize_workflow_error(raw_error: &str, step_type: &str) -> String {
    let raw = raw_error.to_lowercase();
    let step = step_type.to_lowercase();

    if raw.contains("captcha") || raw.contains("anti-bot") || raw.contains("anti bot") {
        return "blocked_by_captcha".to_string();
    }
    if raw.contains("login")
        || raw.contains("sign in")
        || raw.contains("signin")
        || raw.contains("authentication required")
        || raw.contains("auth required")
    {
        return "auth_required".to_string();
    }
    if raw.contains("native-host bridge is not connected")
        || raw.contains("native host is not connected")
        || raw.contains("no native host")
    {
        return "native_host_disconnected".to_string();
    }
    if raw.contains("timeout") || raw.contains("timed out") {
        return "timeout".to_string();
    }
    if raw.contains("native host") || raw.contains("native-host") {
        return "native_host_disconnected".to_string();
    }
    if raw.contains("extension")
        || raw.contains("receiving end does not exist")
        || raw.contains("could not establish connection")
    {
        return "extension_disconnected".to_string();
    }
    if raw.contains("not clickable")
        || raw.contains("not actionable")
        || raw.contains("not interactable")
        || raw.contains("disabled")
    {
        return "element_not_clickable".to_string();
    }
    if raw.contains("navigation")
        || raw.contains("navigate")
        || raw.contains("page load")
        || raw.contains("net::")
    {
        return "navigation_failed".to_string();
    }
    if raw.contains("not found")
        || raw.contains("missing")
        || raw.contains("no element")
        || raw.contains("selector")
        || raw.contains("target")
    {
        if raw.contains("button") || step.contains("button") || step.contains("click") {
            return "button_not_found".to_string();
        }
        if raw.contains("input")
            || step.contains("input")
            || step.contains("fill")
            || step.contains("type")
        {
            return "input_not_found".to_string();
        }
        return "element_not_found".to_string();
    }

    "unknown_failure".to_string()
}

fn validate_report_body(body: &WorkflowFailureReportBody) -> Result<()> {
    require_slug("product", &body.product, false)?;
    require_slug("flow_kind", &body.flow_kind, false)?;
    require_slug("surface", &body.surface, false)?;
    require_slug("flow", &body.flow, true)?;
    require_version_like("flow_version", &body.flow_version)?;
    require_slug("failed_stage", &body.failed_stage, false)?;
    require_slug("error", &body.error, false)?;
    require_version_like("app_version", &body.app_version)?;
    match body.platform.as_str() {
        "macos" | "windows" | "linux" => {}
        _ => return Err(anyhow!("platform must be macos, windows, or linux")),
    }
    if let Some(note) = &body.note {
        if note.chars().count() > MAX_NOTE_CHARS {
            return Err(anyhow!(
                "note must be at most {} characters",
                MAX_NOTE_CHARS
            ));
        }
    }
    Ok(())
}

fn require_slug(field: &str, value: &str, allow_slash: bool) -> Result<()> {
    if value.is_empty() {
        return Err(anyhow!("{field} is required"));
    }
    let ok = value.chars().all(|ch| {
        ch.is_ascii_lowercase()
            || ch.is_ascii_digit()
            || ch == '-'
            || ch == '_'
            || (allow_slash && ch == '/')
    });
    if ok {
        Ok(())
    } else {
        Err(anyhow!("{field} must use lowercase slug characters only"))
    }
}

fn require_version_like(field: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(anyhow!("{field} is required"));
    }
    let ok = value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '+' | ':'));
    if ok {
        Ok(())
    } else {
        Err(anyhow!("{field} contains unsupported characters"))
    }
}

fn trim_note(note: String) -> Option<String> {
    let trimmed = note.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trim_chars(trimmed, MAX_NOTE_CHARS))
}

fn trim_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn derive_system(workflow: &Value, path: &Path) -> String {
    if let Some(id) = workflow.get("id").and_then(Value::as_str) {
        if let Some((system, _)) = id.split_once('/') {
            let slug = safe_slug(system);
            if !slug.is_empty() {
                return slug;
            }
        }
    }
    path.parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .map(safe_slug)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn derive_workflow_id(workflow: &Value, system: &str, path: &Path) -> String {
    if let Some(id) = workflow.get("id").and_then(Value::as_str) {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            let cleaned = trimmed
                .split('/')
                .map(safe_slug)
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("/");
            if cleaned.contains('/') {
                return cleaned;
            }
            if !cleaned.is_empty() {
                return format!("{system}/{cleaned}");
            }
        }
    }

    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(safe_slug)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "workflow".to_string());
    format!("{system}/{name}")
}

fn safe_slug(value: &str) -> String {
    let mut out = String::new();
    let mut last_sep = false;
    for ch in value.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_sep = false;
        } else if (ch == '-' || ch == '_') && !last_sep {
            out.push(ch);
            last_sep = true;
        } else if !last_sep {
            out.push('_');
            last_sep = true;
        }
    }
    out.trim_matches('_').trim_matches('-').to_string()
}

fn sha256_short(workflow: &Value) -> String {
    let bytes = serde_json::to_vec(workflow).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    hex::encode(&digest[..8])
}

fn command_lines(
    ctx: &WorkflowFailureReportContext,
    note: Option<&str>,
    indent: usize,
) -> Vec<String> {
    let pad = " ".repeat(indent);
    let field_pad = " ".repeat(indent + 2);
    let mut lines = vec![format!("{pad}rzn-browser report workflow-broken \\")];
    lines.push(format!("{field_pad}--product {} \\", ctx.product));
    lines.push(format!("{field_pad}--flow-kind {} \\", ctx.flow_kind));
    lines.push(format!("{field_pad}--system {} \\", ctx.system));
    lines.push(format!("{field_pad}--workflow {} \\", ctx.workflow));
    lines.push(format!("{field_pad}--version {} \\", ctx.version));
    lines.push(format!("{field_pad}--step {} \\", ctx.step));
    lines.push(format!("{field_pad}--error {} \\", ctx.error));
    lines.push(format!("{field_pad}--app-version {} \\", ctx.app_version));
    if let Some(note) = note {
        lines.push(format!("{field_pad}--platform {} \\", ctx.platform));
        lines.push(format!("{field_pad}--note {}", quote_note(note)));
    } else {
        lines.push(format!("{field_pad}--platform {}", ctx.platform));
    }
    lines
}

fn quote_note(note: &str) -> String {
    format!("\"{}\"", note.replace('\\', "\\\\").replace('"', "\\\""))
}

fn parse_failed_step_from_error(error: &str) -> (String, Option<String>) {
    if let Some(rest) = error.strip_prefix("step ") {
        if let Some((step, after_step)) = rest.split_once(" (") {
            if let Some((step_type, _)) = after_step.split_once(") failed") {
                return (safe_slug(step), Some(safe_slug(step_type)));
            }
        }
    }
    ("unknown_step".to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn dry_run_body_uses_only_explicit_report_fields() {
        let body = build_report_body(WorkflowBrokenReportInput {
            product: "rzn-browser".to_string(),
            flow_kind: "workflow".to_string(),
            system: "google".to_string(),
            workflow: "google/search-v1".to_string(),
            version: "2026-04-24.1".to_string(),
            step: "search_button".to_string(),
            error: "button_not_found".to_string(),
            app_version: "0.8.3".to_string(),
            platform: "macos".to_string(),
            note: Some("  page changed  ".to_string()),
        })
        .unwrap();

        assert_eq!(
            serde_json::to_value(body).unwrap(),
            json!({
                "schema_version": 1,
                "source": "rzn-browser-cli",
                "submission_mode": "manual_cli",
                "product": "rzn-browser",
                "flow_kind": "workflow",
                "surface": "google",
                "flow": "google/search-v1",
                "flow_version": "2026-04-24.1",
                "failed_stage": "search_button",
                "error": "button_not_found",
                "app_version": "0.8.3",
                "platform": "macos",
                "note": "page changed"
            })
        );

        let pretty = serde_json::to_string_pretty(
            &build_report_body(WorkflowBrokenReportInput {
                product: "rzn-browser".to_string(),
                flow_kind: "workflow".to_string(),
                system: "google".to_string(),
                workflow: "google/search-v1".to_string(),
                version: "2026-04-24.1".to_string(),
                step: "search_button".to_string(),
                error: "button_not_found".to_string(),
                app_version: "0.8.3".to_string(),
                platform: "macos".to_string(),
                note: Some("page changed".to_string()),
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            pretty,
            r#"{
  "schema_version": 1,
  "source": "rzn-browser-cli",
  "submission_mode": "manual_cli",
  "product": "rzn-browser",
  "flow_kind": "workflow",
  "surface": "google",
  "flow": "google/search-v1",
  "flow_version": "2026-04-24.1",
  "failed_stage": "search_button",
  "error": "button_not_found",
  "app_version": "0.8.3",
  "platform": "macos",
  "note": "page changed"
}"#
        );
    }

    #[test]
    fn report_body_rejects_private_looking_slug_values() {
        let err = build_report_body(WorkflowBrokenReportInput {
            product: "rzn-browser".to_string(),
            flow_kind: "workflow".to_string(),
            system: "google".to_string(),
            workflow: "google/search-v1".to_string(),
            version: "1.0.0".to_string(),
            step: "https://example.com?q=private".to_string(),
            error: "element_not_found".to_string(),
            app_version: "0.1.0".to_string(),
            platform: "macos".to_string(),
            note: None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("failed_stage"));
    }

    #[test]
    fn simulated_failure_command_contains_only_safe_fields() {
        let workflow = json!({
            "id": "google/search-v1",
            "version": "2026-04-24.1",
            "browser_automation": {"sequences": [{"steps": []}]}
        });
        let step = json!({"id": "search_button", "type": "click"});
        let ctx = build_failure_context(
            &workflow,
            Path::new("/Users/sara/private/google/search.json"),
            &step,
            0,
            "Selector not found at https://google.com/search?q=private",
        );
        let block = render_failure_report_block(&ctx);

        assert!(block.contains("--workflow google/search-v1"));
        assert!(block.contains("--error button_not_found"));
        assert!(!block.contains("https://google.com"));
        assert!(!block.contains("private"));
        assert!(block.contains("This command sends exactly the visible fields in the command."));
        assert!(block.contains("workflow inputs, search terms, prompts, URLs"));
        assert!(block.contains("DOM/accessibility trees, screenshots, cookies"));
        assert!(block.contains("stdout/stderr, run_id/trace_id"));
    }

    #[test]
    fn raw_private_error_is_normalized_not_printed() {
        let code = normalize_workflow_error(
            "input missing for https://example.com?q=medical-search",
            "fill_input",
        );
        assert_eq!(code, "input_not_found");
    }

    #[test]
    fn bridge_request_timeout_is_reported_as_timeout() {
        let code = normalize_workflow_error(
            "Supervisor error: {\"code\":-32000,\"message\":\"Native-host extension bridge timeout after 40000ms\"}",
            "execute_step",
        );
        assert_eq!(code, "timeout");
    }

    #[test]
    fn endpoint_resolution_prefers_full_override_then_base_url() {
        assert_eq!(
            resolve_workflow_report_url(
                Some("https://legacy.example.test/custom-report"),
                Some("https://base.example.test")
            ),
            "https://legacy.example.test/custom-report"
        );
        assert_eq!(
            resolve_workflow_report_url(None, Some("https://base.example.test/")),
            "https://base.example.test/v1/flow-failure-reports"
        );
        assert_eq!(
            resolve_workflow_report_url(None, None),
            "https://cloud.rzn.ai/v1/flow-failure-reports"
        );
    }

    #[test]
    fn host_draft_builder_returns_same_sanitized_contract() {
        let ctx = WorkflowFailureReportContext {
            product: "rzn-browser".to_string(),
            flow_kind: "workflow".to_string(),
            system: "google".to_string(),
            workflow: "google/search-v1".to_string(),
            version: "2026-04-24.1".to_string(),
            step: "search_button".to_string(),
            error: "button_not_found".to_string(),
            app_version: "0.1.0".to_string(),
            platform: "macos".to_string(),
        };
        let draft =
            build_host_flow_failure_report_draft(&ctx, Some(" user-authored only ".to_string()))
                .unwrap();
        let value = serde_json::to_value(draft).unwrap();

        assert_eq!(
            value,
            json!({
                "schema_version": 1,
                "submission_mode": "host_auto",
                "source": "rzn-browser-host",
                "product": "rzn-browser",
                "flow_kind": "workflow",
                "surface": "google",
                "flow": "google/search-v1",
                "flow_version": "2026-04-24.1",
                "failed_stage": "search_button",
                "error": "button_not_found",
                "app_version": "0.1.0",
                "platform": "macos",
                "note": "user-authored only"
            })
        );
    }

    #[test]
    fn host_failure_builder_returns_same_sanitized_contract_from_raw_failure() {
        let workflow = json!({
            "id": "google/search-v1",
            "version": "2026-04-24.1",
            "browser_automation": {"sequences": [{"steps": []}]}
        });
        let step = json!({"id": "search_button", "type": "click"});
        let draft = build_host_flow_failure_report_draft_from_failure(
            &workflow,
            Path::new("/Users/sara/private/google/search.json"),
            &step,
            0,
            "Selector not found at https://google.com/search?q=private",
            None,
        )
        .unwrap();
        let value = serde_json::to_value(draft).unwrap();

        assert_eq!(value["submission_mode"], "host_auto");
        assert_eq!(value["source"], "rzn-browser-host");
        assert_eq!(value["surface"], "google");
        assert_eq!(value["flow"], "google/search-v1");
        assert_eq!(value["flow_version"], "2026-04-24.1");
        assert_eq!(value["failed_stage"], "search_button");
        assert_eq!(value["error"], "button_not_found");
        assert!(!value.to_string().contains("private"));
        assert!(!value.to_string().contains("https://google.com"));
    }
}
