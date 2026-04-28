// Unified autonomous planner (formerly *_v2). Kept API identical.
use crate::{
    broker_client::BrokerClient,
    element_ref::TargetSpec,
    llm::LLMClient,
    planner::{PlannerMode, PlannerState, PolicyValidator},
    tool_llm::{Tool, ToolCall, ToolOnlyLLMClient},
    PlanError, PlanResult,
};
use log::{error, info, warn};
use regex;
use rzn_core::dsl::Step;
use rzn_core::{FieldSpec, StepKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMAutonomousRequest {
    pub instruction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_url: Option<String>,
    pub max_steps: Option<usize>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMAutonomousResponse {
    pub success: bool,
    pub steps_executed: usize,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub extracted_data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMAction {
    pub thought: String,
    pub action: ActionCommand,
    pub result: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionCommand {
    pub cmd: String,
    pub args: Vec<Value>,
}

#[derive(Debug, Clone)]
struct SelectorDescriptor {
    encoded_id: String,
    css_selector: String,
    frame_id: Option<String>,
    frame_ordinal: Option<u32>,
    role: Option<String>,
    name: Option<String>,
    text: Option<String>,
    attributes: HashMap<String, String>,
    actions: Vec<String>,
}

pub struct LLMAutonomousPlanner {
    llm_client: LLMClient,
    broker_client: BrokerClient,
    system_prompt: String,
    conversation_history: Vec<Value>,
    current_url: Option<String>,
    planner_state: PlannerState,
    policy_validator: PolicyValidator,
    tool_llm_client: Option<ToolOnlyLLMClient>,
    correlation_id: String,
    last_dom_hash: Option<String>,
    scrolls_since_last_extract: u32,
    // Track the most recent scroll direction to avoid oscillation (up/down bouncing)
    last_scroll_direction: Option<String>,
    selector_inventory: HashMap<String, SelectorDescriptor>,
    selector_lookup_css: HashMap<String, String>,
    executed_steps: Vec<Step>,
    recorded_steps: Vec<RecordedStep>,
    last_auto_container_selector: Option<String>,
    last_auto_item_selector: Option<String>,
    options: LLMAutonomousOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedStep {
    pub step: Step,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub pre_url: Option<String>,
    pub post_url: Option<String>,
    pub pre_dom_hash: Option<String>,
    pub post_dom_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug, Clone)]
pub struct LLMAutonomousOptions {
    /// Enable deterministic "fast paths" (macros) that can short-circuit the LLM loop.
    ///
    /// When false, the planner runs a pure observe→LLM→act loop and only performs
    /// substrate-level safety checks (inventory refresh, validation, waits).
    pub enable_macros: bool,
}

impl Default for LLMAutonomousOptions {
    fn default() -> Self {
        Self {
            enable_macros: true,
        }
    }
}

impl LLMAutonomousPlanner {
    /// Best-effort recovery for common interstitials (modals/popups/captcha).
    ///
    /// This is intentionally generic and bounded. It runs outside the LLM decision
    /// loop so we don't "burn" model turns on cookie banners and login modals.
    async fn auto_handle_interstitials(&mut self) -> PlanResult<()> {
        // Captcha detection: do not attempt to solve automatically.
        // If detected, surface a structured error so the host can pause and ask the user to solve.
        let cap_step = Step {
            id: format!("captcha_check_{}", Uuid::new_v4()),
            name: "Detect captcha".to_string(),
            kind: StepKind::HandleCaptcha,
        };
        if let Ok(resp) = self.execute_step_record(cap_step).await {
            let detected = resp
                .get("result")
                .and_then(|r| r.get("captcha_detected"))
                .and_then(|v| v.as_bool())
                .or_else(|| resp.get("captcha_detected").and_then(|v| v.as_bool()))
                .unwrap_or(false);
            if detected {
                // Optional in-page notice (best-effort, no payload support in StepKind v1)
                let _ = self
                    .execute_step_record(Step {
                        id: format!("captcha_ui_{}", Uuid::new_v4()),
                        name: "Request user intervention".to_string(),
                        kind: StepKind::RequestUserIntervention {
                            message: Some(
                                "Captcha detected. Solve it in the browser, then continue."
                                    .to_string(),
                            ),
                            instructions: None,
                            timeout_ms: None,
                            approval_mode: None,
                            approval_policy: None,
                            continue_on_timeout: None,
                            notification_title: None,
                            notification_message: None,
                        },
                    })
                    .await;

                return Err(PlanError::ExecutionError(
                    "captcha_detected: manual solve required".to_string(),
                ));
            }
        }

        // Popup/modal detection + dismissal.
        let detect_step = Step {
            id: format!("detect_popups_{}", Uuid::new_v4()),
            name: "Detect popups".to_string(),
            kind: StepKind::DetectPopups,
        };
        if let Ok(resp) = self.execute_step_record(detect_step).await {
            let detected = resp
                .get("result")
                .and_then(|r| r.get("popups_detected"))
                .and_then(|v| v.as_bool())
                .or_else(|| resp.get("popups_detected").and_then(|v| v.as_bool()))
                .unwrap_or(false);
            if detected {
                let _ = self
                    .execute_step_record(Step {
                        id: format!("dismiss_popups_{}", Uuid::new_v4()),
                        name: "Dismiss popups".to_string(),
                        kind: StepKind::DismissPopups,
                    })
                    .await;
                let _ = self
                    .execute_step_record(Step {
                        id: format!("wait_no_popups_{}", Uuid::new_v4()),
                        name: "Wait for no popups".to_string(),
                        kind: StepKind::WaitForNoPopups,
                    })
                    .await;
            }
        }

        Ok(())
    }

    async fn update_selector_inventory(&mut self) -> Result<(), String> {
        self.selector_inventory.clear();
        self.selector_lookup_css.clear();

        // Prefer the content-script dom_snapshot (no CDP attach / no infobar).
        // If it's missing/unavailable, fall back to CDP context.
        if let Ok(snapshot) = self.broker_client.get_dom_snapshot().await {
            if let Some(elements) = snapshot
                .get("dom_snapshot")
                .and_then(|d| d.get("elements"))
                .and_then(|v| v.as_array())
            {
                for (idx, el) in elements.iter().enumerate() {
                    let css_selector = match el
                        .get("selector")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                    {
                        Some(sel) => sel,
                        None => continue,
                    };

                    // Prefer stable element ids provided by the content script snapshot.
                    // Fallback to a deterministic id derived from the current element order.
                    let encoded_id = el
                        .get("id")
                        .or_else(|| el.get("encoded_id"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| format!("0:{}", idx + 1));

                    let tag = el
                        .get("tag")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let text = el
                        .get("text")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    let attributes = el
                        .get("attributes")
                        .and_then(|v| v.as_object())
                        .map(|map| {
                            map.iter()
                                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                                .collect::<HashMap<_, _>>()
                        })
                        .unwrap_or_default();

                    let role = attributes.get("role").cloned().or_else(|| {
                        if tag == "a" {
                            Some("link".to_string())
                        } else if tag == "button" {
                            Some("button".to_string())
                        } else if tag == "input" || tag == "textarea" || tag == "select" {
                            Some("textbox".to_string())
                        } else {
                            None
                        }
                    });

                    let name = attributes
                        .get("aria-label")
                        .cloned()
                        .or_else(|| attributes.get("placeholder").cloned())
                        .or_else(|| attributes.get("name").cloned())
                        .or_else(|| attributes.get("id").cloned());

                    let mut actions: Vec<String> = Vec::new();
                    let input_type = attributes
                        .get("type")
                        .map(|s| s.to_lowercase())
                        .unwrap_or_default();

                    let is_clickable = tag == "button"
                        || tag == "a"
                        || attributes.contains_key("onclick")
                        || matches!(role.as_deref(), Some("button" | "link"));

                    let is_writable = tag == "textarea"
                        || (tag == "input"
                            && !matches!(
                                input_type.as_str(),
                                "button" | "submit" | "reset" | "checkbox" | "radio" | "file"
                            ))
                        || attributes
                            .get("contenteditable")
                            .map(|v| v == "true")
                            .unwrap_or(false);

                    if is_clickable {
                        actions.push("click".to_string());
                    }
                    if is_writable {
                        actions.push("type".to_string());
                    }
                    if tag == "input" && matches!(input_type.as_str(), "checkbox" | "radio") {
                        if !actions.iter().any(|a| a == "click") {
                            actions.push("click".to_string());
                        }
                    }

                    if actions.is_empty() {
                        continue;
                    }

                    let descriptor = SelectorDescriptor {
                        encoded_id: encoded_id.clone(),
                        css_selector: css_selector.clone(),
                        frame_id: None,
                        frame_ordinal: encoded_id
                            .split(':')
                            .next()
                            .and_then(|s| s.parse::<u32>().ok())
                            .or(Some(0)),
                        role,
                        name,
                        text,
                        attributes,
                        actions,
                    };

                    self.selector_lookup_css
                        .insert(css_selector.clone(), encoded_id.clone());
                    self.selector_lookup_css
                        .insert(css_selector.to_lowercase(), encoded_id.clone());

                    self.selector_inventory.insert(encoded_id, descriptor);
                }

                if !self.selector_inventory.is_empty() {
                    log::info!(
                        "[{}] Selector inventory populated from DOM snapshot: {} elements",
                        self.correlation_id,
                        self.selector_inventory.len()
                    );
                    return Ok(());
                }
            }
        }

        let context = self
            .broker_client
            .get_cdp_context(Some(json!({ "maxElements": 80 })))
            .await
            .map_err(|e| format!("CDP context error: {}", e))?;

        let elements = context
            .get("result")
            .and_then(|r| r.get("elements"))
            .or_else(|| context.get("elements"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut skipped_missing_token = 0usize;
        let mut total_seen = 0usize;
        for elem in elements {
            total_seen += 1;
            let token = elem
                .get("token")
                .or_else(|| elem.get("selector"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string());

            let encoded_id = match token {
                Some(t) if !t.is_empty() => t,
                _ => {
                    skipped_missing_token += 1;
                    continue;
                }
            };

            let css_selector = elem
                .get("css_selector")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| encoded_id.clone());

            let role = elem
                .get("type")
                .or_else(|| elem.get("role"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let name = elem
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let text = elem
                .get("text")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let frame_id = elem
                .get("frame_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let frame_ordinal = elem
                .get("frame_ordinal")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);

            let attributes = elem
                .get("attributes")
                .and_then(|v| v.as_object())
                .map(|map| {
                    map.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect::<HashMap<_, _>>()
                })
                .unwrap_or_default();

            let actions = elem
                .get("actions")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            let descriptor = SelectorDescriptor {
                encoded_id: encoded_id.clone(),
                css_selector: css_selector.clone(),
                frame_id,
                frame_ordinal,
                role,
                name,
                text,
                attributes,
                actions,
            };

            self.selector_lookup_css
                .insert(css_selector.clone(), encoded_id.clone());
            self.selector_lookup_css
                .insert(css_selector.to_lowercase(), encoded_id.clone());

            self.selector_inventory.insert(encoded_id, descriptor);
        }

        if self.selector_inventory.is_empty() {
            log::warn!(
                "[{}] Selector inventory empty (total_seen={}, skipped_token={}). CDP keys: {:?}",
                self.correlation_id,
                total_seen,
                skipped_missing_token,
                context
                    .as_object()
                    .map(|o| o.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
            );
        } else {
            log::info!(
                "[{}] Selector inventory populated: {} elements (total_seen={}, skipped_token={})",
                self.correlation_id,
                self.selector_inventory.len(),
                total_seen,
                skipped_missing_token
            );
        }

        Ok(())
    }

    pub fn export_workflow(&self, goal: &str) -> Option<rzn_core::Workflow> {
        use rzn_core::dsl::{BrowserAutomation, Sequence, Workflow};

        // Derive an id and name from the goal
        fn slugify(s: &str) -> String {
            let mut out = String::new();
            for ch in s.chars() {
                if ch.is_ascii_alphanumeric() {
                    out.push(ch.to_ascii_lowercase());
                } else if ch.is_whitespace() || ch == '-' || ch == '_' {
                    out.push('-');
                }
            }
            while out.contains("--") {
                out = out.replace("--", "-");
            }
            out.trim_matches('-').chars().take(48).collect()
        }

        let mut steps: Vec<Step> = Vec::new();
        // Keep only deterministic, replayable steps (actor-grade surface).
        // We also sprinkle lightweight waits after navigation-triggering actions based on
        // observed URL changes during the run.
        let mut has_explicit_extract = false;
        for rec in &self.recorded_steps {
            let st = &rec.step;
            match &st.kind {
                StepKind::NavigateToUrl { .. }
                | StepKind::ClickElement { .. }
                | StepKind::DblClickElement { .. }
                | StepKind::HoverElement { .. }
                | StepKind::FillInputField { .. }
                | StepKind::PressSpecialKey { .. }
                | StepKind::WaitForElement { .. }
                | StepKind::WaitForTimeout { .. }
                | StepKind::ScrollWindowTo { .. }
                | StepKind::ScrollElementIntoView { .. }
                | StepKind::InfiniteScroll { .. }
                | StepKind::ExtractStructuredData { .. }
                | StepKind::GetPageSource => {
                    if matches!(&st.kind, StepKind::ExtractStructuredData { .. }) {
                        has_explicit_extract = true;
                    }
                    steps.push(st.clone());

                    let url_changed = rec.pre_url.is_some()
                        && rec.post_url.is_some()
                        && rec.pre_url.as_deref() != rec.post_url.as_deref();
                    let is_navish = matches!(
                        &st.kind,
                        StepKind::NavigateToUrl { .. }
                            | StepKind::ClickElement { .. }
                            | StepKind::DblClickElement { .. }
                            | StepKind::PressSpecialKey { .. }
                    );

                    if url_changed && is_navish {
                        steps.push(Step {
                            id: format!("wait_nav_{}", Uuid::new_v4()),
                            name: "Wait for navigation".to_string(),
                            kind: StepKind::WaitForNavigation {
                                url_pattern: None,
                                timeout_ms: Some(30_000),
                            },
                        });
                    } else if is_navish {
                        // Small settle wait after UI actions (generic; avoids racing dynamic DOM updates).
                        let last_is_wait = steps
                            .last()
                            .map(|s| matches!(&s.kind, StepKind::WaitForTimeout { .. }))
                            .unwrap_or(false);
                        if !last_is_wait {
                            steps.push(Step {
                                id: format!("wait_settle_{}", Uuid::new_v4()),
                                name: "Wait for UI to settle".to_string(),
                                kind: StepKind::WaitForTimeout { timeout_ms: 400 },
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        // If we never executed an explicit extract step but we did discover a repeated list,
        // append a minimal deterministic extract (useful for "top-N results" tasks).
        if !has_explicit_extract {
            if let Some(item_sel) = &self.last_auto_item_selector {
                steps.push(Step {
                    id: "extract_items".to_string(),
                    name: "Extract list items".to_string(),
                    kind: StepKind::ExtractStructuredData {
                        item_selector: item_sel.clone(),
                        limit: None,
                        fields: vec![FieldSpec {
                            name: "title".into(),
                            selector: "*".into(),
                            attribute: None,
                            post_processing: vec![],
                        }],
                        frame_id: None,
                        extraction_type: None,
                    },
                });
            }
        }

        if steps.is_empty() {
            return None;
        }

        let seq = Sequence {
            name: "main".to_string(),
            description: "Deterministic replay of learned steps".to_string(),
            required_variables: vec![],
            steps,
        };

        let wf = Workflow {
            id: format!("wf-{}-{}", slugify(goal), &self.correlation_id[..8]),
            name: goal.to_string(),
            description: goal.to_string(),
            version: "1.0".to_string(),
            last_updated: chrono::Utc::now().to_rfc3339(),
            browser_automation: BrowserAutomation {
                sequences: vec![seq],
            },
        };

        Some(wf)
    }

    fn selector_samples(&self, limit: usize) -> Vec<String> {
        let mut entries: Vec<&SelectorDescriptor> = self.selector_inventory.values().collect();
        entries.sort_by(|a, b| {
            self.selector_score(b)
                .cmp(&self.selector_score(a))
                .then_with(|| a.encoded_id.cmp(&b.encoded_id))
        });

        entries
            .into_iter()
            .take(limit)
            .map(|desc| {
                let label = desc
                    .name
                    .as_deref()
                    .or(desc.text.as_deref())
                    .unwrap_or("")
                    .trim();
                let actions = if desc.actions.is_empty() {
                    String::new()
                } else {
                    let joined = desc.actions.join("/");
                    format!(" actions: {}", joined)
                };
                let frame_hint = self.format_frame_hint(desc.frame_ordinal, desc.frame_id.as_ref());
                let attr_hint = self.format_attr_hint(&desc.attributes);
                if label.is_empty() {
                    format!(
                        "{} → {}{}{}{}",
                        desc.encoded_id, desc.css_selector, actions, frame_hint, attr_hint
                    )
                } else {
                    format!(
                        "{} → {} ({}){}{}{}",
                        desc.encoded_id, desc.css_selector, label, actions, frame_hint, attr_hint
                    )
                }
            })
            .collect()
    }

    fn lookup_descriptor(&self, token_or_css: &str) -> Option<&SelectorDescriptor> {
        if let Some(desc) = self.selector_inventory.get(token_or_css) {
            return Some(desc);
        }
        if let Some(token) = self.selector_lookup_css.get(token_or_css) {
            return self.selector_inventory.get(token);
        }
        if let Some(token) = self.selector_lookup_css.get(&token_or_css.to_lowercase()) {
            return self.selector_inventory.get(token);
        }
        None
    }

    fn build_target_spec(&self, token_or_css: &str) -> Result<TargetSpec, String> {
        let descriptor = self.lookup_descriptor(token_or_css).ok_or_else(|| {
            format!(
                "Selector '{}' is not in the current inventory.",
                token_or_css
            )
        })?;

        let mut target = TargetSpec::from_encoded_id(descriptor.encoded_id.clone());
        if !descriptor.css_selector.is_empty() {
            target.css = Some(descriptor.css_selector.clone());
        }
        target.frame_ordinal = descriptor.frame_ordinal;
        if let Some(role) = &descriptor.role {
            target.role_name = Some(role.clone());
        }
        if let Some(name) = &descriptor.name {
            target.text_near = Some(name.clone());
        } else if let Some(text) = &descriptor.text {
            target.text_near = Some(text.clone());
        }

        Ok(target)
    }

    fn selector_score(&self, desc: &SelectorDescriptor) -> i32 {
        let mut score = 0;
        if desc.actions.iter().any(|a| a.eq_ignore_ascii_case("click")) {
            score += 20;
        }
        if desc
            .actions
            .iter()
            .any(|a| a.eq_ignore_ascii_case("extract") || a.eq_ignore_ascii_case("navigate"))
        {
            score += 5;
        }
        if let Some(role) = &desc.role {
            if matches!(role.as_str(), "link" | "button") {
                score += 10;
            }
        }
        if let Some(text) = desc
            .name
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .or(desc.text.as_deref().filter(|t| !t.trim().is_empty()))
        {
            score += (text.len() as i32).min(40);
        }
        if desc.frame_ordinal.unwrap_or(0) == 0 {
            score += 5;
        }
        score
    }

    fn format_frame_hint(&self, frame_ordinal: Option<u32>, frame_id: Option<&String>) -> String {
        let id_hint = frame_id.map(|fid| {
            let truncated = safe_truncate_utf8(fid, 10);
            if fid.len() > truncated.len() {
                format!("{}…", truncated)
            } else {
                truncated.to_string()
            }
        });

        match (frame_ordinal, id_hint) {
            (Some(ord), Some(id)) => format!(" frame:{}@{}", ord, id),
            (Some(ord), None) => format!(" frame:{}", ord),
            (None, Some(id)) => format!(" frameId:{}", id),
            (None, None) => String::new(),
        }
    }

    fn format_attr_hint(&self, attributes: &HashMap<String, String>) -> String {
        if attributes.is_empty() {
            return String::new();
        }

        let mut pairs: Vec<String> = attributes
            .iter()
            .take(2)
            .map(|(k, v)| {
                let truncated = safe_truncate_utf8(v, 30);
                let sanitized = truncated.replace(['\n', '\r'], " ");
                let value = if v.len() > truncated.len() {
                    format!("{}…", sanitized)
                } else {
                    sanitized
                };
                format!("{}={}", k, value)
            })
            .collect();

        let suffix = if attributes.len() > 2 { ",…" } else { "" };
        if pairs.is_empty() {
            String::new()
        } else {
            format!(" attrs:[{}{}]", pairs.join(","), suffix)
        }
    }

    async fn run_step_with_target(
        &mut self,
        step: Step,
        target: &TargetSpec,
        context: &str,
    ) -> PlanResult<Value> {
        let envelope = self
            .broker_client
            .execute_step_with_target(&step, target)
            .await?;

        if envelope.success {
            // record successful step for workflow caching
            self.executed_steps.push(step.clone());
            Ok(envelope.result)
        } else {
            Err(PlanError::ExecutionError(envelope.error.unwrap_or_else(
                || format!("{} failed", context.to_string()),
            )))
        }
    }

    fn selector_too_generic(selector: &str) -> bool {
        let trimmed = selector.trim();
        if trimmed.len() <= 2 {
            return true;
        }
        if !trimmed.contains('#')
            && !trimmed.contains('.')
            && !trimmed.contains('[')
            && !trimmed.contains(':')
            && !trimmed.contains('>')
        {
            return true;
        }
        false
    }

    fn validate_action_targets(&self, action: &ActionCommand) -> Result<(), String> {
        match action.cmd.as_str() {
            "click" | "type" | "type_and_submit" => {
                let token = action.args.get(0).and_then(|s| s.as_str()).ok_or_else(|| {
                    format!("Action '{}' requires a selector argument", action.cmd)
                })?;

                if self.selector_inventory.is_empty() {
                    return Err(
                        "Selector inventory is empty; unable to validate targets".to_string()
                    );
                }
                let descriptor = match self.lookup_descriptor(token) {
                    Some(desc) => desc,
                    None => {
                        let samples = self.selector_samples(6);
                        let hint = if samples.is_empty() {
                            "No selectors cached yet; retry after the next page state.".to_string()
                        } else {
                            format!("Available selectors: {}", samples.join(", "))
                        };
                        if Self::selector_too_generic(token) {
                            return Err(format!(
                                "Selector '{}' is too generic or not recognized. {}",
                                token, hint
                            ));
                        }
                        return Err(format!(
                            "Selector '{}' is not in the current inventory. {}",
                            token, hint
                        ));
                    }
                };

                // Ensure the action is supported by the descriptor inventory
                if action.cmd == "click"
                    && !descriptor
                        .actions
                        .iter()
                        .any(|a| a.eq_ignore_ascii_case("click"))
                {
                    return Err(format!(
                        "Target '{}' does not support click. Allowed actions: {}",
                        token,
                        descriptor.actions.join(", ")
                    ));
                }

                if (action.cmd == "type" || action.cmd == "type_and_submit")
                    && !descriptor.actions.iter().any(|a| {
                        a.eq_ignore_ascii_case("type") || a.eq_ignore_ascii_case("setValue")
                    })
                {
                    return Err(format!(
                        "Target '{}' is not writable. Allowed actions: {}",
                        token,
                        descriptor.actions.join(", ")
                    ));
                }

                // Build target spec (ensures encoded id + css mapping exists)
                if let Err(err) = self.build_target_spec(token) {
                    let samples = self.selector_samples(6);
                    let hint = if samples.is_empty() {
                        "No selectors cached yet; retry after the next page state.".to_string()
                    } else {
                        format!("Available selectors: {}", samples.join(", "))
                    };
                    return Err(format!("{} {}", err, hint));
                }

                if action.cmd.as_str() == "type_and_submit" {
                    if action.args.get(1).and_then(|s| s.as_str()).is_none() {
                        return Err(
                            "type_and_submit requires text as the second argument".to_string()
                        );
                    }
                } else if action.cmd.as_str() == "type" {
                    if action.args.get(1).and_then(|s| s.as_str()).is_none() {
                        return Err("type requires text as the second argument".to_string());
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }
    pub fn new(llm_client: LLMClient, broker_client: BrokerClient) -> Self {
        Self::new_with_options(llm_client, broker_client, LLMAutonomousOptions::default())
    }

    pub fn new_with_options(
        llm_client: LLMClient,
        broker_client: BrokerClient,
        options: LLMAutonomousOptions,
    ) -> Self {
        // Load the autonomous prompt
        let system_prompt = include_str!("prompts/autonomous.md").to_string();
        let correlation_id = Uuid::new_v4().to_string();

        // Initialize tool-only LLM client only when the selected provider is OpenAI.
        // Otherwise, we can end up accidentally calling OpenAI even when the host selected a CLI provider.
        let tool_llm_client = match std::env::var("LLM_PROVIDER")
            .ok()
            .as_deref()
            .and_then(crate::llm_provider::ProviderType::from_str)
        {
            Some(crate::llm_provider::ProviderType::OpenAI) => std::env::var("OPENAI_API_KEY")
                .ok()
                .map(|api_key| ToolOnlyLLMClient::new(api_key, correlation_id.clone())),
            _ => None,
        };

        Self {
            llm_client,
            broker_client,
            system_prompt,
            conversation_history: Vec::new(),
            current_url: None,
            planner_state: PlannerState::new(correlation_id.clone()),
            policy_validator: PolicyValidator::new(correlation_id.clone()),
            tool_llm_client,
            correlation_id,
            last_dom_hash: None,
            scrolls_since_last_extract: 0,
            last_scroll_direction: None,
            selector_inventory: HashMap::new(),
            selector_lookup_css: HashMap::new(),
            executed_steps: Vec::new(),
            recorded_steps: Vec::new(),
            last_auto_container_selector: None,
            last_auto_item_selector: None,
            options,
        }
    }

    fn record_step(&mut self, step: &Step) {
        self.executed_steps.push(step.clone());
    }

    fn record_step_with_result(
        &mut self,
        step: Step,
        started_at_ms: u64,
        finished_at_ms: u64,
        pre_url: Option<String>,
        pre_dom_hash: Option<String>,
        resp: &Value,
    ) {
        let post_url = resp
            .get("current_url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.broker_client.get_current_url());

        let post_dom_hash = resp
            .get("dom_hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                self.broker_client
                    .get_current_dom_snapshot()
                    .map(|s| s.hash.clone())
            });

        let success = resp.get("success").and_then(|v| v.as_bool());
        let error_code = resp
            .get("error_code")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let error = resp
            .get("error_msg")
            .or_else(|| resp.get("error"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        self.executed_steps.push(step.clone());
        self.recorded_steps.push(RecordedStep {
            step,
            started_at_ms,
            finished_at_ms,
            pre_url,
            post_url,
            pre_dom_hash,
            post_dom_hash,
            success,
            error_code,
            error,
        });
    }

    async fn execute_step_record(&mut self, step: Step) -> PlanResult<Value> {
        let started_at_ms = now_ms();
        let pre_url = self.broker_client.get_current_url();
        let pre_dom_hash = self
            .broker_client
            .get_current_dom_snapshot()
            .map(|s| s.hash.clone());

        let res = self.broker_client.execute_step(&step).await?;
        let finished_at_ms = now_ms();

        self.record_step_with_result(
            step,
            started_at_ms,
            finished_at_ms,
            pre_url,
            pre_dom_hash,
            &res,
        );
        Ok(res)
    }

    /// Execute an instruction autonomously using LLM-driven browser control
    pub async fn execute_autonomous(
        &mut self,
        request: LLMAutonomousRequest,
    ) -> PlanResult<LLMAutonomousResponse> {
        info!("Starting LLM autonomous execution: {}", request.instruction);

        // Reset per-run state (planner may be reused by embedded hosts)
        self.executed_steps.clear();
        self.recorded_steps.clear();

        let max_steps = request.max_steps.unwrap_or(30);
        let mut steps_executed = 0;
        let mut last_error: Option<String> = None;
        let mut extracted_data: Option<Value> = None;
        let mut consecutive_failures = 0;
        let mut consecutive_waits = 0;
        let mut should_finish = false; // signal to exit the outer loop cleanly

        // Optional deterministic start URL for testing / embedding use-cases.
        // If provided, we always respect it (and skip search bootstraps).
        let has_start_url = request
            .start_url
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if let Some(url) = request.start_url.as_ref().filter(|s| !s.trim().is_empty()) {
            let nav = Step {
                id: "nav_start_url".to_string(),
                name: format!("Navigate to {}", url),
                kind: StepKind::NavigateToUrl {
                    url: url.to_string(),
                    wait: Some("domcontentloaded".to_string()),
                },
            };
            let _ = self.execute_step_record(nav).await;
            self.current_url = Some(url.to_string());
            self.planner_state
                .update_context("start_url".to_string(), url.to_string());
        }

        // Heuristic policy hint: for "top N ..." tasks that primarily ask for a list of results/links,
        // prefer extracting from the results page before navigating away.
        //
        // Important: do NOT block click-heavy tasks (e.g. "click the top 3 results") because
        // the user explicitly wants per-item navigation.
        let lower_instr = request.instruction.to_lowercase();
        let has_top_n =
            Self::parse_top_n(&request.instruction).is_some() || lower_instr.contains("top ");
        let explicitly_navigates = lower_instr.contains("click")
            || lower_instr.contains("open")
            || lower_instr.contains("visit")
            || lower_instr.contains("go to each")
            || lower_instr.contains("for each");
        let listy_intent = lower_instr.contains("result")
            || lower_instr.contains("link")
            || lower_instr.contains("items")
            || lower_instr.contains("list");
        let prefer_extract_first = has_top_n && listy_intent && !explicitly_navigates;
        if prefer_extract_first {
            self.planner_state
                .update_context("prefer_extract_first".to_string(), "true".to_string());
        }

        // Fast macro: "first comment of top N posts of <site>" → run deterministic generic flow
        if self.options.enable_macros && Self::looks_like_first_comment_task(&request.instruction) {
            if let Ok(Some(data)) = self.try_first_comment_top_n(&request.instruction).await {
                return Ok(LLMAutonomousResponse {
                    success: true,
                    steps_executed: 0,
                    result: None,
                    error: None,
                    extracted_data: Some(json!(data)),
                });
            }
        }

        // Fast macro: top-N links → extract comments/reviews (generic, list-detection based).
        if self.options.enable_macros
            && has_start_url
            && Self::looks_like_top_links_comments_task(&request.instruction)
        {
            if let Ok(Some(data)) = self
                .try_extract_comments_from_top_links(&request.instruction)
                .await
            {
                return Ok(LLMAutonomousResponse {
                    success: true,
                    steps_executed,
                    result: None,
                    error: None,
                    extracted_data: Some(data),
                });
            }
        }

        // Fast-bootstrap: if instruction looks like a simple search (Google/Bing/DDG), navigate immediately
        if self.options.enable_macros && !has_start_url {
            if let Some((engine, _query)) = Self::detect_search_intent(&request.instruction) {
                let url = match engine.as_str() {
                    "google" => "https://www.google.com".to_string(),
                    "bing" => "https://www.bing.com".to_string(),
                    "duckduckgo" => "https://duckduckgo.com".to_string(),
                    _ => "https://www.google.com".to_string(),
                };
                // Navigate
                let nav = Step {
                    id: "nav_bootstrap".to_string(),
                    name: format!("Navigate to {}", url),
                    kind: StepKind::NavigateToUrl {
                        url: url.clone(),
                        wait: Some("domcontentloaded".to_string()),
                    },
                };
                let _ = self.execute_step_record(nav).await;
                self.current_url = Some(url.clone());
                self.planner_state
                    .transition(crate::planner::PlannerMode::Search);
                // If we have a query, attempt deterministic search + extraction and return early
                if let Some((_eng, query)) = Self::detect_search_intent(&request.instruction) {
                    if !query.is_empty() {
                        let top = Self::parse_top_n(&request.instruction).unwrap_or(10);
                        match self.fast_search_extract(&url, &query, top).await {
                            Ok(Some(items)) => {
                                // Mark extraction complete for policy gating
                                self.planner_state
                                    .update_context("extracted".to_string(), "true".to_string());
                                return Ok(LLMAutonomousResponse {
                                    success: true,
                                    steps_executed,
                                    result: None,
                                    error: None,
                                    extracted_data: Some(items),
                                });
                            }
                            Ok(None) => { /* fall through to LLM loop */ }
                            Err(e) => {
                                warn!("Fast search-extract failed: {}", e);
                            }
                        }
                    }
                }
            }
        }

        // Fast-bootstrap: if instruction mentions Amazon search, navigate and try deterministic flow
        if self.options.enable_macros
            && !has_start_url
            && Self::detect_amazon_intent(&request.instruction)
        {
            let url = "https://www.amazon.com".to_string();
            let nav = Step {
                id: "nav_amazon".to_string(),
                name: "Navigate to Amazon".to_string(),
                kind: StepKind::NavigateToUrl {
                    url: url.clone(),
                    wait: Some("domcontentloaded".to_string()),
                },
            };
            let _ = self.execute_step_record(nav).await;
            self.current_url = Some(url.clone());
            self.planner_state
                .transition(crate::planner::PlannerMode::Search);

            if let Some(query) = Self::extract_search_query(&request.instruction) {
                let top = Self::parse_top_n(&request.instruction).unwrap_or(10);
                match self.fast_search_extract(&url, &query, top).await {
                    // If this is a multi-page "open top N and extract comments/reviews" task,
                    // don't exit early after extracting search results. We want to continue into the
                    // macro/LLM loop to visit pages and extract the requested content.
                    Ok(Some(items)) => {
                        if Self::looks_like_top_links_comments_task(&request.instruction) {
                            self.planner_state
                                .transition(crate::planner::PlannerMode::Results);
                            // Save bootstrap results as a hint for later debugging (non-authoritative).
                            self.planner_state.update_context(
                                "bootstrap_results".to_string(),
                                serde_json::to_string(&items).unwrap_or_default(),
                            );
                        } else {
                            return Ok(LLMAutonomousResponse {
                                success: true,
                                steps_executed,
                                result: None,
                                error: None,
                                extracted_data: Some(items),
                            });
                        }
                    }
                    Ok(None) => {
                        // Still proceed into LLM loop; it may recover.
                    }
                    Err(e) => {
                        warn!("Fast amazon search-extract failed: {}", e);
                    }
                }
            }
        }

        // Initialize conversation with system prompt and task
        self.conversation_history.clear();
        self.conversation_history.push(json!({
            "role": "system",
            "content": self.system_prompt
        }));

        let start_msg = if let Some(url) = self.current_url.as_ref() {
            format!(
                "Task: {}\n\nYou are starting on: {}\n\nAnalyze the task and choose the next best action.",
                request.instruction, url
            )
        } else {
            format!(
                "Task: {}\n\nYou are starting with a blank browser tab. Analyze the task and navigate to the appropriate website to complete it.",
                request.instruction
            )
        };

        // Let the LLM decide where to navigate based on the task
        // No hardcoded navigation - fully autonomous
        self.conversation_history.push(json!({
            "role": "user",
            "content": start_msg
        }));

        // Main execution loop
        while steps_executed < max_steps {
            // If FSM already marked task complete, exit gracefully
            if self.planner_state.mode == PlannerMode::Complete {
                break;
            }
            // Cheap deterministic recovery before we spend a model turn.
            // Do not count this against the LLM step budget.
            let _ = self.auto_handle_interstitials().await;
            steps_executed += 1;

            // Fast macro: top-N links → extract comments/reviews (generic, list-detection based).
            // This can run after we arrive on a list/results page (e.g., search results).
            if self.options.enable_macros
                && Self::looks_like_top_links_comments_task(&request.instruction)
                && self.planner_state.mode == PlannerMode::Results
            {
                // Run once we have a real page loaded; the macro itself will return None if it
                // can't find a repeated list of outbound links.
                let has_http_url = self
                    .current_url
                    .as_deref()
                    .map(|u| u.starts_with("http://") || u.starts_with("https://"))
                    .unwrap_or(false);
                if has_http_url {
                    if let Ok(Some(data)) = self
                        .try_extract_comments_from_top_links(&request.instruction)
                        .await
                    {
                        extracted_data = Some(data);
                        should_finish = true;
                        break;
                    }
                }
            }

            // Ultra-fast short-circuit: if we're on a search results page, extract once and finish.
            // Only for "search-only" tasks (not multi-page "open top N" or review/comment tasks).
            if self.options.enable_macros
                && self.planner_state.mode == PlannerMode::Results
                && !Self::looks_like_top_links_comments_task(&request.instruction)
            {
                let already = self
                    .planner_state
                    .context
                    .get("extracted")
                    .map(|s| s.as_str())
                    == Some("true");
                if !already {
                    let top = Self::parse_top_n(&request.instruction).unwrap_or(10);
                    // Prefer AX-based extraction for Google; else fall back to structured search extractor
                    let mut done = false;
                    if let Some(url) = &self.current_url {
                        if url.contains("google.") {
                            match self.ax_extract_google_results(top).await {
                                Ok(Some(items)) => {
                                    extracted_data = Some(items.clone());
                                    self.planner_state.update_context(
                                        "extracted".to_string(),
                                        "true".to_string(),
                                    );
                                    should_finish = true;
                                    done = true;
                                }
                                _ => {}
                            }
                        }
                    }
                    if !done {
                        let enhanced = json!({
                            "type": "extract_structured_data",
                            "extraction_type": "search_results",
                            "fields": []
                        });
                        if let Ok(resp) = self.broker_client.execute_raw_step(enhanced).await {
                            if let Some(arr) = resp.get("result").and_then(|v| v.as_array()) {
                                if !arr.is_empty() {
                                    let mut v = Value::Array(arr.clone());
                                    if let Some(a) = v.as_array_mut() {
                                        if a.len() > top {
                                            a.truncate(top);
                                        }
                                    }
                                    extracted_data = Some(v);
                                    self.planner_state.update_context(
                                        "extracted".to_string(),
                                        "true".to_string(),
                                    );
                                    should_finish = true;
                                }
                            }
                        }
                    }
                    if should_finish {
                        break;
                    }
                }
            }

            // Get current DOM state
            match self.get_page_state().await {
                Ok(dom_state) => {
                    // Add DOM state to conversation
                    self.conversation_history.push(json!({
                        "role": "user",
                        "content": format!("Current page state:\n\n{}", dom_state)
                    }));
                }
                Err(e) => {
                    warn!("Failed to get page state: {}", e);
                    // Continue anyway, LLM might still be able to decide what to do
                }
            }

            // Get next action from LLM
            let llm_response = self.get_llm_action().await?;

            match self.parse_llm_response(&llm_response) {
                Ok(llm_actions) => {
                    // Shortcut: if the first action is a navigate, execute it immediately to allow mode change
                    if let Some(first) = llm_actions.get(0) {
                        if first.action.cmd == "navigate" {
                            // Pre-transition for navigation target
                            if let Some(url) = first.action.args.get(0).and_then(|u| u.as_str()) {
                                let next_mode = self.planner_state.infer_next_mode(url, "");
                                if next_mode != self.planner_state.mode {
                                    info!(
                                        "[{}] Pre-transitioning state for navigation: {:?} -> {:?}",
                                        self.correlation_id, self.planner_state.mode, next_mode
                                    );
                                    self.planner_state.transition(next_mode);
                                }
                                self.current_url = Some(url.to_string());
                            }
                            // Execute navigate
                            match self.execute_action(&first.action).await {
                                Ok(_res) => {
                                    // Log success into conversation and continue to next turn
                                    self.conversation_history.push(json!({
                                        "role": "assistant",
                                        "content": serde_json::to_string(&first).unwrap_or_default()
                                    }));
                                    self.conversation_history.push(json!({
                                        "role": "user",
                                        "content": "Action result: success"
                                    }));
                                    continue; // proceed to next loop with updated mode
                                }
                                Err(e) => {
                                    self.conversation_history.push(json!({
                                        "role": "assistant",
                                        "content": serde_json::to_string(&first).unwrap_or_default()
                                    }));
                                    self.conversation_history.push(json!({
                                        "role": "user",
                                        "content": format!("Action failed: {}", e)
                                    }));
                                    continue;
                                }
                            }
                        }
                    }

                    const SUPPORTED_CMDS: &[&str] = &[
                        "navigate",
                        "click",
                        "type",
                        "type_and_submit",
                        "press",
                        "press_key",
                        "scroll",
                        "wait",
                        "extract",
                        "extract_auto_list",
                        "complete",
                        "error",
                    ];

                    if let Some(unsupported) = llm_actions
                        .iter()
                        .find(|a| !SUPPORTED_CMDS.contains(&a.action.cmd.as_str()))
                    {
                        let message = format!(
                            "Unsupported action '{}'. Allowed actions: {}",
                            unsupported.action.cmd,
                            SUPPORTED_CMDS.join(", ")
                        );
                        self.conversation_history.push(json!({
                            "role": "user",
                            "content": format!("ACTION REJECTED: {}", message)
                        }));
                        last_error = Some(message);
                        continue;
                    }

                    // Validate actions as a batch first
                    if let Some(err) = llm_actions
                        .iter()
                        .find_map(|a| self.validate_action_targets(&a.action).err())
                    {
                        let samples = self.selector_samples(6);
                        let guidance = if samples.is_empty() {
                            err.clone()
                        } else {
                            format!(
                                "{} Available selectors include: {}",
                                err,
                                samples.join(", ")
                            )
                        };
                        self.conversation_history.push(json!({
                            "role": "user",
                            "content": format!(
                                "ACTION REJECTED: {}",
                                guidance
                            )
                        }));
                        last_error = Some(err);
                        continue;
                    }

                    let action_jsons: Vec<Value> = llm_actions
                        .iter()
                        .map(|a| {
                            json!({
                                "cmd": a.action.cmd,
                                "args": a.action.args
                            })
                        })
                        .collect();

                    if let Err(e) = self
                        .policy_validator
                        .validate_batch(&action_jsons, &self.planner_state)
                    {
                        error!("[{}] Policy violation: {}", self.correlation_id, e);
                        last_error = Some(format!("Policy violation: {}", e));
                        consecutive_failures += 1;

                        // If too many consecutive failures, reset to Browse mode
                        if consecutive_failures >= 3 {
                            warn!("Too many consecutive failures, resetting to Browse mode");
                            self.planner_state.transition(PlannerMode::Browse);
                            consecutive_failures = 0;
                        }

                        // Add error to conversation for LLM to learn
                        self.conversation_history.push(json!({
                            "role": "user",
                            "content": format!("ERROR: {}. Current mode is {:?}. Available actions: {:?}", 
                                e, self.planner_state.mode, self.planner_state.get_allowed_tools())
                        }));

                        continue; // Skip this batch and try again
                    }

                    // Reset consecutive failures on success
                    consecutive_failures = 0;

                    // Enforce pre-navigation gating: before any page is loaded, only allow navigate/wait
                    let mut filtered: Vec<LLMAction> = Vec::new();
                    for a in llm_actions.into_iter() {
                        if self.current_url.is_none()
                            || self.current_url.as_deref() == Some("unknown")
                        {
                            if a.action.cmd != "navigate" && a.action.cmd != "wait" {
                                // Provide corrective feedback and skip
                                self.conversation_history.push(json!({
                                    "role": "user",
                                    "content": "ERROR: Do not propose element actions before navigation. Return a single 'navigate' action first."
                                }));
                                continue;
                            }
                        }
                        filtered.push(a);
                    }

                    // Execute actions (normally single-step)
                    for llm_action in filtered {
                        info!("LLM thought: {}", llm_action.thought);
                        info!(
                            "LLM action: {} {:?}",
                            llm_action.action.cmd, llm_action.action.args
                        );

                        // Guardrail: break wait-loops when typing is allowed
                        if llm_action.action.cmd == "wait" {
                            consecutive_waits += 1;
                            if self.planner_state.mode != PlannerMode::Bootstrap
                                && consecutive_waits >= 3
                            {
                                warn!(
                                    "[{}] Detected repeated waits in {:?} mode; nudging LLM to act",
                                    self.correlation_id, self.planner_state.mode
                                );
                                // Inject corrective guidance and skip executing this wait
                                self.conversation_history.push(json!({
                                    "role": "user",
                                    "content": "Avoid repeated 'wait'. In this mode you may type in the search box and submit with Enter. Return a single 'type' followed by 'press_key' action."
                                }));
                                break; // Break inner loop to get a new LLM turn
                            }
                        } else {
                            consecutive_waits = 0;
                        }

                        // Check for completion
                        if llm_action.action.cmd == "complete" {
                            // Guardrail: some providers occasionally "declare victory" without
                            // returning any structured result. Treat that as a format/behavior
                            // error and ask for the next action instead of terminating early.
                            let has_data = llm_action
                                .result
                                .as_ref()
                                .and_then(|r| r.get("data"))
                                .is_some()
                                || extracted_data.is_some();
                            if !has_data && self.planner_state.mode != PlannerMode::Complete {
                                warn!(
                                    "[{}] LLM attempted to complete without result data; rejecting",
                                    self.correlation_id
                                );
                                self.conversation_history.push(json!({
                                    "role": "user",
                                    "content": "FORMAT/LOGIC ERROR: Do not call `complete` unless the task is actually done and you include `result.data` with the requested output. Choose the next best action instead."
                                }));
                                break; // Get a new LLM turn
                            }

                            info!("Task completed by LLM");
                            // Mark FSM complete and capture any provided data
                            self.planner_state.transition(PlannerMode::Complete);
                            if let Some(result) = llm_action.result {
                                extracted_data = result.get("data").cloned();
                            }
                            should_finish = true;
                            break; // break inner loop; outer loop will exit below
                        }

                        // Check for error
                        if llm_action.action.cmd == "error" {
                            let error_msg = llm_action
                                .action
                                .args
                                .get(0)
                                .and_then(|v| v.as_str())
                                .unwrap_or("Unknown error");
                            warn!("LLM reported error: {}", error_msg);
                            last_error = Some(error_msg.to_string());
                            break;
                        }

                        // Pre-update FSM state for navigation actions BEFORE execution
                        // This allows the next LLM call to know we're heading to a search page
                        if llm_action.action.cmd == "navigate" {
                            if let Some(url) =
                                llm_action.action.args.get(0).and_then(|u| u.as_str())
                            {
                                // Immediately transition state based on target URL
                                // This fixes the FSM issue where we're stuck in Bootstrap when navigating to Google
                                let next_mode = self.planner_state.infer_next_mode(url, "");
                                if next_mode != self.planner_state.mode {
                                    info!(
                                        "[{}] Pre-transitioning state for navigation: {:?} -> {:?}",
                                        self.correlation_id, self.planner_state.mode, next_mode
                                    );
                                    self.planner_state.transition(next_mode);
                                }
                                self.current_url = Some(url.to_string());
                            }
                        }

                        // Execute the action
                        match self.execute_action(&llm_action.action).await {
                            Ok(result) => {
                                // Update FSM state based on action results
                                match llm_action.action.cmd.as_str() {
                                    "navigate" => {
                                        // State already transitioned above based on target URL.
                                        // Skip extra DOM fetch here to reduce latency; next loop turn will refresh state.
                                    }
                                    "type" => {
                                        // Only transition to Search if we're in Bootstrap or Browse mode
                                        match self.planner_state.mode {
                                            PlannerMode::Bootstrap | PlannerMode::Browse => {
                                                self.planner_state.transition(PlannerMode::Search);
                                            }
                                            _ => {} // Stay in current mode (Form, Search, etc.)
                                        }
                                        // Remember the most recent typed query in Search mode so we can
                                        // verify that a subsequent Enter press actually submitted.
                                        if self.planner_state.mode == PlannerMode::Search {
                                            if let Some(tok) = llm_action
                                                .action
                                                .args
                                                .get(0)
                                                .and_then(|v| v.as_str())
                                            {
                                                self.planner_state.update_context(
                                                    "pending_search_token".to_string(),
                                                    tok.to_string(),
                                                );
                                            }
                                            if let Some(q) = llm_action
                                                .action
                                                .args
                                                .get(1)
                                                .and_then(|v| v.as_str())
                                            {
                                                self.planner_state.update_context(
                                                    "pending_search_query".to_string(),
                                                    q.to_string(),
                                                );
                                            }
                                        }
                                    }
                                    "press_key" | "press" => {
                                        if let Some(key) =
                                            llm_action.action.args.get(0).and_then(|k| k.as_str())
                                        {
                                            if key == "Enter"
                                                && self.planner_state.mode == PlannerMode::Search
                                            {
                                                // Verify that Enter likely submitted the search before transitioning.
                                                let query = self
                                                    .planner_state
                                                    .context
                                                    .get("pending_search_query")
                                                    .cloned()
                                                    .unwrap_or_default();
                                                let normalize = |s: &str| -> String {
                                                    s.to_lowercase()
                                                        .chars()
                                                        .map(|c| {
                                                            if c.is_ascii_alphanumeric() {
                                                                c
                                                            } else {
                                                                ' '
                                                            }
                                                        })
                                                        .collect::<String>()
                                                };
                                                let query_terms: Vec<String> = normalize(&query)
                                                    .split_whitespace()
                                                    .filter(|w| w.len() >= 3)
                                                    .map(|w| w.to_string())
                                                    .collect();

                                                let mut submitted = false;
                                                if !query_terms.is_empty() {
                                                    let snap = self
                                                        .broker_client
                                                        .get_dom_snapshot()
                                                        .await
                                                        .unwrap_or(json!({}));
                                                    let meta = snap
                                                        .get("dom_snapshot")
                                                        .and_then(|d| d.get("metadata"));
                                                    let title = meta
                                                        .and_then(|m| m.get("title"))
                                                        .and_then(|t| t.as_str())
                                                        .unwrap_or("");
                                                    let url_now = meta
                                                        .and_then(|m| m.get("url"))
                                                        .and_then(|u| u.as_str())
                                                        .unwrap_or("");
                                                    if !url_now.is_empty() {
                                                        self.current_url =
                                                            Some(url_now.to_string());
                                                    }
                                                    let hay_url = normalize(
                                                        self.current_url
                                                            .as_deref()
                                                            .unwrap_or(url_now),
                                                    );
                                                    let hay_title = normalize(title);
                                                    for term in &query_terms {
                                                        if hay_url.contains(term)
                                                            || hay_title.contains(term)
                                                        {
                                                            submitted = true;
                                                            break;
                                                        }
                                                    }
                                                }

                                                if !submitted && !query_terms.is_empty() {
                                                    if let Ok(auto) = self
                                                        .broker_client
                                                        .detect_auto_list(Some(json!({
                                                            "purpose": "results",
                                                            "maxCandidates": 3
                                                        })))
                                                        .await
                                                    {
                                                        let auto_r =
                                                            auto.get("result").unwrap_or(&auto);
                                                        if let Some(items) = auto_r
                                                            .get("items")
                                                            .and_then(|v| v.as_array())
                                                        {
                                                            let mut any_term = false;
                                                            for it in items.iter().take(10) {
                                                                let text = it
                                                                    .get("text")
                                                                    .and_then(|v| v.as_str())
                                                                    .unwrap_or("");
                                                                let lt = text.to_lowercase();
                                                                if query_terms
                                                                    .iter()
                                                                    .any(|t| lt.contains(t))
                                                                {
                                                                    any_term = true;
                                                                    break;
                                                                }
                                                            }
                                                            if items.len() >= 3 && any_term {
                                                                submitted = true;
                                                            }
                                                        }
                                                    }
                                                }

                                                if submitted {
                                                    self.planner_state
                                                        .transition(PlannerMode::Results);
                                                } else {
                                                    // Try one deterministic fallback: click a likely "Search" submit button.
                                                    if self
                                                        .try_click_search_submit()
                                                        .await
                                                        .unwrap_or(false)
                                                    {
                                                        let _ = self
                                                            .ensure_dom_settled(6000, 300)
                                                            .await;
                                                        let snap = self
                                                            .broker_client
                                                            .get_dom_snapshot()
                                                            .await
                                                            .unwrap_or(json!({}));
                                                        let meta = snap
                                                            .get("dom_snapshot")
                                                            .and_then(|d| d.get("metadata"));
                                                        let title = meta
                                                            .and_then(|m| m.get("title"))
                                                            .and_then(|t| t.as_str())
                                                            .unwrap_or("");
                                                        let url_now = meta
                                                            .and_then(|m| m.get("url"))
                                                            .and_then(|u| u.as_str())
                                                            .unwrap_or("");
                                                        if !url_now.is_empty() {
                                                            self.current_url =
                                                                Some(url_now.to_string());
                                                        }
                                                        let hay_url = normalize(
                                                            self.current_url
                                                                .as_deref()
                                                                .unwrap_or(url_now),
                                                        );
                                                        let hay_title = normalize(title);
                                                        let mut has_hint = false;
                                                        for term in &query_terms {
                                                            if hay_url.contains(term)
                                                                || hay_title.contains(term)
                                                            {
                                                                has_hint = true;
                                                                break;
                                                            }
                                                        }
                                                        if has_hint {
                                                            self.planner_state
                                                                .transition(PlannerMode::Results);
                                                        }
                                                    }

                                                    if self.planner_state.mode
                                                        == PlannerMode::Search
                                                    {
                                                        self.conversation_history.push(json!({
                                                            "role": "user",
                                                            "content": "NOTE: Pressing Enter did not appear to submit the search. Try clicking the visible search submit button, or use 'type_and_submit' on the search box."
                                                        }));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    "extract" => {
                                        // `extract` returns structured data but does NOT imply completion.
                                        // The agent must explicitly call `complete` when finished.
                                        if let Some(results) =
                                            result.get("results").and_then(|r| r.as_array())
                                        {
                                            if !results.is_empty() {
                                                extracted_data = Some(serde_json::json!(results));
                                                self.planner_state.update_context(
                                                    "extracted".to_string(),
                                                    "true".to_string(),
                                                );
                                            }
                                        } else if result.is_array()
                                            && !result.as_array().unwrap().is_empty()
                                        {
                                            extracted_data = Some(result.clone());
                                            self.planner_state.update_context(
                                                "extracted".to_string(),
                                                "true".to_string(),
                                            );
                                        }
                                    }
                                    "extract_auto_list" => {
                                        // `extract_auto_list` returns a list of items (usually objects).
                                        // Mark it as an extraction attempt so policy gates (prefer_extract_first)
                                        // can allow subsequent clicks.
                                        if let Some(results) =
                                            result.get("results").and_then(|r| r.as_array())
                                        {
                                            if !results.is_empty() {
                                                // Keep as a hint for downstream runs; not final output.
                                                extracted_data = Some(serde_json::json!(results));
                                                self.planner_state.update_context(
                                                    "extracted".to_string(),
                                                    "true".to_string(),
                                                );
                                            } else {
                                                self.planner_state.update_context(
                                                    "extract_attempted".to_string(),
                                                    "true".to_string(),
                                                );
                                            }
                                        } else {
                                            // Unknown shape, but we did attempt.
                                            self.planner_state.update_context(
                                                "extract_attempted".to_string(),
                                                "true".to_string(),
                                            );
                                        }
                                    }
                                    _ => {}
                                }

                                // Store extraction results if this was an extract-like command (normalize to array for CLI)
                                if llm_action.action.cmd == "extract"
                                    || llm_action.action.cmd == "extract_auto_list"
                                {
                                    if let Some(results) =
                                        result.get("results").and_then(|r| r.as_array())
                                    {
                                        if !results.is_empty() {
                                            extracted_data = Some(serde_json::json!(results));
                                            self.planner_state.update_context(
                                                "extracted".to_string(),
                                                "true".to_string(),
                                            );
                                        } else {
                                            self.planner_state.update_context(
                                                "extract_attempted".to_string(),
                                                "true".to_string(),
                                            );
                                        }
                                    } else if result.is_array() {
                                        let arr = result.as_array().unwrap();
                                        // Normalize shape: if first element is the results array and following elements are debug (e.g., dom_snapshot), pick index 0 only
                                        if let Some(first) = arr.get(0) {
                                            if first.is_array() {
                                                extracted_data = Some(first.clone());
                                            } else {
                                                extracted_data = Some(result.clone());
                                            }
                                        } else {
                                            extracted_data = Some(result.clone());
                                        }
                                        self.planner_state.update_context(
                                            "extracted".to_string(),
                                            "true".to_string(),
                                        );
                                    } else {
                                        self.planner_state.update_context(
                                            "extract_attempted".to_string(),
                                            "true".to_string(),
                                        );
                                    }
                                }

                                // Add successful result to conversation
                                self.conversation_history.push(json!({
                                "role": "assistant",
                                "content": serde_json::to_string(&llm_action).unwrap_or_default()
                            }));

                                // Include extraction results in the response if available
                                let response_content = if (llm_action.action.cmd == "extract"
                                    || llm_action.action.cmd == "extract_auto_list")
                                    && result.get("results").is_some()
                                {
                                    format!(
                                        "Action result: success\nExtracted data: {}",
                                        serde_json::to_string_pretty(&result).unwrap_or_default()
                                    )
                                } else {
                                    format!("Action result: success")
                                };

                                self.conversation_history.push(json!({
                                    "role": "user",
                                    "content": response_content
                                }));
                            }
                            Err(e) => {
                                // Add error to conversation
                                self.conversation_history.push(json!({
                                "role": "assistant",
                                "content": serde_json::to_string(&llm_action).unwrap_or_default()
                            }));

                                self.conversation_history.push(json!({
                                    "role": "user",
                                    "content": format!("Action failed: {}", e)
                                }));

                                // Let LLM decide how to handle the error
                            }
                        }
                    } // End of for loop

                    // If we reached completion in the inner loop, exit outer loop cleanly
                    if should_finish || self.planner_state.mode == PlannerMode::Complete {
                        break;
                    }
                }
                Err(e) => {
                    error!("Failed to parse LLM response: {}", e);
                    last_error = Some(format!("LLM parse error: {}", e));
                    break;
                }
            }
        }

        // If we have extracted data, treat as success regardless of prior recoverable errors
        let success = extracted_data.is_some() || last_error.is_none();
        Ok(LLMAutonomousResponse {
            success,
            steps_executed,
            result: extracted_data.clone(),
            error: if success { None } else { last_error },
            extracted_data,
        })
    }

    /// reference-style schema extraction over AX summary (no CSS selectors)
    async fn ax_extract_google_results(&mut self, top: usize) -> PlanResult<Option<Value>> {
        // Get AX text and id->url map from the extension (top frame only)
        let ax = self.broker_client.get_ax_tree(false, 400).await?;
        let text = ax
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let id_url_map = ax.get("id_url_map").cloned().unwrap_or(json!({}));

        if text.is_empty() {
            return Ok(None);
        }

        // Build an instruction and a strict JSON schema for the LLM
        // The LLM must output: { "items": [ { "title": string, "url_id": number, "snippet": string } ] }
        let system = r#"
You are an information extraction engine.
Given an Accessibility (AX) summary of a web page, extract an array of organic search results.
Rules:
- Use only the AX summary content; do NOT invent URLs.
- Each result must reference the link by its numeric node id (url_id).
- Exclude non-organic modules like Top stories, Videos, People also ask, Ads.
- Output strictly valid JSON with shape:
  { "items": [ { "title": string, "url_id": number, "snippet": string } ] }
- Return at most the requested number of items.
"#;

        let user = format!(
            "Extract the top {} organic search results from the AX summary.\nAX Summary:\n{}",
            top, text
        );

        // Ask the LLM to produce JSON
        let messages = vec![
            json!({"role": "system", "content": system}),
            json!({"role": "user", "content": user}),
        ];

        let parsed = match self.llm_client.chat_json(messages, Some(0.0)).await {
            Ok(v) => v,
            Err(e) => {
                warn!("AX schema extraction LLM error: {}", e);
                // Mark attempt to allow policy to proceed with fallback
                self.planner_state
                    .update_context("ax_attempted".to_string(), "true".to_string());
                return Ok(None);
            }
        };

        let items = parsed
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if items.is_empty() {
            self.planner_state
                .update_context("ax_attempted".to_string(), "true".to_string());
            return Ok(None);
        }

        // Map url_id -> url
        let mut out: Vec<Value> = Vec::new();
        for item in items.into_iter().take(top) {
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let snippet = item
                .get("snippet")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let url_id = item
                .get("url_id")
                .and_then(|v| v.as_i64())
                .unwrap_or_default()
                .to_string();
            let url = id_url_map
                .get(&url_id)
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !title.is_empty() && !url.is_empty() {
                out.push(json!({"title": title, "url": url, "snippet": snippet}));
            }
        }

        if out.is_empty() {
            self.planner_state
                .update_context("ax_attempted".to_string(), "true".to_string());
            return Ok(None);
        }
        // Mark success for policy context
        self.planner_state
            .update_context("extracted".to_string(), "true".to_string());
        Ok(Some(Value::Array(out)))
    }

    fn looks_like_first_comment_task(instr: &str) -> bool {
        let s = instr.to_lowercase();
        (s.contains("first comment") || s.contains("top comment") || s.contains("first reply"))
            && (s.contains("top ") || s.contains("top"))
    }

    fn parse_site_from_instruction(instr: &str) -> Option<String> {
        let s = instr.to_lowercase();
        for (needle, url) in [
            ("hackernews", "https://news.ycombinator.com/"),
            ("hacker news", "https://news.ycombinator.com/"),
            ("old.reddit.com", "https://old.reddit.com/"),
            ("reddit", "https://www.reddit.com/"),
            ("hn", "https://news.ycombinator.com/"),
        ] {
            if s.contains(needle) {
                return Some(url.to_string());
            }
        }
        None
    }

    async fn try_first_comment_top_n(&mut self, instr: &str) -> Result<Option<Value>, String> {
        let top = Self::parse_top_n(instr).unwrap_or(5) as usize;
        // 1) Decide landing page if the instruction names a site
        if let Some(url) = Self::parse_site_from_instruction(instr) {
            let nav = Step {
                id: "macro_nav".into(),
                name: format!("Navigate to {}", url),
                kind: StepKind::NavigateToUrl {
                    url: url.clone(),
                    wait: Some("domcontentloaded".into()),
                },
            };
            let _ = self
                .broker_client
                .execute_step(&nav)
                .await
                .map_err(|e| e.to_string())?;
            self.current_url = Some(url);
        }

        // 2) Build an item queue from a robust DOM inventory (ID-first)
        let inventory = self
            .broker_client
            .process_dom(Some(json!({
                "scopeSelector": null,
                "limit": 1200,
                "detectAutoList": true,
                "maxContainers": 120
            })))
            .await
            .map_err(|e| e.to_string())?;

        let root = inventory.get("result").unwrap_or(&inventory);
        let auto = root.get("autoList").and_then(|v| v.as_object());
        let elements = root
            .get("elements")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if elements.is_empty() || auto.is_none() {
            return Ok(None);
        }

        let item_ids: Vec<String> = auto
            .and_then(|o| o.get("itemIds"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .take(top)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if item_ids.is_empty() {
            return Ok(None);
        }

        let id_to_url = root
            .get("idToUrl")
            .or_else(|| root.get("id_to_url"))
            .cloned()
            .unwrap_or(json!({}));
        let id_to_xp = root
            .get("idToXPaths")
            .or_else(|| root.get("id_to_xpaths"))
            .cloned()
            .unwrap_or(json!({}));

        let current_host = self
            .current_url
            .as_ref()
            .and_then(|u| url::Url::parse(u).ok())
            .and_then(|u| Some(u.host_str().unwrap_or("").to_string()))
            .unwrap_or_default();

        let is_descendant = |parent_id: &str, child_id: &str| -> bool {
            let p_list = id_to_xp
                .get(parent_id)
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let c_list = id_to_xp
                .get(child_id)
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for p in p_list.iter().filter_map(|v| v.as_str()) {
                for c in c_list.iter().filter_map(|v| v.as_str()) {
                    if c.starts_with(p) && c.len() > p.len() {
                        return true;
                    }
                }
            }
            false
        };

        let score_anchor = |txt: &str, url: &str, internal: bool| -> i32 {
            let mut s = 0i32;
            if internal {
                s += 40;
            } else {
                s -= 10;
            }
            let t = txt.to_lowercase();
            let has_num = regex::Regex::new(r"\b\d+\b")
                .ok()
                .map(|re| re.is_match(&t))
                .unwrap_or(false);
            let is_discuss = regex::Regex::new(
                r"\b(comment|comments|discuss|discussion|reply|replies|thread|threads)\b",
            )
            .ok()
            .map(|re| re.is_match(&t))
            .unwrap_or(false);
            if is_discuss {
                s += 50;
            }
            if has_num && is_discuss {
                s += 20;
            }
            if matches!(
                t.as_str(),
                "hide" | "share" | "save" | "report" | "next" | "previous"
            ) {
                s -= 30;
            }
            let lu = url.to_lowercase();
            if lu.contains("#comments") || lu.contains("#discussion") {
                s += 8;
            }
            if lu.contains("/comments") || lu.contains("/discussion") || lu.contains("/thread") {
                s += 8;
            }
            s
        };

        // Map elements for quick lookup
        let mut role_map: std::collections::HashMap<String, (String, String, String)> =
            std::collections::HashMap::new();
        for el in &elements {
            if let Some(id) = el.get("id").and_then(|v| v.as_str()) {
                let role = el
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tag = el
                    .get("tag")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let text = el
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                role_map.insert(id.to_string(), (role, tag, text));
            }
        }

        let mut items: Vec<(String, String)> = Vec::new();
        for (i, item_id) in item_ids.iter().enumerate() {
            if i >= top {
                break;
            }
            let mut best: Option<(i32, String, String)> = None; // (score, url, text)
            for (el_id, (_role, tag, text)) in role_map.iter() {
                if tag != "a" {
                    continue;
                }
                if !is_descendant(item_id, el_id) {
                    continue;
                }
                let abs_url = id_to_url.get(el_id).and_then(|v| v.as_str()).unwrap_or("");
                if abs_url.is_empty() {
                    continue;
                }
                let internal = url::Url::parse(abs_url)
                    .ok()
                    .and_then(|u| Some(u.host_str().unwrap_or("") == current_host))
                    .unwrap_or(false);
                let sc = score_anchor(text, abs_url, internal);
                match &best {
                    Some((bs, _, _)) if sc <= *bs => {}
                    _ => best = Some((sc, abs_url.to_string(), text.clone())),
                }
            }

            // Title: first non-empty text descendant
            let mut title = String::new();
            for (el_id, (_role, _tag, text)) in role_map.iter() {
                if !text.is_empty() && is_descendant(item_id, el_id) {
                    title = text.clone();
                    break;
                }
            }

            if let Some((_, url, _txt)) = best {
                items.push((title, url));
            }
        }
        if items.is_empty() {
            return Ok(None);
        }

        // 3) Visit each discussion URL and extract first comment details generically
        let mut out: Vec<Value> = Vec::new();
        for (idx, (title, url)) in items.into_iter().enumerate() {
            let step = Step {
                id: format!("goto_{}", idx),
                name: format!("Open discussion {}", idx + 1),
                kind: StepKind::NavigateToUrl {
                    url: url.clone(),
                    wait: Some("domcontentloaded".into()),
                },
            };
            let _ = self
                .execute_step_record(step)
                .await
                .map_err(|e| e.to_string())?;

            // Re-inventory on the detail page
            let inv = self
                .broker_client
                .process_dom(Some(
                    json!({"limit": 1500, "detectAutoList": true, "maxContainers": 160 }),
                ))
                .await
                .map_err(|e| e.to_string())?;
            let root = inv.get("result").unwrap_or(&inv);
            let elements = root
                .get("elements")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let id_to_xp = root
                .get("idToXPaths")
                .or_else(|| root.get("id_to_xpaths"))
                .cloned()
                .unwrap_or(json!({}));
            let auto = root.get("autoList").and_then(|v| v.as_object());
            let comment_first_id = auto
                .and_then(|o| o.get("itemIds"))
                .and_then(|v| v.as_array())
                .and_then(|a| a.get(0))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let mut author: Option<String> = None;
            let mut time_text: Option<String> = None;
            let mut body_text: Option<String> = None;

            if let Some(first_id) = comment_first_id {
                let mut el_map: std::collections::HashMap<String, (String, String, String)> =
                    std::collections::HashMap::new();
                for el in &elements {
                    if let Some(id) = el.get("id").and_then(|v| v.as_str()) {
                        let role = el
                            .get("role")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let tag = el
                            .get("tag")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let text = el
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        el_map.insert(id.to_string(), (role, tag, text));
                    }
                }

                let is_desc = |child_id: &str| -> bool {
                    let p_list = id_to_xp
                        .get(&first_id)
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let c_list = id_to_xp
                        .get(child_id)
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    for p in p_list.iter().filter_map(|v| v.as_str()) {
                        for c in c_list.iter().filter_map(|v| v.as_str()) {
                            if c.starts_with(p) && c.len() > p.len() {
                                return true;
                            }
                        }
                    }
                    false
                };

                // Author heuristic: first short link under the comment item
                for (id, (role, tag, text)) in &el_map {
                    if !is_desc(id) {
                        continue;
                    }
                    if tag == "a" && (role == "link" || role.is_empty()) {
                        let t = text.trim();
                        if !t.is_empty()
                            && t.len() <= 20
                            && !t.contains(' ')
                            && !t.contains("@")
                            && !t.eq_ignore_ascii_case("reply")
                            && !t.eq_ignore_ascii_case("parent")
                        {
                            author = Some(t.to_string());
                            break;
                        }
                    }
                }

                // Time heuristic: first text with "ago" or month/Year
                let re_time = regex::Regex::new(r"\b(\d+\s+(minute|hour|day|month|year)s?\s+ago|\b(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\b|20\d{2})").ok();
                for (id, (_role, tag, text)) in &el_map {
                    if !is_desc(id) {
                        continue;
                    }
                    if ["span", "time", "a", "font"].contains(&tag.as_str()) {
                        if let Some(re) = &re_time {
                            if re.is_match(text) {
                                time_text = Some(text.clone());
                                break;
                            }
                        }
                    }
                }

                // Body heuristic: first long-ish text block under the comment item
                for (id, (role, tag, text)) in &el_map {
                    if !is_desc(id) {
                        continue;
                    }
                    if ["p", "div", "span", "article", "section", "blockquote", "td"]
                        .contains(&tag.as_str())
                        && role != "link"
                    {
                        let t = text.trim();
                        if t.len() >= 40 {
                            body_text = Some(t.to_string());
                            break;
                        }
                    }
                }
            }

            out.push(json!({
                "title": title,
                "discussion_url": url,
                "first_comment": { "author": author.unwrap_or_default(), "time": time_text.unwrap_or_default(), "text": body_text.unwrap_or_default() }
            }));
        }

        Ok(Some(json!(out)))
    }

    /// Generic reference-style extraction using browser-side process_dom and ID-first selection
    pub async fn extract_schema_id_first(
        &mut self,
        fields: Vec<(String, Option<String>)>, // (name, attribute)
        limit: usize,
        scope_selector: Option<String>,
    ) -> PlanResult<Option<Value>> {
        // Ask extension for candidates
        let opts = json!({
            "scopeSelector": scope_selector,
            "limit": 800,
            "detectAutoList": true,
            "maxContainers": 80
        });
        let inv = self.broker_client.process_dom(Some(opts)).await?;
        // Support both top-level and nested result shapes
        let root = inv.get("result").unwrap_or(&inv);
        let mut elements = root
            .get("elements")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if elements.is_empty() {
            return Ok(None);
        }
        let id_to_url = root
            .get("idToUrl")
            .or_else(|| root.get("id_to_url"))
            .cloned()
            .unwrap_or(json!({}));
        let id_to_xp = root
            .get("idToXPaths")
            .or_else(|| root.get("id_to_xpaths"))
            .cloned()
            .unwrap_or(json!({}));

        // Prefer autoList from processDom (IDs are already mapped)
        if let Some(al) = root.get("autoList") {
            if let Some(ids) = al.get("itemIds").and_then(|v| v.as_array()) {
                let idset: std::collections::HashSet<String> = ids
                    .iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect();
                if !idset.is_empty() {
                    let filtered: Vec<_> = elements
                        .clone()
                        .into_iter()
                        .filter(|el| {
                            let id = el.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            idset.contains(id)
                        })
                        .collect();
                    if !filtered.is_empty() {
                        elements = filtered;
                    }
                }
            }
        } else {
            // Fallback: separate detector, intersect by xpaths
            if let Ok(auto) = self.broker_client.detect_auto_list(None).await {
                let aroot = auto.get("result").unwrap_or(&auto);
                if let Some(items) = aroot.get("items").and_then(|v| v.as_array()) {
                    use std::collections::HashSet;
                    let mut target_xps: HashSet<String> = HashSet::new();
                    for it in items {
                        if let Some(xps) = it.get("xpaths").and_then(|x| x.as_array()) {
                            for xp in xps {
                                if let Some(s) = xp.as_str() {
                                    target_xps.insert(s.to_string());
                                }
                            }
                        }
                    }
                    if !target_xps.is_empty() {
                        let filtered: Vec<_> = elements
                            .clone()
                            .into_iter()
                            .filter(|el| {
                                let id = el.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                if let Some(xps) = id_to_xp.get(id).and_then(|v| v.as_array()) {
                                    for xp in xps {
                                        if let Some(s) = xp.as_str() {
                                            if target_xps.contains(s) {
                                                return true;
                                            }
                                        }
                                    }
                                }
                                false
                            })
                            .collect();
                        if !filtered.is_empty() {
                            elements = filtered;
                        }
                    }
                }
            }
        }

        // Build compact inventory for the LLM
        let mut inventory_lines: Vec<String> = Vec::new();
        for el in &elements {
            let id = el.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let tag = el.get("tag").and_then(|v| v.as_str()).unwrap_or("?");
            let role = el.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let text = el.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let attrs = el.get("attrs").or_else(|| el.get("attributes"));
            let mut attr_pairs: Vec<(String, String)> = Vec::new();
            if let Some(obj) = attrs.and_then(|a| a.as_object()) {
                for (k, v) in obj {
                    if let Some(s) = v.as_str() {
                        if ["id", "name", "href", "src", "title", "aria-label"]
                            .contains(&k.as_str())
                        {
                            attr_pairs.push((k.clone(), s.to_string()));
                        }
                    }
                }
            }
            let attr_str = if attr_pairs.is_empty() {
                String::new()
            } else {
                format!(
                    " attrs={}",
                    serde_json::to_string(&attr_pairs).unwrap_or_default()
                )
            };
            inventory_lines.push(format!(
                "{} :: <{}{}> text=\"{}\"{}",
                id,
                tag,
                if role.is_empty() {
                    String::new()
                } else {
                    format!(" role=\"{}\"", role)
                },
                text,
                attr_str
            ));
        }
        let inventory = inventory_lines.join("\n");

        // Build the prompt
        let field_names: Vec<String> = fields.iter().map(|(n, _)| n.clone()).collect();
        let sys = "You extract structured data by selecting element IDs from an inventory.\nRules:\n- Use ONLY the provided IDs; do not invent CSS/XPath.\n- For link/image fields (URL), select the element ID; runtime maps ID->URL.\n- If a field is not present and optional, omit it.\n- Output strictly valid JSON: { \"items\": [ { \"<field>\": {\"id\": string}|{\"literal\": string}, ... } ] }";
        let user = format!(
            "Fields: {}\n\nInventory (id, tag/role, text, attrs):\n{}\n\nReturn JSON only.",
            field_names.join(", "),
            inventory
        );

        let messages = vec![
            json!({"role":"system","content": sys}),
            json!({"role":"user","content": user}),
        ];
        let parsed = match self.llm_client.chat_json(messages, Some(0.0)).await {
            Ok(v) => v,
            Err(e) => {
                warn!("extract_schema_id_first LLM error: {}", e);
                return Ok(None);
            }
        };
        let items = parsed
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if items.is_empty() {
            return Ok(None);
        }

        // Resolve IDs -> values
        let mut out: Vec<Value> = Vec::new();
        for item in items.into_iter().take(limit) {
            let mut row = serde_json::Map::new();
            let mut selectors_map = serde_json::Map::new();
            for (name, attr_opt) in &fields {
                if let Some(entry) = item.get(name) {
                    if let Some(lit) = entry.get("literal").and_then(|v| v.as_str()) {
                        row.insert(name.clone(), json!(lit));
                        continue;
                    }
                    if let Some(id) = entry.get("id").and_then(|v| v.as_str()) {
                        // URL-like?
                        let url_field = attr_opt.as_deref() == Some("href")
                            || name.to_lowercase().contains("url")
                            || name.to_lowercase() == "link";
                        if url_field {
                            let url = id_to_url.get(id).and_then(|v| v.as_str()).unwrap_or("");
                            if !url.is_empty() {
                                row.insert(name.clone(), json!(url));
                            }
                        } else {
                            // Find element text by id
                            if let Some(el) = elements
                                .iter()
                                .find(|e| e.get("id").and_then(|v| v.as_str()) == Some(id))
                            {
                                let txt = el.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                if !txt.is_empty() {
                                    row.insert(name.clone(), json!(txt));
                                }
                            }
                        }
                        // Selectors for caching
                        if let Some(xps) = id_to_xp.get(id) {
                            selectors_map.insert(name.clone(), xps.clone());
                        }
                    }
                }
            }
            if !selectors_map.is_empty() {
                row.insert("_selectors".to_string(), Value::Object(selectors_map));
            }
            if !row.is_empty() {
                out.push(Value::Object(row));
            }
        }

        if out.is_empty() {
            return Ok(None);
        }
        self.planner_state
            .update_context("extracted".to_string(), "true".to_string());
        Ok(Some(Value::Array(out)))
    }

    fn detect_amazon_intent(instr: &str) -> bool {
        let l = instr.to_lowercase();
        l.contains("amazon") && (l.contains("search") || l.contains("find"))
    }

    fn extract_search_query(instr: &str) -> Option<String> {
        // Prefer quoted content first
        if let Some(start) = instr.find('"') {
            if let Some(end) = instr[start + 1..].find('"') {
                let q = instr[start + 1..start + 1 + end].trim();
                if !q.is_empty() {
                    return Some(q.to_string());
                }
            }
        }
        if let Some(start) = instr.find('\'') {
            if let Some(end) = instr[start + 1..].find('\'') {
                let q = instr[start + 1..start + 1 + end].trim();
                if !q.is_empty() {
                    return Some(q.to_string());
                }
            }
        }

        // Heuristic: capture after "search for" / "find" / "look up" / "look for", then trim
        // at common delimiters and follow-on verbs ("open", "click", "extract", ...).
        let lower = instr.to_lowercase();
        let anchors: &[&str] = &["search for ", "search ", "find ", "look up ", "look for "];

        let mut start_idx: Option<usize> = None;
        for a in anchors {
            if let Some(i) = lower.find(a) {
                start_idx = Some(i + a.len());
                break;
            }
        }
        let Some(si) = start_idx else { return None };
        let mut q = instr[si..].trim().trim_matches(['"', '\'']).to_string();
        if q.is_empty() {
            return None;
        }

        let q_lower = q.to_lowercase();
        let mut cut: Option<usize> = None;
        let delimiters: &[&str] = &[
            ",",
            ".",
            "\n",
            " and ",
            " then ",
            " open ",
            " click ",
            " extract ",
            " get ",
        ];
        for d in delimiters {
            if let Some(i) = q_lower.find(d) {
                cut = Some(cut.map(|c| c.min(i)).unwrap_or(i));
            }
        }
        if let Some(i) = cut {
            q.truncate(i);
        }
        let q = q.trim().trim_matches(['"', '\'']).to_string();
        if q.is_empty() {
            None
        } else {
            Some(q)
        }
    }

    fn parse_top_n(instr: &str) -> Option<usize> {
        let re = regex::Regex::new(r"top\s+(\d+)").ok()?;
        if let Some(caps) = re.captures(&instr.to_lowercase()) {
            if let Some(m) = caps.get(1) {
                return m.as_str().parse::<usize>().ok();
            }
        }
        None
    }

    fn looks_like_top_links_comments_task(instr: &str) -> bool {
        let l = instr.to_lowercase();
        (Self::parse_top_n(instr).is_some() || l.contains("top "))
            && (l.contains("comment") || l.contains("review"))
            && (l.contains("link")
                || l.contains("result")
                || l.contains("product")
                || l.contains("item"))
    }

    async fn try_extract_comments_from_top_links(
        &mut self,
        instruction: &str,
    ) -> PlanResult<Option<Value>> {
        let top_n = Self::parse_top_n(instruction).unwrap_or(5).max(1).min(10);
        let per_page = 10usize;
        let wants_reviews = instruction.to_lowercase().contains("review");

        let current_url = self
            .current_url
            .clone()
            .or_else(|| self.broker_client.get_current_url());
        let base_host = current_url
            .as_deref()
            .and_then(|u| url::Url::parse(u).ok())
            .and_then(|u| u.host_str().map(|s| s.to_string()));

        // First: detect a repeated list of outbound links/results on the current page.
        // We bias the detector toward "link/result" lists without hardcoding any site structure.
        let auto = match self
            .broker_client
            .detect_auto_list(Some(json!({ "purpose": "links", "maxCandidates": 5 })))
            .await
        {
            Ok(v) => v,
            Err(e) => {
                warn!("top-links comments macro: detect_auto_list failed: {}", e);
                return Ok(None);
            }
        };
        let auto_r = auto.get("result").unwrap_or(&auto);
        // Support multiple response shapes:
        // - New `detect_auto_list`: { containerSelector, itemSelector, items: [{href,...}] }
        // - Older "DOM processor" style: { autoList: { itemIds, containerSelector, itemSelector }, idToUrl: {id: url} }
        let items = auto_r
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut hrefs: Vec<String> = Vec::new();
        let base_url = current_url.clone().unwrap_or_default();
        let base_parsed = url::Url::parse(&base_url).ok();
        let unwrap_redirect_like = |resolved: &str| -> String {
            let Ok(u) = url::Url::parse(resolved) else {
                return resolved.to_string();
            };
            let base_host = base_host.as_deref().or_else(|| u.host_str());
            // Common redirect parameter names used across the web.
            const KEYS: &[&str] = &[
                "url",
                "u",
                "target",
                "dest",
                "destination",
                "redirect",
                "redir",
                "r",
            ];
            for key in KEYS {
                let Some(val) =
                    u.query_pairs()
                        .find_map(|(k, v)| if k == *key { Some(v.to_string()) } else { None })
                else {
                    continue;
                };
                let v = val.trim();
                if v.is_empty() {
                    continue;
                }
                let candidate = if v.starts_with("http://") || v.starts_with("https://") {
                    Some(v.to_string())
                } else {
                    // Treat as a relative URL/path.
                    u.join(v).ok().map(|uu| uu.to_string())
                };
                let Some(candidate) = candidate else { continue };
                if let (Some(bh), Ok(cu)) = (base_host, url::Url::parse(&candidate)) {
                    if cu.host_str() != Some(bh) {
                        continue;
                    }
                }
                return candidate;
            }
            resolved.to_string()
        };
        if !items.is_empty() {
            for it in items {
                // Different detectors may surface the destination under different keys.
                // Accept common variants (href/url/link) without any site-specific logic.
                let href = it
                    .get("href")
                    .or_else(|| it.get("url"))
                    .or_else(|| it.get("link"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if href.is_empty() {
                    continue;
                }
                if href.starts_with("javascript:") || href == "#" {
                    continue;
                }
                let resolved = if href.starts_with("http://") || href.starts_with("https://") {
                    Some(href)
                } else {
                    base_parsed
                        .as_ref()
                        .and_then(|b| b.join(&href).ok())
                        .map(|u| u.to_string())
                };
                let Some(resolved) = resolved else { continue };
                let resolved = unwrap_redirect_like(&resolved);
                if !resolved.starts_with("http://") && !resolved.starts_with("https://") {
                    continue;
                }
                if !hrefs.contains(&resolved) {
                    hrefs.push(resolved);
                }
            }
        } else if let Some(auto_list) = auto_r.get("autoList").and_then(|v| v.as_object()) {
            let id_to_url = auto_r.get("idToUrl").and_then(|v| v.as_object());
            let item_ids = auto_list
                .get("itemIds")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for idv in item_ids {
                let id = idv.as_str().unwrap_or("").trim();
                if id.is_empty() {
                    continue;
                }
                let href = id_to_url
                    .and_then(|m| m.get(id))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if href.is_empty() {
                    continue;
                }
                let resolved = if href.starts_with("http://") || href.starts_with("https://") {
                    Some(href)
                } else {
                    base_parsed
                        .as_ref()
                        .and_then(|b| b.join(&href).ok())
                        .map(|u| u.to_string())
                };
                let Some(resolved) = resolved else { continue };
                let resolved = unwrap_redirect_like(&resolved);
                if !resolved.starts_with("http://") && !resolved.starts_with("https://") {
                    continue;
                }
                if !hrefs.contains(&resolved) {
                    hrefs.push(resolved);
                }
            }
        }
        if hrefs.is_empty() {
            warn!("top-links comments macro: no http(s) links detected");
            return Ok(None);
        }
        info!(
            "top-links comments macro: detected {} candidate links (taking top {})",
            hrefs.len(),
            top_n
        );

        // Same-origin preference when possible (generic heuristic; no domain special-casing).
        if let Some(host) = base_host {
            hrefs.sort_by_key(|h| {
                url::Url::parse(h)
                    .ok()
                    .and_then(|u| u.host_str().map(|s| s != host.as_str()))
                    .unwrap_or(true)
            });
        }

        let mut out_pages: Vec<Value> = Vec::new();
        for href in hrefs.into_iter().take(top_n) {
            info!("top-links comments macro: visiting {}", href);
            let nav = Step {
                id: format!("nav_{}", Uuid::new_v4()),
                name: format!("Navigate to {}", href),
                kind: StepKind::NavigateToUrl {
                    url: href.clone(),
                    wait: Some("domcontentloaded".to_string()),
                },
            };
            let _ = self.execute_step_record(nav).await;
            self.current_url = Some(href.clone());

            let wait_body = Step {
                id: format!("wait_body_{}", Uuid::new_v4()),
                name: "Wait for body".to_string(),
                kind: StepKind::WaitForElement {
                    selector: "body".to_string(),
                    frame_id: None,
                    timeout_ms: Some(15_000),
                    condition: Some("visible".to_string()),
                },
            };
            let _ = self.execute_step_record(wait_body).await;

            // Try to find a repeated list (reviews/comments) on the page.
            // We retry with scrolls to trigger lazy-loaded sections, and we bias selection toward
            // "review-like" lists via generic signals (text density, rating markers, low link density).
            let mut chosen: Option<(String, String)> = None; // (containerSelector, itemSelector)
            for attempt in 0..6 {
                let auto2 = match self
                    .broker_client
                    .detect_auto_list(Some(json!({ "purpose": "reviews", "maxCandidates": 6 })))
                    .await
                {
                    Ok(v) => v,
                    Err(_) => {
                        // If the detector itself fails, we still try a scroll+retry.
                        json!({})
                    }
                };

                let auto2_r = auto2.get("result").unwrap_or(&auto2);
                let candidates = auto2_r
                    .get("candidates")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();

                // When we want reviews, prefer a quick probe of the top candidates to avoid
                // choosing repeated non-review lists (spec tables, related products, etc.).
                if wants_reviews && !candidates.is_empty() {
                    if let Some((csel, isel)) = self
                        .probe_review_candidate(href.as_str(), &candidates, 6)
                        .await
                    {
                        chosen = Some((csel, isel));
                        break;
                    }
                }

                let best = if candidates.is_empty() {
                    // Backwards-compatible: fall back to the single "best" candidate fields.
                    let auto2_list = auto2_r.get("autoList").unwrap_or(auto2_r);
                    let csel = auto2_list
                        .get("containerSelector")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let isel = auto2_list
                        .get("itemSelector")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if csel.is_empty() || isel.is_empty() {
                        None
                    } else {
                        // If we have per-item snippets, avoid accepting short, non-review lists (e.g. feature bullets).
                        let items = auto2_r
                            .get("items")
                            .and_then(|v| v.as_array())
                            .cloned()
                            .unwrap_or_default();
                        if !items.is_empty() {
                            let mut total_len = 0usize;
                            let mut count = 0usize;
                            let mut has_review_signal = false;
                            let mut has_rating_signal = false;
                            let mut has_verified_signal = false;
                            let mut price_like_hits = 0usize;
                            for it in &items {
                                if let Some(t) = it.get("text").and_then(|v| v.as_str()) {
                                    let lt = t.to_lowercase();
                                    total_len += t.len();
                                    count += 1;
                                    if t.contains('★')
                                        || lt.contains("out of 5")
                                        || lt.contains("stars")
                                    {
                                        has_rating_signal = true;
                                        has_review_signal = true;
                                    }
                                    if lt.contains("review") {
                                        has_review_signal = true;
                                    }
                                    if lt.contains("verified purchase") || lt.contains("verified") {
                                        has_verified_signal = true;
                                    }
                                    if t.contains('$')
                                        || lt.contains("offers from")
                                        || lt.contains("deal")
                                        || lt.contains("add to cart")
                                    {
                                        price_like_hits += 1;
                                    }
                                }
                            }
                            let avg_len = if count > 0 {
                                total_len as f64 / count as f64
                            } else {
                                0.0
                            };
                            let price_ratio = if count > 0 {
                                price_like_hits as f64 / count as f64
                            } else {
                                0.0
                            };
                            // Heuristic:
                            // - reject short lists with no review markers
                            // - reject "product card" lists (prices/offers) unless we see strong review markers like "verified purchase"
                            // - if the instruction explicitly asks for reviews, require a rating-like signal (stars/out-of-5)
                            if (!has_review_signal && avg_len < 60.0)
                                || (price_ratio > 0.15 && !has_verified_signal)
                                || (wants_reviews && !has_rating_signal)
                            {
                                None
                            } else {
                                Some((csel, isel))
                            }
                        } else {
                            Some((csel, isel))
                        }
                    }
                } else {
                    Self::pick_best_review_candidate(&candidates, wants_reviews)
                };

                if let Some((csel, isel)) = best {
                    // Don't stop on the first "kinda plausible" list. Many pages have repeated
                    // spec tables or related-product lists near the top; reviews are often lower.
                    // If we're asking for reviews and the candidate looks weak, keep scrolling.
                    let mut accept_now = true;
                    if wants_reviews && attempt < 5 {
                        if let Some(cand) = candidates.iter().find(|cand| {
                            cand.get("containerSelector")
                                .and_then(|v| v.as_str())
                                .map(|s| s.trim() == csel)
                                .unwrap_or(false)
                                && cand
                                    .get("itemSelector")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.trim() == isel)
                                    .unwrap_or(false)
                        }) {
                            let metrics = cand.get("metrics").and_then(|v| v.as_object());
                            let item_count = metrics
                                .and_then(|m| m.get("item_count"))
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0)
                                .max(1) as f64;
                            let median_text = metrics
                                .and_then(|m| m.get("median_text_len"))
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as f64;
                            let rating_hits = metrics
                                .and_then(|m| m.get("rating_hits"))
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as f64;
                            let price_hits = metrics
                                .and_then(|m| m.get("price_hits"))
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as f64;
                            let rating_ratio = (rating_hits / item_count).min(1.0);
                            let price_ratio = (price_hits / item_count).min(1.0);
                            let mut spec_like = 0usize;
                            let mut seen = 0usize;
                            if let Some(items) = cand.get("items").and_then(|v| v.as_array()) {
                                for it in items.iter().take(20) {
                                    let Some(t) = it.get("text").and_then(|v| v.as_str()) else {
                                        continue;
                                    };
                                    let tt = t.trim();
                                    if tt.is_empty() {
                                        continue;
                                    }
                                    seen += 1;
                                    if tt.contains(':') && tt.len() < 80 {
                                        spec_like += 1;
                                    }
                                }
                            }
                            let spec_ratio = if seen > 0 {
                                spec_like as f64 / seen as f64
                            } else {
                                0.0
                            };

                            // Reviews tend to be: long-ish text, rating markers, low price/spec-rows.
                            if median_text < 90.0
                                || rating_ratio < 0.15
                                || price_ratio > 0.08
                                || spec_ratio > 0.25
                            {
                                accept_now = false;
                            }
                        }
                    }

                    if accept_now || attempt == 5 {
                        chosen = Some((csel, isel));
                        break;
                    }
                }

                if attempt < 5 {
                    // Reviews are often far down on commerce/product pages; scroll more than once.
                    for _ in 0..2 {
                        let _ = self
                            .execute_step_record(Step {
                                id: format!("scroll_reviews_{}", Uuid::new_v4()),
                                name: "Scroll down for reviews".to_string(),
                                kind: StepKind::ScrollWindowTo {
                                    x: None,
                                    y: None,
                                    direction: Some("down".to_string()),
                                },
                            })
                            .await;
                        let _ = self
                            .execute_step_record(Step {
                                id: format!("wait_after_scroll_{}", Uuid::new_v4()),
                                name: "Wait after scroll".to_string(),
                                kind: StepKind::WaitForTimeout { timeout_ms: 450 },
                            })
                            .await;
                    }
                }
            }

            if chosen.is_none() && wants_reviews {
                // Some sites place reviews behind a "See all reviews" / "Ratings" affordance.
                // Use a generic click heuristic (no site-specific selectors) to open that view once.
                if self.try_click_reviews_affordance().await.unwrap_or(false) {
                    let _ = self
                        .execute_step_record(Step {
                            id: format!("wait_after_open_reviews_{}", Uuid::new_v4()),
                            name: "Wait after opening reviews".to_string(),
                            kind: StepKind::WaitForTimeout { timeout_ms: 650 },
                        })
                        .await;
                    let auto2 = self
                        .broker_client
                        .detect_auto_list(Some(json!({ "purpose": "reviews", "maxCandidates": 6 })))
                        .await
                        .unwrap_or(json!({}));
                    let auto2_r = auto2.get("result").unwrap_or(&auto2);
                    let candidates = auto2_r
                        .get("candidates")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    if !candidates.is_empty() {
                        chosen = Self::pick_best_review_candidate(&candidates, wants_reviews);
                    }
                }
            }

            let Some((container_sel, item_sel)) = chosen else {
                warn!(
                    "top-links comments macro: no review/comment list detected on {}",
                    href
                );
                continue;
            };
            info!(
                "top-links comments macro: review selector on {} -> container='{}' item='{}'",
                href, container_sel, item_sel
            );
            let mut container_sel = container_sel;
            let mut item_sel = item_sel;
            let mut results = self
                .extract_comment_rows_scoped(href.as_str(), &container_sel, &item_sel, per_page)
                .await;

            // If we landed on a non-review list (e.g. related products), or we got too few items,
            // try opening the dedicated reviews/comments view and re-detect once.
            if wants_reviews
                && (!results.is_empty())
                && (results.len() < per_page
                    || Self::extracted_rows_look_like_commerce(&results)
                    || !Self::extracted_rows_look_like_reviews(&results))
            {
                warn!(
                    "top-links comments macro: extracted {} items but they look non-review-like; retrying review discovery on {}",
                    results.len(),
                    href
                );

                if self.try_click_reviews_affordance().await.unwrap_or(false) {
                    let _ = self
                        .execute_step_record(Step {
                            id: format!("wait_after_open_reviews_retry_{}", Uuid::new_v4()),
                            name: "Wait after opening reviews".to_string(),
                            kind: StepKind::WaitForTimeout { timeout_ms: 700 },
                        })
                        .await;
                }

                for retry_attempt in 0..4 {
                    let auto_retry = self
                        .broker_client
                        .detect_auto_list(Some(json!({ "purpose": "reviews", "maxCandidates": 6 })))
                        .await
                        .unwrap_or(json!({}));
                    let auto_retry_r = auto_retry.get("result").unwrap_or(&auto_retry);
                    let candidates = auto_retry_r
                        .get("candidates")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();

                    if wants_reviews && !candidates.is_empty() {
                        if let Some((csel2, isel2)) = self
                            .probe_review_candidate(href.as_str(), &candidates, 6)
                            .await
                        {
                            container_sel = csel2;
                            item_sel = isel2;
                            let retried = self
                                .extract_comment_rows_scoped(
                                    href.as_str(),
                                    &container_sel,
                                    &item_sel,
                                    per_page,
                                )
                                .await;
                            if !retried.is_empty()
                                && !Self::extracted_rows_look_like_commerce(&retried)
                                && Self::extracted_rows_look_like_reviews(&retried)
                            {
                                results = retried;
                                break;
                            }
                        }
                    }

                    if !candidates.is_empty() {
                        if let Some((csel2, isel2)) =
                            Self::pick_best_review_candidate(&candidates, wants_reviews)
                        {
                            container_sel = csel2;
                            item_sel = isel2;
                            let retried = self
                                .extract_comment_rows_scoped(
                                    href.as_str(),
                                    &container_sel,
                                    &item_sel,
                                    per_page,
                                )
                                .await;
                            if !retried.is_empty()
                                && !Self::extracted_rows_look_like_commerce(&retried)
                                && Self::extracted_rows_look_like_reviews(&retried)
                            {
                                results = retried;
                                break;
                            }
                        }
                    }

                    if retry_attempt < 3 {
                        let _ = self
                            .execute_step_record(Step {
                                id: format!("scroll_reviews_retry_{}", Uuid::new_v4()),
                                name: "Scroll down for reviews (retry)".to_string(),
                                kind: StepKind::ScrollWindowTo {
                                    x: None,
                                    y: None,
                                    direction: Some("down".to_string()),
                                },
                            })
                            .await;
                        let _ = self
                            .execute_step_record(Step {
                                id: format!("wait_after_scroll_retry_{}", Uuid::new_v4()),
                                name: "Wait after scroll (retry)".to_string(),
                                kind: StepKind::WaitForTimeout { timeout_ms: 450 },
                            })
                            .await;
                    }
                }
            }

            if wants_reviews
                && !results.is_empty()
                && !Self::extracted_rows_look_like_reviews(&results)
            {
                warn!(
                    "top-links comments macro: skipping {} because extracted items do not resemble reviews",
                    href
                );
                continue;
            }

            if results.is_empty() {
                warn!(
                    "top-links comments macro: extracted 0 items from {} (container='{}' item='{}')",
                    href,
                    container_sel,
                    item_sel
                );
                continue;
            }
            info!(
                "top-links comments macro: extracted {} items from {}",
                results.len(),
                href
            );

            out_pages.push(json!({
                "url": href,
                "comments": Value::Array(results.into_iter().take(per_page).collect()),
            }));
        }

        if out_pages.is_empty() {
            warn!("top-links comments macro: no pages extracted");
            return Ok(None);
        }
        Ok(Some(Value::Array(out_pages)))
    }

    async fn try_click_reviews_affordance(&mut self) -> PlanResult<bool> {
        self.update_selector_inventory()
            .await
            .map_err(|e| PlanError::ExecutionError(e))?;

        let mut best: Option<(i32, SelectorDescriptor)> = None;
        for desc in self.selector_inventory.values() {
            if !desc.actions.iter().any(|a| a == "click") {
                continue;
            }
            let mut label = String::new();
            if let Some(n) = &desc.name {
                label.push_str(n);
                label.push(' ');
            }
            if let Some(t) = &desc.text {
                label.push_str(t);
                label.push(' ');
            }
            if let Some(a) = desc.attributes.get("aria-label") {
                label.push_str(a);
            }
            let l = label.to_lowercase();
            if l.is_empty() {
                continue;
            }
            // Avoid "write a review" actions; we want to read existing reviews/comments.
            if l.contains("write") || l.contains("leave a review") {
                continue;
            }
            let mut score: i32 = 0;
            if l.contains("review") {
                score += 8;
            }
            if l.contains("rating") {
                score += 4;
            }
            if l.contains("see all") {
                score += 4;
            }
            if let Some(href) = desc.attributes.get("href") {
                let lh = href.to_lowercase();
                if lh.contains("review") {
                    score += 10;
                }
                // Many sites use anchor fragments to jump to review sections.
                if lh.contains("#") && lh.contains("review") {
                    score += 2;
                }
            }
            if desc.css_selector.to_lowercase().contains("review") {
                score += 2;
            }
            if l.contains("see") {
                score += 2;
            }
            if l.contains("all") {
                score += 1;
            }
            if l.contains("customer") {
                score += 1;
            }
            // Some pages have "questions" adjacent to reviews; de-prioritize those.
            if l.contains("question") || l.contains("qa") {
                score -= 2;
            }

            match &best {
                None => best = Some((score, desc.clone())),
                Some((best_score, _)) if score > *best_score => best = Some((score, desc.clone())),
                _ => {}
            }
        }

        let Some((score, desc)) = best else {
            return Ok(false);
        };
        if score < 6 {
            return Ok(false);
        }

        let click = Step {
            id: format!("open_reviews_{}", Uuid::new_v4()),
            name: "Open reviews".to_string(),
            kind: StepKind::ClickElement {
                selector: desc.css_selector,
                frame_id: desc.frame_id,
                random_offset: Some(true),
                timeout_ms: Some(10_000),
            },
        };
        let _ = self.execute_step_record(click).await;
        Ok(true)
    }

    async fn try_click_search_submit(&mut self) -> PlanResult<bool> {
        self.update_selector_inventory()
            .await
            .map_err(PlanError::ExecutionError)?;

        let mut best: Option<(i32, SelectorDescriptor)> = None;
        for desc in self.selector_inventory.values() {
            if !desc.actions.iter().any(|a| a == "click") {
                continue;
            }
            let role = desc.role.as_deref().unwrap_or("").to_lowercase();
            let t = desc
                .attributes
                .get("type")
                .map(|s| s.to_lowercase())
                .unwrap_or_default();

            let mut label = String::new();
            if let Some(n) = &desc.name {
                label.push_str(n);
                label.push(' ');
            }
            if let Some(txt) = &desc.text {
                label.push_str(txt);
                label.push(' ');
            }
            if let Some(a) = desc.attributes.get("aria-label") {
                label.push_str(a);
            }
            let l = label.to_lowercase();

            // Avoid obvious non-submit actions.
            if l.contains("cart") || l.contains("account") || l.contains("orders") {
                continue;
            }
            if l.contains("sign in") || l.contains("log in") {
                continue;
            }

            let mut score: i32 = 0;
            if t == "submit" {
                score += 10;
            }
            if role.contains("button") {
                score += 2;
            }
            if desc.css_selector.to_lowercase().contains("submit") {
                score += 4;
            }
            if desc.css_selector.to_lowercase().contains("search") {
                score += 2;
            }
            if l.contains("search") {
                score += 6;
            }
            if l.trim() == "go" {
                score += 2;
            }
            if l.contains("go") && l.len() <= 10 {
                score += 1;
            }

            match &best {
                None => best = Some((score, desc.clone())),
                Some((best_score, _)) if score > *best_score => best = Some((score, desc.clone())),
                _ => {}
            }
        }

        let Some((score, desc)) = best else {
            return Ok(false);
        };
        if score < 7 {
            return Ok(false);
        }

        let click = Step {
            id: format!("click_search_submit_{}", Uuid::new_v4()),
            name: "Click search submit".to_string(),
            kind: StepKind::ClickElement {
                selector: desc.css_selector,
                frame_id: desc.frame_id,
                random_offset: Some(true),
                timeout_ms: Some(10_000),
            },
        };
        let _ = self.execute_step_record(click).await;
        Ok(true)
    }

    fn is_specific_container_selector(sel: &str) -> bool {
        let s = sel.trim();
        if s.is_empty() {
            return false;
        }
        if s == "div" || s == "section" || s == "article" || s == "ul" || s == "ol" || s == "table"
        {
            return false;
        }
        // Heuristic: IDs, attributes, combinators, or class tokens are typically more specific.
        s.contains('#') || s.contains('[') || s.contains('>') || s.contains('.') || s.contains(':')
    }

    fn extracted_rows_look_like_commerce(rows: &[Value]) -> bool {
        let mut n = 0usize;
        let mut commerce = 0usize;
        for row in rows.iter().take(12) {
            let Some(text) = row
                .get("comment")
                .and_then(|v| v.as_str())
                .or_else(|| row.get("text").and_then(|v| v.as_str()))
            else {
                continue;
            };
            let t = text.trim();
            if t.is_empty() {
                continue;
            }
            n += 1;
            let lt = t.to_lowercase();
            if t.contains('$')
                || t.contains('€')
                || t.contains('£')
                || t.contains('¥')
                || lt.contains("offer")
                || lt.contains("deal")
                || lt.contains("discount")
                || lt.contains("add to cart")
                || lt.contains("in stock")
                || lt.contains("shipping")
                || lt.contains("buy now")
            {
                commerce += 1;
            }
        }
        if n == 0 {
            return false;
        }
        (commerce as f64 / n as f64) > 0.20
    }

    fn extracted_rows_look_like_reviews(rows: &[Value]) -> bool {
        let mut n = 0usize;
        let mut total_len = 0usize;
        let mut rating = 0usize;
        let mut review_meta = 0usize;

        for row in rows.iter().take(12) {
            let Some(text) = row
                .get("comment")
                .and_then(|v| v.as_str())
                .or_else(|| row.get("text").and_then(|v| v.as_str()))
            else {
                continue;
            };
            let t = text.trim();
            if t.is_empty() {
                continue;
            }
            n += 1;
            total_len += t.len();
            let lt = t.to_lowercase();

            if t.contains('★') || lt.contains("out of 5") || lt.contains("stars") {
                rating += 1;
            }
            if lt.contains("reviewed in")
                || lt.contains("reviewed on")
                || lt.contains("verified purchase")
                || lt.contains("helpful")
            {
                review_meta += 1;
            }
            if lt.contains(" jan ")
                || lt.contains(" feb ")
                || lt.contains(" mar ")
                || lt.contains(" apr ")
                || lt.contains(" may ")
                || lt.contains(" jun ")
                || lt.contains(" jul ")
                || lt.contains(" aug ")
                || lt.contains(" sep ")
                || lt.contains(" oct ")
                || lt.contains(" nov ")
                || lt.contains(" dec ")
            {
                review_meta += 1;
            }
        }

        if n == 0 {
            return false;
        }
        let avg_len = total_len as f64 / n as f64;
        if avg_len < 60.0 {
            return false;
        }
        (rating + review_meta) > 0
    }

    async fn probe_review_candidate(
        &mut self,
        href: &str,
        candidates: &[Value],
        sample_limit: usize,
    ) -> Option<(String, String)> {
        if candidates.is_empty() || sample_limit == 0 {
            return None;
        }

        let mut scored: Vec<(f64, String, String)> = Vec::new();
        for cand in candidates {
            let csel = cand
                .get("containerSelector")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let isel = cand
                .get("itemSelector")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if csel.is_empty() || isel.is_empty() {
                continue;
            }
            let score = cand.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            scored.push((score, csel.to_string(), isel.to_string()));
        }

        scored.sort_by(|a, b| b.0.total_cmp(&a.0));

        // Probe a small number of top candidates. This is slower than pure heuristics,
        // but much more reliable and remains generic (no site-specific selectors).
        for (_, csel, isel) in scored.into_iter().take(4) {
            let rows = self
                .extract_comment_rows_scoped(href, &csel, &isel, sample_limit)
                .await;
            if rows.is_empty() {
                continue;
            }
            if Self::extracted_rows_look_like_commerce(&rows) {
                continue;
            }
            if Self::extracted_rows_look_like_reviews(&rows) {
                return Some((csel, isel));
            }
        }

        None
    }

    async fn extract_comment_rows_scoped(
        &mut self,
        href: &str,
        container_sel: &str,
        item_sel: &str,
        per_page: usize,
    ) -> Vec<Value> {
        let extract_rows = |resp: &Value| -> Vec<Value> {
            // Prefer result/results fields, but handle broker wrappers that return
            // `[rows, dom_snapshot]` or similar multi-part arrays.
            let candidate = resp
                .get("result")
                .or_else(|| resp.get("results"))
                .or_else(|| resp.get("data"))
                .unwrap_or(resp);
            if let Some(arr) = candidate.as_array() {
                for el in arr {
                    if let Some(inner) = el.as_array() {
                        return inner.clone();
                    }
                }
                return arr.clone();
            }
            Vec::new()
        };

        let plan = if Self::is_specific_container_selector(container_sel) {
            json!({
                "version": 1,
                "mode": "list",
                "scope": { "css": container_sel },
                "item_selector": item_sel,
                "limit": per_page,
                "fields": [
                    { "name": "comment", "selector": ":scope" }
                ]
            })
        } else {
            json!({
                "version": 1,
                "mode": "list",
                "scope": { "css": "body" },
                "item_selector": item_sel,
                "limit": per_page,
                "fields": [
                    { "name": "comment", "selector": ":scope" }
                ]
            })
        };

        // Prefer validated extraction plans (scoped, deterministic).
        let mut results = match self.broker_client.execute_extraction_plan(plan).await {
            Ok(resp) => extract_rows(&resp),
            Err(_) => Vec::new(),
        };

        // Fallback for older extension builds: use legacy `extract_structured_data` if the
        // validated plan isn't supported or returns empty.
        if results.is_empty() {
            let scoped = if Self::is_specific_container_selector(container_sel) {
                format!("{} {}", container_sel.trim(), item_sel.trim())
            } else {
                item_sel.trim().to_string()
            };
            if !scoped.is_empty() {
                let extract = Step {
                    id: format!("extract_comments_fallback_{}", Uuid::new_v4()),
                    name: "Extract comments/reviews (fallback)".to_string(),
                    kind: StepKind::ExtractStructuredData {
                        item_selector: scoped,
                        limit: None,
                        fields: vec![FieldSpec {
                            name: "comment".into(),
                            selector: "*".into(),
                            attribute: None,
                            post_processing: vec![],
                        }],
                        frame_id: None,
                        extraction_type: None,
                    },
                };
                if let Ok(resp) = self.execute_step_record(extract).await {
                    results = extract_rows(&resp);
                    if results.is_empty() {
                        let keys: Vec<String> = resp
                            .as_object()
                            .map(|o| o.keys().cloned().collect())
                            .unwrap_or_default();
                        warn!(
                            "top-links comments macro: fallback extract returned empty for {} (resp_keys={:?}, error={:?}, error_code={:?})",
                            href,
                            keys,
                            resp.get("error").and_then(|v| v.as_str()),
                            resp.get("error_code").and_then(|v| v.as_str())
                        );
                    }
                }
            }
        }

        results
    }

    fn pick_best_review_candidate(
        candidates: &[Value],
        wants_reviews: bool,
    ) -> Option<(String, String)> {
        let mut best: Option<(f64, String, String)> = None;
        for cand in candidates {
            let csel = cand
                .get("containerSelector")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let isel = cand
                .get("itemSelector")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if csel.is_empty() || isel.is_empty() {
                continue;
            }
            let metrics = cand.get("metrics").and_then(|v| v.as_object());
            let items_arr = cand.get("items").and_then(|v| v.as_array());
            let item_count_u64 = metrics
                .and_then(|m| m.get("item_count"))
                .and_then(|v| v.as_u64())
                .or_else(|| items_arr.map(|a| a.len() as u64))
                .unwrap_or(0);
            let item_count = item_count_u64.max(1) as f64;

            let median_text = metrics
                .and_then(|m| m.get("median_text_len"))
                .and_then(|v| v.as_u64())
                .map(|v| v as f64)
                .or_else(|| {
                    let Some(items) = items_arr else { return None };
                    let mut lens: Vec<usize> = items
                        .iter()
                        .filter_map(|it| it.get("text").and_then(|t| t.as_str()))
                        .map(|s| s.len())
                        .collect();
                    if lens.is_empty() {
                        return None;
                    }
                    lens.sort_unstable();
                    Some(lens[lens.len() / 2] as f64)
                })
                .unwrap_or(0.0);
            let anchor_density = metrics
                .and_then(|m| m.get("anchor_density"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let price_hits = metrics
                .and_then(|m| m.get("price_hits"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as f64;
            let rating_hits = metrics
                .and_then(|m| m.get("rating_hits"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as f64;
            let aria_list = metrics
                .and_then(|m| m.get("aria_list"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as f64;
            let carousel_penalty = metrics
                .and_then(|m| m.get("carousel_penalty"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as f64;

            // Skip tiny/low-signal lists; they are unlikely to be reviews/comments.
            if item_count < 3.0 || (median_text > 0.0 && median_text < 20.0) {
                continue;
            }

            // Supplement extension metrics with a cheap text scan, because some candidate lists
            // (e.g. commerce "recommended products") can look comment-like but are full of price/offer UI.
            let (
                scan_price_ratio,
                scan_rating_ratio,
                scan_commerce_ratio,
                scan_review_word_ratio,
                scan_spec_ratio,
            ) = if let Some(items) = items_arr {
                let mut n = 0usize;
                let mut price = 0usize;
                let mut rating = 0usize;
                let mut commerce = 0usize;
                let mut review_word = 0usize;
                let mut spec_like = 0usize;
                for it in items.iter().take(24) {
                    let Some(t) = it.get("text").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let lt = t.to_lowercase();
                    n += 1;
                    if t.contains('$')
                        || t.contains('€')
                        || t.contains('£')
                        || t.contains('¥')
                        || lt.contains(" usd")
                        || lt.contains(" eur")
                        || lt.contains(" gbp")
                        || lt.contains(" jpy")
                        || lt.contains("offer")
                        || lt.contains("deal")
                        || lt.contains("discount")
                    {
                        price += 1;
                    }
                    if t.contains('★') || lt.contains("out of 5") || lt.contains("stars") {
                        rating += 1;
                    }
                    if lt.contains("add to cart")
                        || lt.contains("in stock")
                        || lt.contains("ships")
                        || lt.contains("shipping")
                        || lt.contains("buy now")
                        || lt.contains("subscribe")
                    {
                        commerce += 1;
                    }
                    if lt.contains("review") || lt.contains("reviewed") {
                        review_word += 1;
                    }
                    // Heuristic for spec-like tables/lists: short "Label: Value" rows.
                    // This helps reject product specification tables when the user asked for reviews.
                    if t.contains(':') && t.len() < 80 {
                        spec_like += 1;
                    }
                }
                let denom = (n.max(1)) as f64;
                (
                    (price as f64 / denom).min(1.0),
                    (rating as f64 / denom).min(1.0),
                    (commerce as f64 / denom).min(1.0),
                    (review_word as f64 / denom).min(1.0),
                    (spec_like as f64 / denom).min(1.0),
                )
            } else {
                (0.0, 0.0, 0.0, 0.0, 0.0)
            };

            let rating_ratio = (rating_hits / item_count).min(1.0).max(scan_rating_ratio);
            let price_ratio = (price_hits / item_count).min(1.0).max(scan_price_ratio);
            let text_score = (median_text / 180.0).min(1.0);

            // Require "review-like" signal to avoid grabbing arbitrary repeated lists
            // (e.g., product feature bullets on commerce product pages).
            // Also reject product-card lists that tend to include prices and many links.
            if (rating_ratio < 0.10 && median_text < 60.0)
                || (price_ratio > 0.20 && anchor_density > 0.35)
                || (anchor_density > 0.55 && median_text < 140.0)
            {
                continue;
            }
            if wants_reviews {
                // Strongly reject commerce-style repeated lists when the user asked for reviews.
                // Reviews/comments rarely contain prices/offers, and also rarely include "add to cart" UI text.
                if carousel_penalty > 0.0 {
                    continue;
                }
                if rating_ratio < 0.10 {
                    continue;
                }
                if price_ratio > 0.04 {
                    continue;
                }
                if scan_commerce_ratio > 0.03 {
                    continue;
                }
                if scan_spec_ratio > 0.25 && median_text < 90.0 {
                    continue;
                }
                // Prefer lists that explicitly reference reviews, but don't make it mandatory.
                if median_text < 70.0 && scan_review_word_ratio < 0.05 {
                    continue;
                }
            }

            // Generic “review/comment list” score: dense text + rating markers, low price/link density.
            let score = (2.0 * rating_ratio) + (1.2 * text_score)
                - (1.2 * price_ratio)
                - (1.5 * anchor_density.min(1.0))
                + (0.3 * aria_list)
                - (1.8 * carousel_penalty.min(1.0));

            match &best {
                None => best = Some((score, csel.to_string(), isel.to_string())),
                Some((best_score, _, _)) if score > *best_score => {
                    best = Some((score, csel.to_string(), isel.to_string()))
                }
                _ => {}
            }
        }
        best.map(|(_, c, i)| (c, i))
    }

    /// Deterministic, code-first search + extract for speed & low tokens.
    ///
    /// This is intentionally generic (no site-specific selectors). It uses:
    /// - the selector inventory (accessibility/semantics-derived) to find a search box
    /// - a generic submit helper (Enter/form submit)
    /// - repeated-list detection to identify the results list
    async fn fast_search_extract(
        &mut self,
        url: &str,
        query: &str,
        top: usize,
    ) -> PlanResult<Option<Value>> {
        // Best-effort navigation to the starting URL (avoid hard failures).
        if self
            .current_url
            .as_deref()
            .map(|u| u != url)
            .unwrap_or(true)
        {
            let nav = Step {
                id: format!("fast_nav_{}", Uuid::new_v4()),
                name: format!("Navigate to {}", url),
                kind: StepKind::NavigateToUrl {
                    url: url.to_string(),
                    wait: Some("domcontentloaded".to_string()),
                },
            };
            let _ = self.execute_step_record(nav).await;
            self.current_url = Some(url.to_string());
        }

        let _ = self.ensure_dom_settled(3500, 250).await;

        // Find a search input from the selector inventory (generic, role/label-based).
        let mut search_desc: Option<SelectorDescriptor> = None;
        for _ in 0..10 {
            let _ = self.update_selector_inventory().await;
            let mut best: Option<(i32, SelectorDescriptor)> = None;

            for desc in self.selector_inventory.values() {
                if !desc.actions.iter().any(|a| a == "type") {
                    continue;
                }

                let field_type = desc
                    .attributes
                    .get("type")
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default();
                if field_type.contains("password")
                    || field_type.contains("email")
                    || field_type.contains("tel")
                    || field_type.contains("number")
                {
                    continue;
                }

                let mut label = String::new();
                if let Some(n) = &desc.name {
                    label.push_str(n);
                    label.push(' ');
                }
                if let Some(t) = &desc.text {
                    label.push_str(t);
                    label.push(' ');
                }
                if let Some(a) = desc.attributes.get("aria-label") {
                    label.push_str(a);
                    label.push(' ');
                }
                if let Some(p) = desc.attributes.get("placeholder") {
                    label.push_str(p);
                }
                let l = label.to_lowercase();
                if l.contains("password")
                    || l.contains("sign in")
                    || l.contains("log in")
                    || l.contains("email")
                    || l.contains("zip")
                    || l.contains("postal")
                {
                    continue;
                }

                let role = desc.role.as_deref().unwrap_or("").to_lowercase();
                let mut score: i32 = 0;
                if role.contains("searchbox") {
                    score += 8;
                }
                if field_type == "search" {
                    score += 6;
                }
                if l.contains("search") || l.contains("find") {
                    score += 6;
                }
                if desc.css_selector.contains("input") || desc.css_selector.contains("textarea") {
                    score += 2;
                }

                match &best {
                    None => best = Some((score, desc.clone())),
                    Some((best_score, _)) if score > *best_score => {
                        best = Some((score, desc.clone()))
                    }
                    _ => {}
                }
            }

            if let Some((score, desc)) = best {
                if score >= 6 {
                    search_desc = Some(desc);
                    break;
                }
            }

            let _ = self
                .execute_step_record(Step {
                    id: format!("wait_search_ui_{}", Uuid::new_v4()),
                    name: "Wait for search UI".to_string(),
                    kind: StepKind::WaitForTimeout { timeout_ms: 450 },
                })
                .await;
        }

        let Some(search_desc) = search_desc else {
            return Ok(None);
        };

        let fill = Step {
            id: format!("fast_fill_q_{}", Uuid::new_v4()),
            name: "Fill search query".to_string(),
            kind: StepKind::FillInputField {
                selector: search_desc.css_selector.clone(),
                value: query.to_string(),
                frame_id: search_desc.frame_id.clone(),
                clear_first: Some(true),
                simulate_typing: Some(false),
                delay_ms: None,
                timeout_ms: Some(12_000),
            },
        };
        let _ = self.execute_step_record(fill).await?;

        // Generic robust submit: press Enter first, then form submit fallback.
        let submit = json!({
            "type": "submit_text_query",
            "selector": search_desc.css_selector,
            "press_enter_first": true,
            "try_form_submit": true,
            "timeoutMs": 12_000
        });
        let _ = self.broker_client.execute_raw_step(submit).await;

        let _ = self.ensure_dom_settled(6000, 300).await;

        // Sanity gate: only accept a "results list" if the page appears to reflect the query.
        // This prevents accidentally treating arbitrary repeated lists on a home page as search results.
        let normalize = |s: &str| -> String {
            s.to_lowercase()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
                .collect::<String>()
        };
        let query_norm = normalize(query);
        let query_terms: Vec<String> = query_norm
            .split_whitespace()
            .filter(|w| w.len() >= 3)
            .map(|w| w.to_string())
            .collect();
        if !query_terms.is_empty() {
            let snap = self
                .broker_client
                .get_dom_snapshot()
                .await
                .unwrap_or(json!({}));
            let meta = snap.get("dom_snapshot").and_then(|d| d.get("metadata"));
            let title = meta
                .and_then(|m| m.get("title"))
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let url_now = meta
                .and_then(|m| m.get("url"))
                .and_then(|u| u.as_str())
                .unwrap_or("");
            if !url_now.is_empty() {
                self.current_url = Some(url_now.to_string());
            }
            let hay_url = normalize(self.current_url.as_deref().unwrap_or(url_now));
            let hay_title = normalize(title);
            let mut has_hint = false;
            for term in &query_terms {
                if hay_url.contains(term) || hay_title.contains(term) {
                    has_hint = true;
                    break;
                }
            }
            if !has_hint {
                return Ok(None);
            }
        }

        // Try to detect the results list and return top N links.
        let mut out: Vec<Value> = Vec::new();
        let mut seen_urls: std::collections::HashSet<String> = std::collections::HashSet::new();
        let base_url = self
            .current_url
            .clone()
            .or_else(|| self.broker_client.get_current_url())
            .unwrap_or_else(|| url.to_string());
        let base_parsed = url::Url::parse(&base_url).ok();

        for attempt in 0..8 {
            let auto = self
                .broker_client
                .detect_auto_list(Some(json!({ "purpose": "results", "maxCandidates": 6 })))
                .await
                .unwrap_or(json!({}));
            let auto_r = auto.get("result").unwrap_or(&auto);
            let items = auto_r
                .get("items")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            for it in items {
                let href_raw = it.get("href").and_then(|v| v.as_str()).unwrap_or("").trim();
                if href_raw.is_empty() || href_raw == "#" || href_raw.starts_with("javascript:") {
                    continue;
                }
                let href = if href_raw.starts_with("http://") || href_raw.starts_with("https://") {
                    href_raw.to_string()
                } else if let Some(base) = &base_parsed {
                    base.join(href_raw)
                        .ok()
                        .map(|u| u.to_string())
                        .unwrap_or_else(|| href_raw.to_string())
                } else {
                    href_raw.to_string()
                };
                if !href.starts_with("http://") && !href.starts_with("https://") {
                    continue;
                }
                if !seen_urls.insert(href.clone()) {
                    continue;
                }

                let title = it
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(|s| {
                        s.replace('\n', " ")
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();

                out.push(json!({
                    "title": title,
                    "url": href,
                }));
                if out.len() >= top.max(1).min(25) {
                    break;
                }
            }

            if out.len() >= top.max(1).min(25) || out.len() >= 3 {
                break;
            }

            if attempt == 3 {
                let _ = self
                    .execute_step_record(Step {
                        id: format!("scroll_results_{}", Uuid::new_v4()),
                        name: "Scroll results".to_string(),
                        kind: StepKind::ScrollWindowTo {
                            x: None,
                            y: None,
                            direction: Some("down".to_string()),
                        },
                    })
                    .await;
            } else {
                let _ = self
                    .execute_step_record(Step {
                        id: format!("wait_results_{}", Uuid::new_v4()),
                        name: "Wait results".to_string(),
                        kind: StepKind::WaitForTimeout { timeout_ms: 600 },
                    })
                    .await;
            }
        }

        if out.is_empty() {
            return Ok(None);
        }
        Ok(Some(Value::Array(
            out.into_iter().take(top.max(1).min(25)).collect(),
        )))
    }

    fn detect_search_intent(instruction: &str) -> Option<(String, String)> {
        use regex::Regex;
        let mut engine = "google".to_string();
        let lower = instruction.to_lowercase();

        // If the instruction explicitly targets a specific site/app ("go to X.com ... search for ..."),
        // do not treat it as a generic web-search intent. This avoids hijacking site-specific flows
        // (e.g. Amazon search) into Google.
        let mentions_engine =
            lower.contains("google") || lower.contains("bing") || lower.contains("duckduckgo");
        if !mentions_engine {
            if let Ok(host_re) = Regex::new(r"(?i)\b[a-z0-9][a-z0-9.-]*\.[a-z]{2,}\b") {
                if host_re.is_match(instruction) {
                    return None;
                }
            }
        }

        // Helper: clean captured query
        let clean = |s: &str| -> String {
            let trimmed = s.trim().trim_matches(['"', '\'', '.', ' ', ':']);
            // Strip trailing qualifiers like "titles of top N" accidentally captured
            trimmed.to_string()
        };

        let patterns: &[(&str, Option<&str>)] = &[
            (
                r"(?i)search\s+(google|bing|duckduckgo)\s+for\s+(.+)$",
                Some("engine_first"),
            ),
            (
                r"(?i)(google|bing|duckduckgo)\s+search(?:\s+for)?\s+(.+)$",
                Some("engine_first"),
            ),
            (
                r"(?i)when\s+i\s+search(?:\s+on\s+(google|bing|duckduckgo))?\s+(.+)$",
                None,
            ),
            (r"(?i)(?:search\s+for|find|look\s+up)\s+(.+)$", None),
            (r"(?i)\bgoogle\s+(.+)$", Some("engine_word")),
        ];

        for (pat, kind) in patterns {
            if let Ok(re) = Regex::new(pat) {
                if let Some(caps) = re.captures(instruction) {
                    match *kind {
                        Some("engine_first") => {
                            if let Some(eng) = caps.get(1).map(|m| m.as_str().to_lowercase()) {
                                engine = eng;
                            }
                            if let Some(q) = caps.get(2).map(|m| clean(m.as_str())) {
                                if !q.is_empty() {
                                    return Some((engine.clone(), q));
                                }
                            }
                        }
                        Some("engine_word") => {
                            engine = "google".to_string();
                            if let Some(q) = caps.get(1).map(|m| clean(m.as_str())) {
                                if !q.is_empty() {
                                    return Some((engine.clone(), q));
                                }
                            }
                        }
                        _ => {
                            if let Some(q) = caps.get(1).map(|m| clean(m.as_str())) {
                                if !q.is_empty() {
                                    return Some((engine.clone(), q));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Last resort: phrases like "top 10 .* when i search <query>"
        if let Ok(re) = Regex::new(r"(?i)when\s+i\s+search\s+(.+)$") {
            if let Some(caps) = re.captures(instruction) {
                if let Some(q) = caps.get(1).map(|m| clean(m.as_str())) {
                    if !q.is_empty() {
                        return Some((engine, q));
                    }
                }
            }
        }

        None
    }

    /// Ensure we're on a navigable page
    async fn ensure_navigable_page(&mut self) -> PlanResult<()> {
        // Prefer snapshot metadata to avoid unsupported actions
        let snapshot = match self
            .broker_client
            .get_cdp_context(Some(json!({"maxElements": 10})))
            .await
        {
            Ok(v) => v,
            Err(_) => self
                .broker_client
                .get_dom_snapshot()
                .await
                .unwrap_or(json!({})),
        };

        let url_meta = snapshot
            .get("dom_snapshot")
            .and_then(|v| v.get("metadata"))
            .and_then(|m| m.get("url"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                snapshot
                    .get("context")
                    .and_then(|v| v.get("metadata"))
                    .and_then(|m| m.get("url"))
                    .and_then(|u| u.as_str())
                    .map(|s| s.to_string())
            });

        if let Some(url) = url_meta
            .clone()
            .or_else(|| self.broker_client.get_current_url())
        {
            self.current_url = Some(url.clone());
            if url.starts_with("chrome://") || url.starts_with("about:") {
                info!("On chrome/about page, navigating to Google");
                let nav_step = Step {
                    id: "nav_google".to_string(),
                    name: "Navigate to Google".to_string(),
                    kind: StepKind::NavigateToUrl {
                        url: "https://www.google.com".to_string(),
                        wait: Some("domcontentloaded".to_string()),
                    },
                };
                self.execute_step_record(nav_step).await?;
                self.current_url = Some("https://www.google.com".to_string());
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
            return Ok(());
        }

        // Fallback: navigate to Google
        info!("No URL available, navigating to Google");
        let nav_step = Step {
            id: "nav_google".to_string(),
            name: "Navigate to Google".to_string(),
            kind: StepKind::NavigateToUrl {
                url: "https://www.google.com".to_string(),
                wait: Some("domcontentloaded".to_string()),
            },
        };
        self.execute_step_record(nav_step).await?;
        self.current_url = Some("https://www.google.com".to_string());
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        Ok(())
    }

    /// Get current page state
    async fn get_page_state(&mut self) -> Result<String, String> {
        // Best-effort settle check before fetching full snapshot
        let _ = self.ensure_dom_settled(1800, 200).await; // don't block hard on errors
                                                          // Initialize with last known URL; we'll try to refresh from snapshot metadata
        let mut current_url = self
            .current_url
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let mut output = String::new();

        // Use content snapshot (dom_snapshot) to avoid CDP attach churn in this path
        let response = self
            .broker_client
            .get_dom_snapshot()
            .await
            .unwrap_or(json!({}));
        if let Err(err) = self.update_selector_inventory().await {
            warn!("Failed to refresh selector inventory: {}", err);
        }
        if !response.is_null() {
            // Log the response structure for debugging
            warn!(
                "GetDomSnapshot response keys: {:?}",
                response.as_object().map(|o| o.keys().collect::<Vec<_>>())
            );

            // Parse compact snapshot and extract key hints
            output.push_str("Page analysis:\n");
            if let Some(dom) = response.get("dom_snapshot") {
                // Title hint
                if let Some(meta) = dom.get("metadata") {
                    if let Some(title) = meta.get("title").and_then(|t| t.as_str()) {
                        output.push_str(&format!("- Page title: {}\n", title));
                    }
                    if let Some(url) = meta.get("url").and_then(|u| u.as_str()) {
                        current_url = url.to_string();
                        self.current_url = Some(current_url.clone());
                    }
                }
                // Include compact selector prompt to ground the LLM in observed selectors
                if let Some(prompt_str) = dom.get("prompt").and_then(|p| p.as_str()) {
                    let trimmed = safe_truncate_utf8(prompt_str, 3000);
                    output.push_str("\n<rzn_dom_snapshot>\n");
                    output.push_str(trimmed);
                    output.push_str("\n</rzn_dom_snapshot>\n");
                }
            }
        }

        if !self.selector_inventory.is_empty() {
            let mut entries: Vec<&SelectorDescriptor> = self.selector_inventory.values().collect();
            entries.sort_by(|a, b| {
                self.selector_score(b)
                    .cmp(&self.selector_score(a))
                    .then_with(|| a.encoded_id.cmp(&b.encoded_id))
            });

            output.push_str("\n<rzn_selector_inventory>\n");
            for desc in entries.into_iter().take(60) {
                let label = desc
                    .name
                    .as_deref()
                    .or(desc.text.as_deref())
                    .unwrap_or("")
                    .trim();
                let actions = if desc.actions.is_empty() {
                    "-".to_string()
                } else {
                    desc.actions.join("/")
                };
                let css_preview = if desc.css_selector.len() > 120 {
                    format!("{}…", &desc.css_selector[..117])
                } else {
                    desc.css_selector.clone()
                };
                let frame_hint = self.format_frame_hint(desc.frame_ordinal, desc.frame_id.as_ref());
                let role_hint = desc
                    .role
                    .as_deref()
                    .map(|r| format!(" {}", r))
                    .unwrap_or_default();
                let attr_hint = self.format_attr_hint(&desc.attributes);

                if label.is_empty() {
                    output.push_str(&format!(
                        "- [{}]{} css={} actions={}{}{}\n",
                        desc.encoded_id, role_hint, css_preview, actions, frame_hint, attr_hint
                    ));
                } else {
                    output.push_str(&format!(
                        "- [{}]{} \"{}\" css={} actions={}{}{}\n",
                        desc.encoded_id,
                        role_hint,
                        label,
                        css_preview,
                        actions,
                        frame_hint,
                        attr_hint
                    ));
                }
            }
            output.push_str("</rzn_selector_inventory>\n");
        }

        // Prepend URL line now that we may have updated it from metadata
        output = format!("Current page URL: {}\n\n{}", current_url, output);

        // Keep the planner FSM aligned with the observed page. This avoids getting stuck in
        // Bootstrap (navigate-only) when the caller provided `--url` or when navigation already
        // happened outside the LLM loop.
        if self.planner_state.mode != PlannerMode::Complete {
            let next_mode = self
                .planner_state
                .infer_next_mode(&current_url, &output.to_lowercase());
            if next_mode != self.planner_state.mode {
                info!(
                    "[{}] Inferred mode from page: {:?} -> {:?}",
                    self.correlation_id, self.planner_state.mode, next_mode
                );
                self.planner_state.mode = next_mode;
            }
        }

        // Also include a compact list of interactive elements (from CDP AX) if compact prompt missing
        // Temporarily disabled to avoid heavyweight calls and potential async context issues in some environments.

        // Do not prime generic selectors; only provide what was observed

        Ok(output)
    }

    /// Wait briefly until DOM hash stabilizes (two consecutive equal hashes), or timeout
    async fn ensure_dom_settled(&mut self, max_wait_ms: u64, poll_ms: u64) -> Result<(), String> {
        use tokio::time::{sleep, Duration, Instant};
        let start = Instant::now();
        let mut prev: Option<String> = None;
        loop {
            match self.broker_client.get_dom_hash().await {
                Ok(h) => {
                    if let Some(p) = &prev {
                        if *p == h {
                            self.last_dom_hash = Some(h);
                            return Ok(());
                        }
                    }
                    prev = Some(h);
                }
                Err(_) => {
                    // If hash cannot be obtained, just exit quickly
                    return Err("dom_hash_unavailable".to_string());
                }
            }
            if start.elapsed() >= Duration::from_millis(max_wait_ms) {
                return Ok(());
            }
            sleep(Duration::from_millis(poll_ms)).await;
        }
    }

    /// Format DOM for LLM consumption
    fn format_dom_for_llm(&self, dom: &Value) -> String {
        let mut output = String::new();

        // Extract interactive elements
        if let Some(elements) = dom.get("elements").and_then(|e| e.as_array()) {
            output.push_str("Interactive elements:\n");

            for element in elements {
                if let Some(idx) = element.get("highlightIndex").and_then(|i| i.as_i64()) {
                    let tag = element
                        .get("tag")
                        .and_then(|t| t.as_str())
                        .unwrap_or("unknown");
                    let text = element.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    let attrs = element.get("attributes").and_then(|a| a.as_object());

                    output.push_str(&format!("[{}] <{}> ", idx, tag));

                    // Add relevant attributes
                    if let Some(attrs) = attrs {
                        if let Some(href) = attrs.get("href").and_then(|h| h.as_str()) {
                            output.push_str(&format!("href=\"{}\" ", href));
                        }
                        if let Some(value) = attrs.get("value").and_then(|v| v.as_str()) {
                            output.push_str(&format!("value=\"{}\" ", value));
                        }
                        if let Some(placeholder) = attrs.get("placeholder").and_then(|p| p.as_str())
                        {
                            output.push_str(&format!("placeholder=\"{}\" ", placeholder));
                        }
                    }

                    if !text.is_empty() {
                        output.push_str(&format!("\"{}\"", text));
                    }

                    output.push('\n');
                }
            }
        }

        output
    }

    /// Get next action from LLM
    async fn get_llm_action(&mut self) -> PlanResult<Value> {
        // Try tool-only approach first if available
        if let Some(ref tool_llm) = self.tool_llm_client {
            if let Ok(tool_response) = self.get_tool_based_action(tool_llm).await {
                return Ok(tool_response);
            }
            // Fall back to regular LLM if tool-only fails
        }

        // Prepare messages for LLM with FSM context
        let mut messages = self.conversation_history.clone();

        // Add FSM state context to system message
        if !messages.is_empty() {
            let state_context = format!(
                "\n\nCurrent Mode: {:?}\n{}",
                self.planner_state.mode,
                self.planner_state.get_system_prompt()
            );

            if let Some(system_msg) = messages.get_mut(0) {
                if let Some(content) = system_msg.get_mut("content").and_then(|c| c.as_str()) {
                    *system_msg = json!({
                        "role": "system",
                        "content": format!("{}{}", content, state_context)
                    });
                }
            }
        }

        // Log what we're sending to LLM
        info!("=== Sending to LLM ===");
        info!("[Correlation ID: {}]", self.correlation_id);
        info!("[FSM Mode: {:?}]", self.planner_state.mode);
        for (i, msg) in messages.iter().enumerate() {
            if let Some(role) = msg.get("role").and_then(|r| r.as_str()) {
                if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                    // Truncate very long content for readability (UTF-8 safe)
                    let truncated = if content.len() > 500 {
                        let head = safe_truncate_utf8(content, 500);
                        format!(
                            "{}... [truncated ~{} bytes]",
                            head,
                            content.len().saturating_sub(head.len())
                        )
                    } else {
                        content.to_string()
                    };
                    info!("[{}] {}: {}", i, role, truncated);
                }
            }
        }
        info!("=== End LLM Request ===");

        // Call LLM
        let response = self.llm_client.chat_json(messages, Some(0.1)).await?;

        // Log what we got back
        info!("=== LLM Response ===");
        info!(
            "{}",
            serde_json::to_string_pretty(&response).unwrap_or_else(|_| response.to_string())
        );
        info!("=== End LLM Response ===");

        Ok(response)
    }

    /// Get action using tool-only LLM approach
    async fn get_tool_based_action(&self, tool_llm: &ToolOnlyLLMClient) -> PlanResult<Value> {
        // Get FSM-based system prompt
        let system_prompt = format!(
            "{}\n\nCurrent Mode: {:?}\n{}",
            self.system_prompt,
            self.planner_state.mode,
            self.planner_state.get_system_prompt()
        );

        // Get allowed tools based on FSM state
        let allowed_tools = self.planner_state.get_allowed_tools();
        let all_tools = ToolOnlyLLMClient::get_standard_tools();

        // Build user prompt including the original instruction
        let original_task = self
            .conversation_history
            .get(1) // First user message after system
            .and_then(|msg| msg.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("");

        let current_state = self
            .conversation_history
            .last()
            .and_then(|msg| msg.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("");

        let user_prompt = format!("{}\n\nCurrent state: {}", original_task, current_state);

        // Call tool-only LLM
        match tool_llm
            .call_with_tools(&system_prompt, &user_prompt, all_tools, allowed_tools)
            .await
        {
            Ok(tool_calls) => {
                // Convert tool calls to our action format
                let actions: Vec<Value> = tool_calls
                    .into_iter()
                    .map(|tc| {
                        json!({
                            "cmd": tc.name,
                            "args": if tc.arguments.is_object() {
                                // Convert object arguments to array
                                tc.arguments.as_object()
                                    .map(|obj| obj.values().cloned().collect::<Vec<_>>())
                                    .unwrap_or_default()
                            } else if tc.arguments.is_array() {
                                tc.arguments.as_array().cloned().unwrap_or_default()
                            } else {
                                vec![tc.arguments]
                            }
                        })
                    })
                    .collect();

                if actions.len() == 1 {
                    Ok(json!({
                        "thought": "Executing action via tool-only mode",
                        "action": actions[0]
                    }))
                } else {
                    Ok(json!({
                        "thought": "Executing multiple actions via tool-only mode",
                        "actions": actions
                    }))
                }
            }
            Err(e) => {
                warn!("Tool-only LLM failed: {}", e);
                Err(PlanError::ExecutionError(format!(
                    "Tool-only LLM failed: {}",
                    e
                )))
            }
        }
    }

    /// Parse LLM response into structured action(s)
    fn parse_llm_response(&self, response: &Value) -> Result<Vec<LLMAction>, String> {
        // Handle both direct JSON response and content field
        let content = if let Some(content_str) = response.get("content").and_then(|c| c.as_str()) {
            // Try to parse content as JSON
            serde_json::from_str::<Value>(content_str)
                .unwrap_or_else(|_| json!({"thought": content_str, "action": {"cmd": "error", "args": ["Failed to parse response"]}}))
        } else {
            response.clone()
        };

        // Check if this is a direct action format (missing thought/action wrapper)
        if content.get("cmd").is_some() && !content.get("action").is_some() {
            // Convert direct format to expected format
            let cmd = content
                .get("cmd")
                .and_then(|c| c.as_str())
                .ok_or("No cmd in response")?
                .to_string();

            let args = content
                .get("args")
                .and_then(|a| a.as_array())
                .cloned()
                .unwrap_or_default();

            let result = content.get("result").cloned();

            // Generate a thought based on the command
            let thought = match cmd.as_str() {
                "complete" => "Task completed successfully".to_string(),
                "error" => format!(
                    "Error: {}",
                    args.get(0)
                        .and_then(|a| a.as_str())
                        .unwrap_or("Unknown error")
                ),
                _ => format!("Executing {} action", cmd),
            };

            return Ok(vec![LLMAction {
                thought,
                action: ActionCommand { cmd, args },
                result,
            }]);
        }

        // Standard format with thought and action fields
        let thought = content
            .get("thought")
            .and_then(|t| t.as_str())
            .unwrap_or("No thought provided")
            .to_string();

        // Check for multiple actions format — keep only the first (single-step discipline)
        if let Some(actions_array) = content.get("actions").and_then(|a| a.as_array()) {
            if let Some(first) = actions_array.first() {
                let cmd = first
                    .get("cmd")
                    .and_then(|c| c.as_str())
                    .ok_or_else(|| "Missing cmd in action".to_string())?
                    .to_string();
                let args = first
                    .get("args")
                    .and_then(|a| a.as_array())
                    .cloned()
                    .unwrap_or_default();
                return Ok(vec![LLMAction {
                    thought,
                    action: ActionCommand { cmd, args },
                    result: None,
                }]);
            }
        }

        // Single action format
        let action = content.get("action").ok_or("No action in response")?;

        let cmd = action
            .get("cmd")
            .and_then(|c| c.as_str())
            .ok_or("No cmd in action")?
            .to_string();

        let args = action
            .get("args")
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default();

        let mut result = content.get("result").cloned();
        if result.is_none() {
            // Some models nest completion payloads under `action.result`.
            result = action.get("result").cloned();
        }

        Ok(vec![LLMAction {
            thought,
            action: ActionCommand { cmd, args },
            result,
        }])
    }

    /// Execute an action using the existing step mechanism
    async fn execute_action(&mut self, action: &ActionCommand) -> PlanResult<Value> {
        // New generic action: extract_auto_list (domain-agnostic)
        if action.cmd.as_str() == "extract_auto_list" {
            let top: usize = action.args.get(0).and_then(|v| v.as_u64()).unwrap_or(10) as usize;

            // Fast path on results pages: use structured search extractor first (site-agnostic)
            if self.planner_state.mode == PlannerMode::Results {
                let enhanced = json!({
                    "type": "extract_structured_data",
                    "extraction_type": "search_results",
                    "fields": []
                });
                if let Ok(resp) = self.broker_client.execute_raw_step(enhanced).await {
                    if let Some(arr) = resp.get("result").and_then(|v| v.as_array()) {
                        if !arr.is_empty() {
                            let mut v = Value::Array(arr.clone());
                            if let Some(a) = v.as_array_mut() {
                                if a.len() > top {
                                    a.truncate(top);
                                }
                            }
                            return Ok(json!({ "results": v }));
                        }
                    }
                }
            }

            // Generic auto-list detection (with minimal backoff to avoid scroll oscillation)
            for attempt in 0..1 {
                if let Ok(auto) = self.broker_client.detect_auto_list(None).await {
                    // Broker may unwrap `result` (see read_response_from_broker). Handle both forms.
                    let obj = auto
                        .get("items")
                        .and_then(|_| auto.as_object())
                        .or_else(|| auto.get("result").and_then(|v| v.as_object()));
                    if let Some(obj) = obj {
                        if let Some(items) = obj.get("items").and_then(|v| v.as_array()) {
                            if !items.is_empty() {
                                let mut out = Vec::new();
                                for it in items.iter().take(top) {
                                    let title =
                                        it.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                    let url = it.get("href").and_then(|v| v.as_str()).unwrap_or("");
                                    if !title.is_empty() || !url.is_empty() {
                                        out.push(json!({ "title": title, "url": url }));
                                    }
                                }
                                if !out.is_empty() {
                                    return Ok(json!({ "results": out }));
                                }
                            }
                        }
                    }
                }
                // Nudge page to load more
                let _ = self
                    .execute_step_record(Step {
                        id: format!("wait_auto_{}", attempt),
                        name: "Wait for content".to_string(),
                        kind: StepKind::WaitForTimeout { timeout_ms: 600 },
                    })
                    .await;
                let _ = self
                    .execute_step_record(Step {
                        id: format!("scroll_auto_{}", attempt),
                        name: "Scroll a bit".to_string(),
                        kind: StepKind::ScrollWindowTo {
                            x: None,
                            y: Some(600),
                            direction: Some("down".to_string()),
                        },
                    })
                    .await;
            }
            // Final fallback on results pages: structured search extractor
            if self.planner_state.mode == PlannerMode::Results {
                let enhanced = json!({
                    "type": "extract_structured_data",
                    "extraction_type": "search_results",
                    "fields": []
                });
                if let Ok(resp) = self.broker_client.execute_raw_step(enhanced).await {
                    if let Some(arr) = resp.get("result").and_then(|v| v.as_array()) {
                        if !arr.is_empty() {
                            let mut v = Value::Array(arr.clone());
                            if let Some(a) = v.as_array_mut() {
                                if a.len() > top {
                                    a.truncate(top);
                                }
                            }
                            return Ok(json!({ "results": v }));
                        }
                    }
                }
            }

            return Err(PlanError::ExecutionError(
                "extract_auto_list: no repeated items detected".to_string(),
            ));
        }
        // Convert action to Step
        let step = match action.cmd.as_str() {
            "type_and_submit" => {
                // Execute a robust text submit in the content script:
                // fill value (if provided) + submit (Enter/form/button) with bounded retries.
                let token = action.args.get(0).and_then(|s| s.as_str()).ok_or_else(|| {
                    PlanError::ExecutionError("type_and_submit requires selector".to_string())
                })?;
                let text = action.args.get(1).and_then(|t| t.as_str()).ok_or_else(|| {
                    PlanError::ExecutionError("type_and_submit requires text".to_string())
                })?;

                let target_spec = self
                    .build_target_spec(token)
                    .map_err(PlanError::ExecutionError)?;
                let selector_label = target_spec.css.clone().unwrap_or_else(|| token.to_string());
                let submit = json!({
                    "type": "submit_text_query",
                    "selector": selector_label,
                    "value": text,
                    "press_enter_first": true,
                    "try_form_submit": true,
                    "timeoutMs": 12_000
                });

                let resp = self.broker_client.execute_raw_step(submit).await?;
                let _ = self.ensure_dom_settled(6000, 300).await;
                return Ok(resp);
            }
            "navigate" => {
                let url = action.args.get(0).and_then(|u| u.as_str()).ok_or_else(|| {
                    PlanError::ExecutionError("Navigate requires URL".to_string())
                })?;

                // Optional allowlist: set RZN_ALLOWED_HOSTS to a comma-separated list (e.g., "google.com,reddit.com")
                if let Ok(allowlist) = std::env::var("RZN_ALLOWED_HOSTS") {
                    if !allowlist.trim().is_empty() {
                        let allowed: Vec<String> = allowlist
                            .split(',')
                            .map(|s| s.trim().to_lowercase())
                            .filter(|s| !s.is_empty())
                            .collect();
                        let u = url.to_lowercase();
                        let mut ok = false;
                        for pat in &allowed {
                            if u.contains(pat) {
                                ok = true;
                                break;
                            }
                        }
                        if !ok {
                            return Err(PlanError::ExecutionError(format!(
                                "Navigation to '{}' blocked by allowlist (RZN_ALLOWED_HOSTS)",
                                url
                            )));
                        }
                    }
                }

                self.current_url = Some(url.to_string());

                Step {
                    id: format!("nav_{}", Uuid::new_v4()),
                    name: format!("Navigate to {}", url),
                    kind: StepKind::NavigateToUrl {
                        url: url.to_string(),
                        wait: Some("domcontentloaded".to_string()),
                    },
                }
            }
            "detect_popups" => Step {
                id: format!("detect_popups_{}", Uuid::new_v4()),
                name: "Detect popups".to_string(),
                kind: StepKind::DetectPopups,
            },
            "dismiss_popups" => Step {
                id: format!("dismiss_popups_{}", Uuid::new_v4()),
                name: "Dismiss popups".to_string(),
                kind: StepKind::DismissPopups,
            },
            "wait_for_no_popups" => Step {
                id: format!("wait_for_no_popups_{}", Uuid::new_v4()),
                name: "Wait for no popups".to_string(),
                kind: StepKind::WaitForNoPopups,
            },
            "handle_captcha" => Step {
                id: format!("handle_captcha_{}", Uuid::new_v4()),
                name: "Handle captcha".to_string(),
                kind: StepKind::HandleCaptcha,
            },
            "request_user_intervention" => {
                let message = action
                    .args
                    .get(0)
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string());
                Step {
                    id: format!("request_user_intervention_{}", Uuid::new_v4()),
                    name: "Request user intervention".to_string(),
                    kind: StepKind::RequestUserIntervention {
                        message,
                        instructions: None,
                        timeout_ms: None,
                        approval_mode: None,
                        approval_policy: None,
                        continue_on_timeout: None,
                        notification_title: None,
                        notification_message: None,
                    },
                }
            }
            "click" => {
                let token = action.args.get(0).and_then(|s| s.as_str()).ok_or_else(|| {
                    PlanError::ExecutionError("Click requires selector".to_string())
                })?;

                let target_spec = self
                    .build_target_spec(token)
                    .map_err(PlanError::ExecutionError)?;
                let selector_label = target_spec.css.clone().unwrap_or_else(|| token.to_string());

                let step = Step {
                    id: format!("click_{}", Uuid::new_v4()),
                    name: format!("Click {}", selector_label),
                    kind: StepKind::ClickElement {
                        selector: selector_label.clone(),
                        frame_id: None,
                        random_offset: Some(true),
                        timeout_ms: Some(5000),
                    },
                };

                return self
                    .run_step_with_target(step, &target_spec, &format!("Click {}", selector_label))
                    .await;
            }
            "type" => {
                let token = action.args.get(0).and_then(|s| s.as_str()).ok_or_else(|| {
                    PlanError::ExecutionError("Type requires selector".to_string())
                })?;
                let text =
                    action.args.get(1).and_then(|t| t.as_str()).ok_or_else(|| {
                        PlanError::ExecutionError("Type requires text".to_string())
                    })?;
                let mut target_spec = self
                    .build_target_spec(token)
                    .map_err(PlanError::ExecutionError)?;
                let mut selector_label =
                    target_spec.css.clone().unwrap_or_else(|| token.to_string());

                // Heuristic rewrite for Google: allow textarea[name='q']
                if let Some(url) = &self.current_url {
                    if url.contains("google.") && selector_label == "input[name='q']" {
                        selector_label = "textarea[name='q']".to_string();
                        target_spec.css = Some(selector_label.clone());
                    }
                }

                let step = Step {
                    id: format!("type_{}", Uuid::new_v4()),
                    name: format!("Type into {}", selector_label),
                    kind: StepKind::FillInputField {
                        selector: selector_label.clone(),
                        value: text.to_string(),
                        frame_id: None,
                        clear_first: Some(true),
                        simulate_typing: Some(false),
                        delay_ms: None,
                        timeout_ms: Some(5000),
                    },
                };

                let resp = self
                    .run_step_with_target(
                        step,
                        &target_spec,
                        &format!("Type into {}", selector_label),
                    )
                    .await?;

                // Self-healing: only for web search engines. For site-specific search UIs (e.g. commerce),
                // do not auto-submit here; let the LLM choose `type_and_submit` / `press_key` explicitly.
                let on_engine = self
                    .current_url
                    .as_ref()
                    .map(|u| {
                        let l = u.to_lowercase();
                        l.contains("google.") || l.contains("bing.") || l.contains("duckduckgo.")
                    })
                    .unwrap_or(false);
                if on_engine {
                    let submit = json!({
                        "type": "submit_text_query",
                        "selector": "input[name='q'], textarea[name='q'], input[type='search']",
                        "press_enter_first": true,
                        "try_form_submit": true,
                        "timeoutMs": 8000
                    });
                    let _ = self.broker_client.execute_raw_step(submit).await; // best-effort
                                                                               // Wait for a generic result signal (h3 headings commonly used)
                    let wait_res = Step {
                        id: "wait_results".to_string(),
                        name: "Wait results".to_string(),
                        kind: StepKind::WaitForElement {
                            selector: "#search h3, .MjjYud h3, .g h3, h3".to_string(),
                            frame_id: None,
                            condition: None,
                            timeout_ms: Some(12_000),
                        },
                    };
                    let _ = self.execute_step_record(wait_res).await;
                }

                return Ok(resp);
            }
            "press" | "press_key" => {
                let key =
                    action.args.get(0).and_then(|k| k.as_str()).ok_or_else(|| {
                        PlanError::ExecutionError("Press requires key".to_string())
                    })?;
                // If Enter in Search mode, prefer robust submit on the focused/known search box.
                // Do NOT assume web-search-engine selectors here; keep it generic.
                if key.eq_ignore_ascii_case("Enter")
                    && self.planner_state.mode == PlannerMode::Search
                {
                    let on_engine = self
                        .current_url
                        .as_ref()
                        .map(|u| {
                            let l = u.to_lowercase();
                            l.contains("google.")
                                || l.contains("bing.")
                                || l.contains("duckduckgo.")
                        })
                        .unwrap_or(false);

                    let selector = if on_engine {
                        Some(
                            "input[name='q'], textarea[name='q'], input[type='search']".to_string(),
                        )
                    } else {
                        self.planner_state
                            .context
                            .get("pending_search_token")
                            .and_then(|tok| self.build_target_spec(tok).ok())
                            .and_then(|t| t.css)
                    };

                    if let Some(selector) = selector {
                        let submit = json!({
                            "type": "submit_text_query",
                            "selector": selector,
                            "press_enter_first": true,
                            "try_form_submit": true,
                            "timeoutMs": 12_000
                        });
                        let _ = self.broker_client.execute_raw_step(submit).await; // best-effort
                        let _ = self.ensure_dom_settled(6000, 300).await;
                        // Return a lightweight OK-ish response (the outer loop will verify state).
                        return Ok(json!({ "success": true }));
                    }
                }
                // Otherwise, use press_special_key (DOM-based)
                Step {
                    id: format!("press_{}", Uuid::new_v4()),
                    name: format!("Press {} key", key),
                    kind: StepKind::PressSpecialKey {
                        key: key.to_string(),
                        selector: None,
                        frame_id: None,
                        timeout_ms: Some(5000),
                    },
                }
            }
            "scroll" => {
                let direction = action
                    .args
                    .get(0)
                    .and_then(|d| d.as_str())
                    .unwrap_or("down");
                let amount = action.args.get(1).and_then(|a| a.as_i64()).unwrap_or(500) as u32;

                Step {
                    id: format!("scroll_{}", Uuid::new_v4()),
                    name: format!("Scroll {} by {}px", direction, amount),
                    kind: StepKind::ScrollWindowTo {
                        x: if direction == "right" {
                            Some(amount as i32)
                        } else {
                            None
                        },
                        y: if direction == "down" {
                            Some(amount as i32)
                        } else {
                            None
                        },
                        direction: Some(direction.to_string()),
                    },
                }
            }
            "extract" => {
                // Prefer AX-based schema extraction in Results mode (no selectors)
                if self.planner_state.mode == PlannerMode::Results {
                    let top = Self::parse_top_n(
                        &self
                            .conversation_history
                            .iter()
                            .find_map(|m| m.get("content").and_then(|c| c.as_str()))
                            .unwrap_or(""),
                    )
                    .unwrap_or(10);
                    if let Some(url) = &self.current_url {
                        if url.contains("google.") {
                            match self.ax_extract_google_results(top).await {
                                Ok(Some(items)) => {
                                    // Wrap into a response-like shape expected by caller
                                    return Ok(json!({ "results": items }));
                                }
                                Ok(None) => {
                                    // Mark attempt; fall back to legacy extract below
                                    self.planner_state.update_context(
                                        "ax_attempted".to_string(),
                                        "true".to_string(),
                                    );
                                }
                                Err(e) => {
                                    warn!("AX extraction failed: {}", e);
                                    self.planner_state.update_context(
                                        "ax_attempted".to_string(),
                                        "true".to_string(),
                                    );
                                }
                            }
                        }
                    }
                }

                // If LLM provided a simple CSS selector, convert to structured extraction.
                // Accept forms like: {"cmd":"extract","args":["a[data-click-id='title']"]}
                if let Some(sel_str) = action.args.get(0).and_then(|v| v.as_str()) {
                    // Wait briefly for elements to appear (dynamic sites)
                    let wait = Step {
                        id: format!("wait_for_{}", Uuid::new_v4()),
                        name: format!("Wait for {}", sel_str),
                        kind: StepKind::WaitForElement {
                            selector: sel_str.to_string(),
                            frame_id: None,
                            condition: None,
                            timeout_ms: Some(8_000),
                        },
                    };
                    let _ = self.execute_step_record(wait).await;
                    // Default fields: text (title) and href (url) from the matched element itself
                    let mut fields: Vec<FieldSpec> = Vec::new();
                    fields.push(FieldSpec {
                        name: "title".to_string(),
                        selector: "*".to_string(),
                        attribute: None,
                        post_processing: vec![],
                    });
                    fields.push(FieldSpec {
                        name: "url".to_string(),
                        selector: "*".to_string(),
                        attribute: Some("href".to_string()),
                        post_processing: vec![],
                    });

                    let step = Step {
                        id: format!("extract_css_{}", Uuid::new_v4()),
                        name: format!("Extract items for selector: {}", sel_str),
                        kind: StepKind::ExtractStructuredData {
                            item_selector: sel_str.to_string(),
                            limit: None,
                            fields,
                            frame_id: None,
                            extraction_type: None,
                        },
                    };

                    let resp = self.execute_step_record(step).await?;
                    // Normalize to { results: [...] } if the raw response is an array
                    if let Some(arr) = resp.as_array() {
                        // If second arg is a number, use as top-N
                        let arg_top =
                            action.args.get(1).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        let top = if arg_top > 0 {
                            arg_top
                        } else {
                            Self::parse_top_n(
                                &self
                                    .conversation_history
                                    .iter()
                                    .find_map(|m| m.get("content").and_then(|c| c.as_str()))
                                    .unwrap_or(""),
                            )
                            .unwrap_or(10)
                        };
                        let mut items = arr.clone();
                        items.truncate(top);
                        return Ok(json!({ "results": items }));
                    }
                    return Ok(resp);
                }

                // Heuristic fallback: auto-detect list container/item selectors and return titles/urls
                // Retry auto-list detection with small waits/scrolls (domain-agnostic)
                for attempt in 0..3 {
                    if let Ok(auto) = self.broker_client.detect_auto_list(None).await {
                        // Broker may unwrap `result` (see read_response_from_broker). Handle both forms.
                        let obj = auto
                            .get("items")
                            .and_then(|_| auto.as_object())
                            .or_else(|| auto.get("result").and_then(|v| v.as_object()));
                        if let Some(obj) = obj {
                            if let Some(items) = obj.get("items").and_then(|v| v.as_array()) {
                                if !items.is_empty() {
                                    let top = Self::parse_top_n(
                                        &self
                                            .conversation_history
                                            .iter()
                                            .find_map(|m| m.get("content").and_then(|c| c.as_str()))
                                            .unwrap_or(""),
                                    )
                                    .unwrap_or(10);
                                    let mut out = Vec::new();
                                    for it in items.iter().take(top) {
                                        let title =
                                            it.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                        let url =
                                            it.get("href").and_then(|v| v.as_str()).unwrap_or("");
                                        if !title.is_empty() || !url.is_empty() {
                                            out.push(json!({ "title": title, "url": url }));
                                        }
                                    }
                                    if !out.is_empty() {
                                        return Ok(json!({ "results": out }));
                                    }
                                }
                            }
                        }
                    }
                    // If nothing found, wait and scroll to trigger lazy-loading
                    let _ = self
                        .execute_step_record(Step {
                            id: format!("wait_auto_{}", attempt),
                            name: "Wait for content".to_string(),
                            kind: StepKind::WaitForTimeout { timeout_ms: 700 },
                        })
                        .await;
                    let _ = self
                        .execute_step_record(Step {
                            id: format!("scroll_auto_{}", attempt),
                            name: "Scroll a bit".to_string(),
                            kind: StepKind::ScrollWindowTo {
                                x: None,
                                y: Some(600),
                                direction: Some("down".to_string()),
                            },
                        })
                        .await;
                }

                // Special-case: quick extraction from document title
                if let Some(arg0) = action.args.get(0) {
                    if let Some(src) = arg0.as_str() {
                        if src.eq_ignore_ascii_case("title") {
                            let html = self
                                .broker_client
                                .get_current_dom()
                                .await
                                .unwrap_or_else(|_| "".to_string());
                            // Parse <title> ... </title>
                            let title = match html
                                .split("<title>")
                                .nth(1)
                                .and_then(|s| s.split("</title>").next())
                            {
                                Some(t) => t.trim().to_string(),
                                None => self.current_url.clone().unwrap_or_default(),
                            };
                            // Extract price like $123.45 (basic USD pattern)
                            let price =
                                regex::Regex::new(r"\$\s*([0-9]{1,3}(?:,[0-9]{3})*(?:\.[0-9]+)?)")
                                    .ok()
                                    .and_then(|re| re.captures(&title))
                                    .and_then(|cap| cap.get(1).map(|m| m.as_str().to_string()));
                            let symbol = title
                                .split_whitespace()
                                .next()
                                .unwrap_or("")
                                .trim_matches(|c: char| !c.is_alphanumeric())
                                .to_string();
                            let result = json!({
                                "results": [
                                    {
                                        "symbol": symbol,
                                        "price": price.unwrap_or_default(),
                                        "source": "title",
                                        "title": title
                                    }
                                ],
                                "success": true
                            });
                            return Ok(result);
                        }
                    }
                }

                // ID-first schema extraction when selectors are missing or itemSelector is 'auto'
                if let Some(arg0) = action.args.get(0) {
                    let fields_vec: Vec<(String, Option<String>)> = arg0
                        .get("fields")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|f| {
                                    let name = f.get("name").and_then(|v| v.as_str())?.to_string();
                                    let attr = f
                                        .get("attribute")
                                        .and_then(|a| a.as_str())
                                        .map(|s| s.to_string());
                                    Some((name, attr))
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let any_missing_selector = arg0
                        .get("fields")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter().any(|f| {
                                f.get("selector")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("")
                                    .is_empty()
                            })
                        })
                        .unwrap_or(true);
                    let item_auto = arg0
                        .get("itemSelector")
                        .or_else(|| arg0.get("item_selector"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.eq_ignore_ascii_case("auto"))
                        .unwrap_or(false);
                    if !fields_vec.is_empty() && (any_missing_selector || item_auto) {
                        let limit =
                            arg0.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                        let scope = arg0
                            .get("scopeSelector")
                            .or_else(|| arg0.get("scope_selector"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        if let Ok(Some(items)) =
                            self.extract_schema_id_first(fields_vec, limit, scope).await
                        {
                            return Ok(json!({ "results": items }));
                        } else {
                            self.planner_state.update_context(
                                "extract_attempted".to_string(),
                                "true".to_string(),
                            );
                        }
                    }
                }

                // Fallback: legacy structured extraction via content-script
                let fields = action.args.get(0).cloned().unwrap_or(json!({}));
                Step {
                    id: format!("extract_{}", Uuid::new_v4()),
                    name: "Extract data".to_string(),
                    kind: StepKind::ExtractStructuredData {
                        item_selector: "body".to_string(),
                        limit: None,
                        fields: fields
                            .get("fields")
                            .and_then(|f| f.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|f| {
                                        let name = f.get("name")?.as_str()?.to_string();
                                        let selector = f.get("selector")?.as_str()?.to_string();
                                        let attribute = f
                                            .get("attribute")
                                            .and_then(|a| a.as_str())
                                            .map(|s| s.to_string());
                                        Some(FieldSpec {
                                            name,
                                            selector,
                                            attribute,
                                            post_processing: vec![],
                                        })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        frame_id: None,
                        extraction_type: None,
                    },
                }
            }
            "wait" => {
                let ms = action.args.get(0).and_then(|m| m.as_i64()).unwrap_or(1000) as u32;

                Step {
                    id: format!("wait_{}", Uuid::new_v4()),
                    name: format!("Wait {} ms", ms),
                    kind: StepKind::WaitForTimeout { timeout_ms: ms },
                }
            }
            _ => {
                return Err(PlanError::ExecutionError(format!(
                    "Unknown action: {}",
                    action.cmd
                )));
            }
        };

        // Execute the step
        let step_for_match = step.clone();
        let resp = self.execute_step_record(step).await?;

        // Post-action hooks for robustness (domain-agnostic)
        match &step_for_match.kind {
            StepKind::ScrollWindowTo { direction, .. } => {
                if !self.options.enable_macros {
                    return Ok(resp);
                }
                // Track repeated scrolls; after 2+, force list detect if goal implies top-N
                self.scrolls_since_last_extract = self.scrolls_since_last_extract.saturating_add(1);
                // Remember last direction to prevent injected opposite scrolls later
                if let Some(dir) = direction {
                    self.last_scroll_direction = Some(dir.to_lowercase());
                }
                // If the instruction implies "top N" items, try to auto-detect items after scroll
                let top = Self::parse_top_n(
                    &self
                        .conversation_history
                        .iter()
                        .find_map(|m| m.get("content").and_then(|c| c.as_str()))
                        .unwrap_or(""),
                )
                .unwrap_or(0);
                // Check if planner hinted to extract-first (from instruction semantics)
                let prefer_extract_first = self
                    .planner_state
                    .context
                    .get("prefer_extract_first")
                    .map(|v| v == "true")
                    .unwrap_or(false);
                let last_dir = self
                    .last_scroll_direction
                    .clone()
                    .unwrap_or_else(|| "down".to_string());
                if top > 0 {
                    if let Ok(auto) = self.broker_client.detect_auto_list(None).await {
                        if let Some(obj) = auto.get("result").and_then(|v| v.as_object()) {
                            if let Some(items) = obj.get("items").and_then(|v| v.as_array()) {
                                if !items.is_empty() {
                                    let mut out = Vec::new();
                                    for it in items.iter().take(top) {
                                        let title =
                                            it.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                        let url =
                                            it.get("href").and_then(|v| v.as_str()).unwrap_or("");
                                        if !title.is_empty() || !url.is_empty() {
                                            out.push(json!({ "title": title, "url": url }));
                                        }
                                    }
                                    if !out.is_empty() {
                                        return Ok(json!({ "results": out }));
                                    }
                                }
                            }
                        }
                    }
                }
                // Hard stop: after 2 scrolls without items, proactively try detect_auto_list with small wait/scroll attempts
                if self.scrolls_since_last_extract >= 2 {
                    // For extract-first intents or when the last LLM intent was to scroll up, do NOT inject more scrolls.
                    // This prevents oppositional up/down bouncing and returns control to the LLM with extraction results if possible.
                    if prefer_extract_first || last_dir == "up" {
                        for attempt in 0..2 {
                            if let Ok(auto) = self.broker_client.detect_auto_list(None).await {
                                if let Some(obj) = auto.get("result").and_then(|v| v.as_object()) {
                                    if let Some(items) = obj.get("items").and_then(|v| v.as_array())
                                    {
                                        if !items.is_empty() {
                                            let out_top = if top > 0 { top } else { 10 };
                                            let mut out = Vec::new();
                                            for it in items.iter().take(out_top) {
                                                let title = it
                                                    .get("text")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("");
                                                let url = it
                                                    .get("href")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("");
                                                if !title.is_empty() || !url.is_empty() {
                                                    out.push(json!({ "title": title, "url": url }));
                                                }
                                            }
                                            if !out.is_empty() {
                                                self.scrolls_since_last_extract = 0;
                                                return Ok(json!({ "results": out }));
                                            }
                                        }
                                    }
                                }
                            }
                            // Small settle wait only; no injected scroll
                            let _ = self
                                .broker_client
                                .execute_step(&Step {
                                    id: format!("wait_force_extract_{}", attempt),
                                    name: "Wait for content".to_string(),
                                    kind: StepKind::WaitForTimeout { timeout_ms: 500 },
                                })
                                .await;
                        }
                    } else {
                        // Maintain direction of nudge to avoid reversing user intent
                        for attempt in 0..2 {
                            if let Ok(auto) = self.broker_client.detect_auto_list(None).await {
                                if let Some(obj) = auto.get("result").and_then(|v| v.as_object()) {
                                    if let Some(items) = obj.get("items").and_then(|v| v.as_array())
                                    {
                                        if !items.is_empty() {
                                            let out_top = if top > 0 { top } else { 10 };
                                            let mut out = Vec::new();
                                            for it in items.iter().take(out_top) {
                                                let title = it
                                                    .get("text")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("");
                                                let url = it
                                                    .get("href")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("");
                                                if !title.is_empty() || !url.is_empty() {
                                                    out.push(json!({ "title": title, "url": url }));
                                                }
                                            }
                                            if !out.is_empty() {
                                                self.scrolls_since_last_extract = 0;
                                                return Ok(json!({ "results": out }));
                                            }
                                        }
                                    }
                                }
                            }
                            // Nudge and retry in the same direction as the last user-initiated scroll
                            let _ = self
                                .broker_client
                                .execute_step(&Step {
                                    id: format!("wait_force_extract_{}", attempt),
                                    name: "Wait for content".to_string(),
                                    kind: StepKind::WaitForTimeout { timeout_ms: 500 },
                                })
                                .await;
                            let _ = self
                                .broker_client
                                .execute_step(&Step {
                                    id: format!("scroll_force_extract_{}", attempt),
                                    name: "Scroll a bit".to_string(),
                                    kind: StepKind::ScrollWindowTo {
                                        x: None,
                                        y: Some(500),
                                        direction: Some(last_dir.clone()),
                                    },
                                })
                                .await;
                        }
                    }
                }
                Ok(resp)
            }
            _ => Ok(resp),
        }
    }
}

// UTF-8 safe truncate: returns a &str up to a maximum byte length without
// splitting multi-byte characters.
fn safe_truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_provider::LLMProvider;
    use async_trait::async_trait;

    // This test validates the pure JSON parsing logic and avoids
    // requiring real LLM credentials by using a minimal, inert setup.
    #[test]
    fn test_parse_llm_response() {
        // Minimal mock provider to satisfy the client without external API
        struct DummyProvider;

        #[async_trait]
        impl LLMProvider for DummyProvider {
            fn provider_name(&self) -> &str {
                "dummy"
            }
            fn model_name(&self) -> &str {
                "dummy-model"
            }
            async fn chat_completion(
                &self,
                _messages: Vec<Value>,
                _temperature: f32,
                _tools: Option<Vec<Value>>,
                _tool_choice: Option<Value>,
                _max_tokens: Option<u32>,
            ) -> crate::PlanResult<Value> {
                Ok(json!({"choices": []}))
            }
            async fn simple_chat(
                &self,
                _messages: Vec<Value>,
                _temperature: Option<f32>,
            ) -> crate::PlanResult<String> {
                Ok("{}".to_string())
            }
        }

        let llm_client = LLMClient::with_provider(Box::new(DummyProvider));

        // Pipe transport is fine here; execute_step is never called
        let planner = LLMAutonomousPlanner::new(
            llm_client,
            BrokerClient::new(crate::broker_client::Transport::Pipe),
        );

        // Test direct JSON response
        let response = json!({
            "thought": "I need to click the search button",
            "action": {
                "cmd": "click",
                "args": [5]
            }
        });

        let parsed = planner.parse_llm_response(&response).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].thought, "I need to click the search button");
        assert_eq!(parsed[0].action.cmd, "click");
        assert_eq!(parsed[0].action.args, vec![json!(5)]);

        // Test response with content field
        let response = json!({
            "content": r#"{"thought": "Typing search query", "action": {"cmd": "type", "args": [1, "test"]}}"#
        });

        let parsed = planner.parse_llm_response(&response).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].thought, "Typing search query");
        assert_eq!(parsed[0].action.cmd, "type");
        assert_eq!(parsed[0].action.args, vec![json!(1), json!("test")]);
    }
}
