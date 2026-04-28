use crate::broker_client::BrokerClient;
use crate::llm::LLMClient;
use crate::llm_autonomous::LLMAutonomousPlanner;
use crate::{PlanError, PlanResult};
use log::debug;
use rzn_core::{Step, StepKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Options for planning a single Surface-style action
#[derive(Debug, Clone, Default)]
pub struct SurfaceActOptions {
    pub scope_selector: Option<String>,
    pub max_inventory: usize,
    pub temperature: Option<f32>,
    pub instruction_prefix: Option<String>,
}

/// Planned Surface action (before execution)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfacePlannedAction {
    pub method: String,
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Result of Surface act (planning-only)
#[derive(Debug, Clone)]
pub struct SurfaceActPlan {
    pub step: Step,
    pub reasoning: Option<String>,
    pub raw_plan: Value,
    pub inventory_excerpt: String,
}

/// Result of executing a Surface action
#[derive(Debug, Clone)]
pub struct SurfaceActExecution {
    pub plan: SurfaceActPlan,
    pub execution_result: Value,
}

/// Observation candidate from Surface-style observe
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceObservationCandidate {
    pub element_id: Option<String>,
    pub method: String,
    pub selector: Option<String>,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

/// Result of Surface observe
#[derive(Debug, Clone)]
pub struct SurfaceObserveResult {
    pub actions: Vec<SurfaceObservationCandidate>,
    pub raw: Value,
    pub inventory_excerpt: String,
}

/// Field definition for structured extraction
#[derive(Debug, Clone)]
pub struct SurfaceExtractField {
    pub name: String,
    pub attribute: Option<String>,
    pub optional: bool,
}

/// Request for Surface-style extraction
#[derive(Debug, Clone)]
pub struct SurfaceExtractRequest {
    pub fields: Vec<SurfaceExtractField>,
    pub limit: usize,
    pub scope_selector: Option<String>,
}

/// Result of Surface-style extraction
#[derive(Debug, Clone)]
pub struct SurfaceExtractResult {
    pub items: Value,
    pub raw_inventory_excerpt: String,
}

/// Gather DOM inventory using process_dom bridge
async fn gather_inventory(
    broker: &mut BrokerClient,
    scope_selector: Option<String>,
    max_inventory: usize,
) -> PlanResult<(Vec<InventoryElement>, String)> {
    let options = json!({
        "scopeSelector": scope_selector,
        "limit": max_inventory,
        "detectAutoList": true,
        "maxContainers": 120
    });

    let response = broker.process_dom(Some(options)).await?;
    let root = response.get("result").unwrap_or(&response);
    let elements = root
        .get("elements")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut parsed: Vec<InventoryElement> = Vec::new();
    let mut idx = 0usize;
    let mut lines: Vec<String> = Vec::new();

    for value in elements.into_iter() {
        if let Some(elem) = InventoryElement::from_value(value.clone()) {
            idx += 1;
            lines.push(elem.to_prompt_line(idx));
            parsed.push(elem);
        }
        if parsed.len() >= max_inventory {
            break;
        }
    }

    let excerpt = if lines.is_empty() {
        "(no interactive elements detected)".to_string()
    } else {
        lines.join("\n")
    };

    Ok((parsed, excerpt))
}

/// Lightweight inventory element for prompts
#[derive(Debug, Clone)]
struct InventoryElement {
    id: Option<String>,
    tag: String,
    role: Option<String>,
    text: Option<String>,
    attrs: HashMap<String, String>,
}

impl InventoryElement {
    fn from_value(value: Value) -> Option<Self> {
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let tag = value
            .get("tag")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())?
            .to_lowercase();
        let role = value
            .get("role")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let text = value
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let attrs_val = value.get("attrs").or_else(|| value.get("attributes"));
        let mut attrs = HashMap::new();
        if let Some(map) = attrs_val.and_then(|v| v.as_object()) {
            for (k, v) in map {
                if let Some(s) = v.as_str() {
                    if matches!(
                        k.as_str(),
                        "id" | "name"
                            | "href"
                            | "src"
                            | "title"
                            | "aria-label"
                            | "aria-labelledby"
                            | "aria-describedby"
                    ) {
                        attrs.insert(k.clone(), s.to_string());
                    }
                }
            }
        }

        Some(Self {
            id,
            tag,
            role,
            text,
            attrs,
        })
    }

    fn selector_hint(&self) -> Option<String> {
        if let Some(id_attr) = self.attrs.get("id") {
            return Some(format!("#{}", css_escape(id_attr)));
        }
        if let Some(name_attr) = self.attrs.get("name") {
            return Some(format!("{}[name=\"{}\"]", self.tag, css_escape(name_attr)));
        }
        if let Some(aria) = self.attrs.get("aria-label") {
            return Some(format!("{}[aria-label=\"{}\"]", self.tag, css_escape(aria)));
        }
        if let Some(title) = self.attrs.get("title") {
            return Some(format!("{}[title=\"{}\"]", self.tag, css_escape(title)));
        }
        if let Some(href) = self.attrs.get("href") {
            let trimmed = href.chars().take(60).collect::<String>();
            return Some(format!("{}[href*\"{}\"]", self.tag, css_escape(&trimmed)));
        }
        None
    }

    fn to_prompt_line(&self, index: usize) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push(format!("{}.", index));
        if let Some(id) = &self.id {
            parts.push(format!("id={} ", id));
        }
        parts.push(format!("<{}>", self.tag));
        if let Some(role) = &self.role {
            parts.push(format!(" role={} ", role));
        }
        if let Some(text) = &self.text {
            parts.push(format!(" text=\"{}\"", truncate(text, 80)));
        }
        if !self.attrs.is_empty() {
            let attrs_string = self
                .attrs
                .iter()
                .map(|(k, v)| format!("{}=\"{}\"", k, truncate(v, 60)))
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!(" attrs=[{}]", attrs_string));
        }
        if let Some(sel) = self.selector_hint() {
            parts.push(format!(" selector_hint={}", sel));
        }
        parts.join("")
    }
}

/// Plan a Surface-style action without executing it
pub async fn plan_act(
    llm_client: &LLMClient,
    broker: &mut BrokerClient,
    instruction: &str,
    options: SurfaceActOptions,
) -> PlanResult<SurfaceActPlan> {
    let (inventory, inventory_excerpt) = gather_inventory(
        broker,
        options.scope_selector.clone(),
        options.max_inventory,
    )
    .await?;

    let current_url = broker
        .get_current_url()
        .unwrap_or_else(|| "unknown".to_string());

    let system_prompt = build_act_system_prompt();
    let user_prompt = build_act_user_prompt(
        instruction,
        &current_url,
        options.instruction_prefix.as_deref(),
        inventory.len(),
        &inventory_excerpt,
    );

    let messages = vec![
        json!({"role": "system", "content": system_prompt}),
        json!({"role": "user", "content": user_prompt}),
    ];

    let plan = llm_client
        .chat_json(messages, options.temperature)
        .await
        .map_err(|e| PlanError::LLMError(format!("Failed to plan action: {}", e)))?;

    debug!("Surface plan: {}", plan);

    let action = parse_stagehand_action(&plan)?;
    let step = action_to_step(&action, instruction)?;

    Ok(SurfaceActPlan {
        step,
        reasoning: action.reason.clone(),
        raw_plan: plan,
        inventory_excerpt,
    })
}

/// Execute Surface-style action end-to-end
pub async fn execute_act(
    llm_client: &LLMClient,
    broker: &mut BrokerClient,
    instruction: &str,
    options: SurfaceActOptions,
) -> PlanResult<SurfaceActExecution> {
    let plan = plan_act(llm_client, broker, instruction, options).await?;
    let exec = broker.execute_step(&plan.step).await?;
    Ok(SurfaceActExecution {
        plan,
        execution_result: exec,
    })
}

/// Observe actionable items via LLM summarisation
pub async fn observe(
    llm_client: &LLMClient,
    broker: &mut BrokerClient,
    instruction: &str,
    scope_selector: Option<String>,
    max_inventory: usize,
    max_results: usize,
    temperature: Option<f32>,
) -> PlanResult<SurfaceObserveResult> {
    let (inventory, inventory_excerpt) =
        gather_inventory(broker, scope_selector, max_inventory).await?;
    let current_url = broker
        .get_current_url()
        .unwrap_or_else(|| "unknown".to_string());

    let system_prompt = build_observe_system_prompt(max_results);
    let user_prompt = build_observe_user_prompt(instruction, &current_url, &inventory_excerpt);

    let messages = vec![
        json!({"role": "system", "content": system_prompt}),
        json!({"role": "user", "content": user_prompt}),
    ];

    let raw = llm_client
        .chat_json(messages, temperature)
        .await
        .map_err(|e| PlanError::LLMError(format!("Failed to observe: {}", e)))?;

    let actions = parse_observe_candidates(raw.clone())?;

    Ok(SurfaceObserveResult {
        actions,
        raw,
        inventory_excerpt,
    })
}

/// Extract structured data Surface-style (ID-first selection)
pub async fn extract(
    llm_client: LLMClient,
    broker: BrokerClient,
    request: SurfaceExtractRequest,
) -> PlanResult<SurfaceExtractResult> {
    let mut planner = LLMAutonomousPlanner::new(llm_client, broker);

    let fields: Vec<(String, Option<String>)> = request
        .fields
        .iter()
        .map(|f| (f.name.clone(), f.attribute.clone()))
        .collect();

    let limit = request.limit.max(1);
    let scope = request.scope_selector.clone();

    match planner
        .extract_schema_id_first(fields, limit, scope)
        .await?
    {
        Some(items) => Ok(SurfaceExtractResult {
            items,
            raw_inventory_excerpt: "extract_schema_id_first".to_string(),
        }),
        None => Err(PlanError::LLMError(
            "Extraction returned no data".to_string(),
        )),
    }
}

fn build_act_system_prompt() -> String {
    r#"You are SurfaceCompat, an expert browser action planner.
Return ONE atomic action that directly progresses the instruction.
Allowed methods: click, fill, press, hover, wait_for_selector, wait_for_timeout, scroll.

Rules:
1. Use CSS selectors from provided inventory hints.
2. Prefer stable identifiers (id, name, aria-label, href).
3. Keep actions atomic (one click, one fill, etc.).
4. Never fabricate selectors that cannot be built from provided data.
5. Respond with strict JSON: {"action": {"method": "<method>", "selector": "<selector>", "text": "<value?>", "key": "<key?>", "timeout_ms": <number?>}, "reason": "<why>"}.
"#.to_string()
}

fn build_act_user_prompt(
    instruction: &str,
    current_url: &str,
    prefix: Option<&str>,
    inventory_count: usize,
    inventory_excerpt: &str,
) -> String {
    let mut prompt = String::new();
    if let Some(p) = prefix {
        prompt.push_str(p);
        prompt.push_str("\n\n");
    }
    prompt.push_str(&format!(
        "Instruction: {}\nCurrent URL: {}\n\nInventory (top {} actionable elements):\n{}\n\nRespond with JSON shaped like: {{\"action\":{{\"method\":\"<method>\",\"selector\":\"<selector>\",\"text\":\"<optional>\",\"key\":\"<optional>\",\"timeout_ms\":<optional_number>}},\"reason\":\"<why>\"}}.",
        instruction,
        current_url,
        inventory_count,
        inventory_excerpt
    ));
    prompt
}

fn build_observe_system_prompt(max_results: usize) -> String {
    format!("You are SurfaceCompat observer. Identify up to {} high-impact actions users could take next. Return JSON: {{\"actions\": [{{\"description\": \"<what>\", \"method\": \"<method>\", \"selector\": \"<selector?>\", \"element_id\": \"<id?>\", \"confidence\": <0-1>}}]}}. Scores between 0 and 1.", max_results)
}

fn build_observe_user_prompt(
    instruction: &str,
    current_url: &str,
    inventory_excerpt: &str,
) -> String {
    format!(
        "Instruction focus: {}\nCurrent URL: {}\nInventory snapshot:\n{}\n\nList actionable candidates with method + selector. Only use selectors derivable from hints.",
        instruction,
        current_url,
        inventory_excerpt
    )
}

fn parse_stagehand_action(plan: &Value) -> PlanResult<SurfacePlannedAction> {
    let action = plan
        .get("action")
        .ok_or_else(|| PlanError::LLMError("Missing action field".to_string()))?;
    let method = action
        .get("method")
        .and_then(|v| v.as_str())
        .ok_or_else(|| PlanError::LLMError("Action missing method".to_string()))?
        .to_lowercase();

    let selector = action
        .get("selector")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let text = action
        .get("text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let key = action
        .get("key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let timeout_ms = action
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let reason = plan
        .get("reason")
        .or_else(|| plan.get("reasoning"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(SurfacePlannedAction {
        method,
        selector,
        text,
        key,
        timeout_ms,
        reason,
    })
}

fn action_to_step(action: &SurfacePlannedAction, instruction: &str) -> PlanResult<Step> {
    let timeout = action.timeout_ms.unwrap_or(8000);
    match action.method.as_str() {
        "click" => {
            let selector = require_selector(action)?;
            Ok(Step {
                id: "stagehand_click".to_string(),
                name: format!("Click - {}", instruction),
                kind: StepKind::ClickElement {
                    selector,
                    frame_id: None,
                    random_offset: Some(true),
                    timeout_ms: Some(timeout),
                },
            })
        }
        "fill" | "type" => {
            let selector = require_selector(action)?;
            let value = action
                .text
                .clone()
                .ok_or_else(|| PlanError::LLMError("Fill action missing text".to_string()))?;
            Ok(Step {
                id: "stagehand_fill".to_string(),
                name: format!("Fill - {}", instruction),
                kind: StepKind::FillInputField {
                    selector,
                    value,
                    frame_id: None,
                    clear_first: Some(true),
                    simulate_typing: Some(true),
                    delay_ms: Some(40),
                    timeout_ms: Some(timeout),
                },
            })
        }
        "press" => {
            let selector = action.selector.clone();
            let key = action
                .key
                .clone()
                .ok_or_else(|| PlanError::LLMError("Press action missing key".to_string()))?;
            Ok(Step {
                id: "stagehand_press".to_string(),
                name: format!("Press - {}", instruction),
                kind: StepKind::PressSpecialKey {
                    key,
                    selector,
                    frame_id: None,
                    timeout_ms: Some(timeout),
                },
            })
        }
        "hover" => {
            let selector = require_selector(action)?;
            Ok(Step {
                id: "stagehand_hover".to_string(),
                name: format!("Hover - {}", instruction),
                kind: StepKind::HoverElement {
                    selector,
                    frame_id: None,
                    random_offset: Some(true),
                    timeout_ms: Some(timeout),
                },
            })
        }
        "wait_for_selector" => {
            let selector = require_selector(action)?;
            Ok(Step {
                id: "stagehand_wait_selector".to_string(),
                name: format!("Wait for selector - {}", instruction),
                kind: StepKind::WaitForElement {
                    selector,
                    frame_id: None,
                    condition: Some("visible".to_string()),
                    timeout_ms: Some(timeout),
                },
            })
        }
        "wait_for_timeout" | "wait" => Ok(Step {
            id: "stagehand_wait_time".to_string(),
            name: format!("Wait - {}", instruction),
            kind: StepKind::WaitForTimeout {
                timeout_ms: timeout,
            },
        }),
        "scroll" => Ok(Step {
            id: "stagehand_scroll".to_string(),
            name: format!("Scroll - {}", instruction),
            kind: StepKind::ScrollWindowTo {
                x: None,
                y: Some(800),
                direction: Some("down".to_string()),
            },
        }),
        other => Err(PlanError::LLMError(format!(
            "Unsupported action method: {}",
            other
        ))),
    }
}

fn require_selector(action: &SurfacePlannedAction) -> PlanResult<String> {
    action
        .selector
        .clone()
        .ok_or_else(|| PlanError::LLMError("Action missing selector".to_string()))
}

fn parse_observe_candidates(raw: Value) -> PlanResult<Vec<SurfaceObservationCandidate>> {
    let list = raw
        .get("actions")
        .or_else(|| raw.get("candidates"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out: Vec<SurfaceObservationCandidate> = Vec::new();
    for item in list {
        let method = item
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("click")
            .to_string();
        let selector = item
            .get("selector")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let description = item
            .get("description")
            .or_else(|| item.get("reason"))
            .and_then(|v| v.as_str())
            .unwrap_or("Suggested action")
            .to_string();
        let element_id = item
            .get("element_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let text = item
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let confidence = item
            .get("confidence")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32);
        out.push(SurfaceObservationCandidate {
            element_id,
            method,
            selector,
            description,
            text,
            confidence,
        });
    }

    if out.is_empty() {
        return Err(PlanError::LLMError(
            "Observe returned no candidates".to_string(),
        ));
    }

    Ok(out)
}

fn css_escape(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push(' '),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

fn truncate(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        return value.to_string();
    }
    let mut end = max_len;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_click_action() {
        let plan = json!({
            "action": {
                "method": "click",
                "selector": "#cta"
            },
            "reason": "Click the primary CTA"
        });

        let parsed = parse_stagehand_action(&plan).expect("parse");
        let step = action_to_step(&parsed, "Click CTA").expect("step");

        match step.kind {
            StepKind::ClickElement { selector, .. } => {
                assert_eq!(selector, "#cta");
            }
            other => panic!("unexpected step kind: {:?}", other),
        }
        assert_eq!(parsed.reason.unwrap(), "Click the primary CTA");
    }

    #[test]
    fn test_css_escape_quotes() {
        let escaped = css_escape("button\"Save\"");
        assert_eq!(escaped, "button\\\"Save\\\"");
    }
}
