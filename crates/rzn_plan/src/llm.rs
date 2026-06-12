use crate::anthropic_client::AnthropicClient;
use crate::cli_client::{CliClient, CliKind};
use crate::gemini_client::GeminiClient;
use crate::groq_client::GroqClient;
use crate::llm_provider::{LLMProvider, ProviderType};
use crate::openai_client::OpenAIClient;
use crate::{PlanConfig, PlanError, PlanResult};
use log::{debug, error, info};
use rzn_core::secure_files::{append_secret_file_capped, secure_dir};
use rzn_core::{Step, StepKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

const LLM_DEBUG_LOG_ENV: &str = "RZN_LLM_DEBUG_LOG";
const LLM_DEBUG_LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;
const LLM_RAW_RESPONSE_LOG: &str = "llm_responses.debug.jsonl";
const LLM_MESSAGE_LOG: &str = "llm_messages.debug.log";

/// LLM client that abstracts different providers (OpenAI, Gemini, etc.)
pub struct LLMClient {
    provider: Arc<dyn LLMProvider>,
    temperature: f32,
}

fn llm_debug_logging_enabled() -> bool {
    matches!(
        std::env::var(LLM_DEBUG_LOG_ENV)
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
    )
}

fn append_llm_debug_log(file_name: &str, build: impl FnOnce(&mut String)) -> Option<PathBuf> {
    if !llm_debug_logging_enabled() {
        return None;
    }
    let dir = secure_dir("llm-debug").ok()?;
    let path = dir.join(file_name);
    let mut buffer = String::new();
    build(&mut buffer);
    if !buffer.ends_with('\n') {
        buffer.push('\n');
    }
    append_secret_file_capped(&path, buffer.as_bytes(), LLM_DEBUG_LOG_MAX_BYTES).ok()?;
    Some(path)
}

fn append_llm_raw_response_log(message: &str, response: &Value) {
    append_llm_debug_log(LLM_RAW_RESPONSE_LOG, |buffer| {
        let _ = writeln!(
            buffer,
            "{}",
            serde_json::json!({
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "level": "DEBUG",
                "component": "llm_raw",
                "message": message,
                "data": response
            })
        );
    });
}

fn append_llm_message_log(
    label: &str,
    messages: Option<&[Value]>,
    response: Option<&Value>,
) -> Option<PathBuf> {
    append_llm_debug_log(LLM_MESSAGE_LOG, |buffer| {
        let _ = writeln!(buffer, "\n{}", "=".repeat(80));
        let _ = writeln!(
            buffer,
            "{} [{}]",
            label,
            chrono::Local::now().format("%H:%M:%S")
        );
        let _ = writeln!(buffer, "{}", "=".repeat(80));
        if let Some(messages) = messages {
            let value = serde_json::to_string_pretty(messages)
                .unwrap_or_else(|_| "Failed to serialize messages".to_string());
            let _ = writeln!(buffer, "{}", value);
        }
        if let Some(response) = response {
            let value = serde_json::to_string_pretty(response)
                .unwrap_or_else(|_| "Failed to serialize response".to_string());
            let _ = writeln!(buffer, "{}", value);
        }
        let _ = writeln!(buffer, "{}", "=".repeat(80));
    })
}

/// Simple chat response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMChatResponse {
    pub content: String,
}

/// Response from LLM planning request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMResponse {
    /// Whether the goal has been completed
    pub is_complete: bool,

    /// Next step to execute (if not complete)
    pub next_step: Option<Step>,

    /// Extracted data (if complete)
    pub extracted_data: Option<Value>,

    /// Reasoning from the LLM
    pub reasoning: Option<String>,
}

/// Action group selection for two-tier planning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupSelection {
    /// Selected action group
    pub group: ActionGroup,

    /// Reasoning for group selection
    pub reasoning: String,
}

/// Available action groups for two-tier planning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionGroup {
    Navigate,
    Element,
    Scroll,
    Wait,
    Data,
    Assert,
    JsEval,
    Util,
}

/// Concrete step result from two-tier planning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcreteStep {
    /// The concrete step to execute
    pub step: Step,

    /// Reasoning for this specific action
    pub reasoning: String,
}

/// OpenAI function schema for browser steps
const BROWSER_STEP_SCHEMA: &str = r#"
{
  "name": "browser_step",
  "description": "Execute a single browser automation action",
  "parameters": {
    "type": "object",
    "required": ["id", "name", "type"],
    "properties": {
      "id": {
        "type": "string",
        "description": "Unique identifier for this step"
      },
      "name": {
        "type": "string", 
        "description": "Human-readable description of this step"
      },
      "type": {
        "type": "string",
        "enum": [
          "navigate_to_url",
          "click_element",
          "fill_input_field",
          "press_key",
          "hover_element",
          "wait_for_selector",
          "wait_timeout",
          "extract_structured_data",
          "execute_javascript",
          "eval_main_world",
          "eval_isolated_world",
          "inspect_element",
          "inspect_click_surface",
          "capture_ui_bundle",
          "verify_ui_change",
          "read_field_value",
          "semantic_action"
        ],
        "description": "Type of browser action to perform"
      },
      "url": {
        "type": "string",
        "description": "URL to navigate to (for navigate_to_url steps)"
      },
      "selector": {
        "type": "string",
        "description": "CSS selector for the target element"
      },
      "value": {
        "type": "string",
        "description": "Text to enter (for fill_input_field steps)"
      },
      "key": {
        "type": "string",
        "description": "Key to press (for press_key steps)"
      },
      "action": {
        "type": "string",
        "description": "Semantic action name for semantic_action steps"
      },
      "script": {
        "type": "string",
        "description": "JavaScript source for eval/execute_javascript steps"
      },
      "args": {
        "type": "array",
        "description": "Arguments for eval/execute_javascript steps"
      },
      "return_value": {
        "type": "boolean",
        "description": "Whether to return the JS result"
      },
      "world": {
        "type": "string",
        "enum": ["isolated", "main"],
        "description": "Execution world for execute_javascript compatibility"
      },
      "timeout": {
        "type": "integer",
        "minimum": 0,
        "description": "Timeout in milliseconds"
      },
      "timeout_ms": {
        "type": "integer",
        "minimum": 0,
        "description": "Timeout in milliseconds"
      },
      "item_selector": {
        "type": "string",
        "description": "CSS selector for container elements (for extract_structured_data steps)"
      },
      "fields": {
        "type": "array",
        "description": "Fields to extract (for extract_structured_data steps)",
        "items": {
          "type": "object",
          "required": ["name", "selector"],
          "properties": {
            "name": {
              "type": "string",
              "description": "Name of the field"
            },
            "selector": {
              "type": "string", 
              "description": "CSS selector for the field"
            },
            "attribute": {
              "type": "string",
              "description": "HTML attribute to extract (optional)"
            },
            "post_processing": {
              "type": "array",
              "description": "Post-processing operations to apply",
              "items": {
                "type": "string"
              }
            }
          }
        }
      },
      "include_ancestors": {
        "type": "boolean",
        "description": "Include ancestor summaries in inspect_element results"
      },
      "include_shadow_path": {
        "type": "boolean",
        "description": "Include shadow-root ancestry in inspect_element results"
      },
      "include_dom_snapshot": {
        "type": "boolean",
        "description": "Include a DOM snapshot in capture_ui_bundle"
      },
      "include_screenshot": {
        "type": "boolean",
        "description": "Include a screenshot in capture_ui_bundle"
      },
      "annotate": {
        "type": "boolean",
        "description": "Annotate screenshots when supported"
      },
      "max_elements": {
        "type": "integer",
        "minimum": 1,
        "description": "Maximum elements to include in UI bundles"
      },
      "condition": {
        "type": "string",
        "description": "Condition for wait/assert/verify steps"
      },
      "text": {
        "type": "string",
        "description": "Expected text for verify/assert steps"
      },
      "match_type": {
        "type": "string",
        "description": "Text match mode for verify/assert steps"
      },
      "value_equals": {
        "type": "string",
        "description": "Expected exact field value"
      },
      "value_contains": {
        "type": "string",
        "description": "Expected partial field value"
      },
      "url_includes": {
        "type": "string",
        "description": "Substring expected in the URL"
      },
      "url_matches": {
        "type": "string",
        "description": "Regex expected to match the URL"
      },
      "active_selector": {
        "type": "string",
        "description": "Selector expected to be active/focused"
      },
      "count_at_least": {
        "type": "integer",
        "minimum": 0,
        "description": "Minimum matching count"
      },
      "count_equals": {
        "type": "integer",
        "minimum": 0,
        "description": "Exact matching count"
      },
      "all": {
        "type": "array",
        "items": { "type": "object" },
        "description": "All nested verify expectations must pass"
      },
      "any": {
        "type": "array",
        "items": { "type": "object" },
        "description": "At least one nested verify expectation must pass"
      },
      "step": {
        "type": "object",
        "description": "Explicit nested step for semantic_action"
      },
      "postcondition": {
        "type": "object",
        "description": "Postcondition for semantic_action"
      },
      "postcondition_required": {
        "type": "boolean",
        "description": "Whether semantic_action must enforce a postcondition"
      }
    }
  }
}
"#;

/// OpenAI function schema for completion
const COMPLETE_SCHEMA: &str = r#"
{
  "name": "complete",
  "description": "Indicate that the goal has been completed",
  "parameters": {
    "type": "object",
    "properties": {
      "extracted_data": {
        "type": "object",
        "description": "Final extracted data (if any)"
      },
      "reasoning": {
        "type": "string",
        "description": "Explanation of why the goal is complete"
      }
    }
  }
}
"#;

/// Two-tier planning: Group selection schema
const GROUP_SELECTION_SCHEMA: &str = r#"
{
  "name": "select_action_group",
  "description": "Select the most appropriate action group for the current task",
  "parameters": {
    "type": "object",
    "required": ["group", "reasoning"],
    "properties": {
      "group": {
        "type": "string",
        "enum": ["NAVIGATE", "ELEMENT", "SCROLL", "WAIT", "DATA", "ASSERT", "JS_EVAL", "UTIL"],
        "description": "The action group that best fits the current task"
      },
      "reasoning": {
        "type": "string",
        "description": "Brief explanation of why this group was selected"
      }
    }
  }
}
"#;

/// Two-tier planning: NAVIGATE group actions
const NAVIGATE_ACTIONS_SCHEMA: &str = r#"
{
  "name": "navigate_action",
  "description": "Execute a navigation action",
  "parameters": {
    "type": "object",
    "required": ["id", "name", "action"],
    "properties": {
      "id": {
        "type": "string",
        "description": "Unique identifier for this step"
      },
      "name": {
        "type": "string",
        "description": "Human-readable description of this step"
      },
      "action": {
        "type": "string",
        "enum": ["navigate_to_url", "open_new_tab", "switch_to_tab", "close_current_tab", "get_current_url"],
        "description": "Specific navigation action to perform"
      },
      "url": {
        "type": "string",
        "description": "URL to navigate to (REQUIRED for navigate_to_url action, optional for open_new_tab)"
      },
      "wait": {
        "type": "string",
        "description": "Wait condition after navigation (load, domcontentloaded, networkidle)"
      },
      "tab_id": {
        "type": "integer",
        "description": "Tab ID for tab operations"
      },
      "reasoning": {
        "type": "string",
        "description": "Brief explanation of why this specific action was chosen"
      }
    }
  }
}
"#;

/// Two-tier planning: ELEMENT group actions
const ELEMENT_ACTIONS_SCHEMA: &str = r#"
{
  "name": "element_action",
  "description": "Execute an element interaction action",
  "parameters": {
    "type": "object",
    "required": ["id", "name", "action", "selector"],
    "properties": {
      "id": {
        "type": "string",
        "description": "Unique identifier for this step"
      },
      "name": {
        "type": "string",
        "description": "Human-readable description of this step"
      },
      "action": {
        "type": "string",
        "enum": ["click_element", "dbl_click_element", "fill_input_field", "press_special_key", "hover_element", "select_option_in_dropdown", "upload_file", "drag_and_drop"],
        "description": "Specific element action to perform"
      },
      "selector": {
        "type": "string",
        "description": "CSS selector for the target element. Use specific, unique selectors when possible."
      },
      "value": {
        "type": "string",
        "description": "Text to enter (for fill_input_field) or option to select"
      },
      "key": {
        "type": "string",
        "description": "Special key to press (Enter, Tab, Escape, etc.)"
      },
      "file_path": {
        "type": "string",
        "description": "Path to file for upload"
      },
      "target_selector": {
        "type": "string",
        "description": "Target selector for drag and drop operations"
      },
      "reasoning": {
        "type": "string",
        "description": "Brief explanation of why this specific action and selector were chosen"
      }
    }
  }
}
"#;

/// Two-tier planning: SCROLL group actions
const SCROLL_ACTIONS_SCHEMA: &str = r#"
{
  "name": "scroll_action",
  "description": "Execute a scroll action",
  "parameters": {
    "type": "object",
    "required": ["id", "name", "action"],
    "properties": {
      "id": {
        "type": "string",
        "description": "Unique identifier for this step"
      },
      "name": {
        "type": "string",
        "description": "Human-readable description of this step"
      },
      "action": {
        "type": "string",
        "enum": ["scroll_window_to", "scroll_element_into_view", "infinite_scroll"],
        "description": "Specific scroll action to perform"
      },
      "selector": {
        "type": "string",
        "description": "CSS selector for element to scroll into view"
      },
      "x": {
        "type": "integer",
        "description": "X coordinate for window scroll"
      },
      "y": {
        "type": "integer",
        "description": "Y coordinate for window scroll"
      },
      "direction": {
        "type": "string",
        "enum": ["top", "bottom", "down_small", "up_small", "left", "right"],
        "description": "Scroll direction shorthand"
      },
      "item_selector": {
        "type": "string",
        "description": "CSS selector for items to count during infinite scroll"
      },
      "target_count": {
        "type": "integer",
        "minimum": 1,
        "description": "Target number of items for infinite scroll"
      },
      "max_cycles": {
        "type": "integer",
        "minimum": 1,
        "maximum": 100,
        "default": 30,
        "description": "Maximum scroll cycles to prevent infinite loops"
      },
      "reasoning": {
        "type": "string",
        "description": "Brief explanation of why this specific scroll action was chosen"
      }
    }
  }
}
"#;

/// Two-tier planning: WAIT group actions
const WAIT_ACTIONS_SCHEMA: &str = r#"
{
  "name": "wait_action",
  "description": "Execute a wait action",
  "parameters": {
    "type": "object",
    "required": ["id", "name", "action"],
    "properties": {
      "id": {
        "type": "string",
        "description": "Unique identifier for this step"
      },
      "name": {
        "type": "string",
        "description": "Human-readable description of this step"
      },
      "action": {
        "type": "string",
        "enum": ["wait_for_timeout", "wait_for_element", "wait_for_navigation", "wait_for_network_idle"],
        "description": "Specific wait action to perform"
      },
      "timeout_ms": {
        "type": "integer",
        "minimum": 100,
        "maximum": 60000,
        "description": "Timeout in milliseconds"
      },
      "selector": {
        "type": "string",
        "description": "CSS selector for element to wait for"
      },
      "condition": {
        "type": "string",
        "enum": ["visible", "hidden", "exists", "interactive", "stable"],
        "default": "visible",
        "description": "Wait condition for elements"
      },
      "url_pattern": {
        "type": "string",
        "description": "URL pattern to wait for during navigation"
      },
      "idle_time_ms": {
        "type": "integer",
        "minimum": 100,
        "maximum": 10000,
        "default": 500,
        "description": "Network idle time in milliseconds"
      },
      "max_wait_ms": {
        "type": "integer",
        "minimum": 1000,
        "maximum": 60000,
        "default": 30000,
        "description": "Maximum wait time in milliseconds"
      },
      "reasoning": {
        "type": "string",
        "description": "Brief explanation of why this specific wait action was chosen"
      }
    }
  }
}
"#;

/// Two-tier planning: DATA group actions
const DATA_ACTIONS_SCHEMA: &str = r#"
{
  "name": "data_action",
  "description": "Execute a data extraction action",
  "parameters": {
    "type": "object",
    "required": ["id", "name", "action"],
    "properties": {
      "id": {
        "type": "string",
        "description": "Unique identifier for this step"
      },
      "name": {
        "type": "string",
        "description": "Human-readable description of this step"
      },
      "action": {
        "type": "string",
        "enum": ["extract_structured_data", "get_element_text", "get_element_attribute", "get_element_value", "get_element_count", "take_screenshot", "get_page_source"],
        "description": "Specific data action to perform"
      },
      "selector": {
        "type": "string",
        "description": "CSS selector for the target element"
      },
      "item_selector": {
        "type": "string",
        "description": "CSS selector for container elements (for extract_structured_data)"
      },
      "fields": {
        "type": "array",
        "description": "Fields to extract (for extract_structured_data)",
        "items": {
          "type": "object",
          "required": ["name", "selector"],
          "properties": {
            "name": {
              "type": "string",
              "description": "Name of the field"
            },
            "selector": {
              "type": "string",
              "description": "CSS selector for the field"
            },
            "attribute": {
              "type": "string",
              "description": "HTML attribute to extract (optional)"
            },
            "post_processing": {
              "type": "array",
              "description": "Post-processing operations to apply",
              "items": {
                "type": "string",
                "enum": ["trim", "lowercase", "uppercase", "strip_html"]
              }
            }
          }
        }
      },
      "attribute": {
        "type": "string",
        "description": "HTML attribute to extract (for get_element_attribute)"
      },
      "full_page": {
        "type": "boolean",
        "default": false,
        "description": "Take full page screenshot"
      },
      "quality": {
        "type": "integer",
        "minimum": 1,
        "maximum": 100,
        "default": 90,
        "description": "Screenshot quality"
      },
      "format": {
        "type": "string",
        "enum": ["png", "jpeg"],
        "default": "png",
        "description": "Screenshot format"
      },
      "reasoning": {
        "type": "string",
        "description": "Brief explanation of why this specific data action was chosen"
      }
    }
  }
}
"#;

/// Two-tier planning: ASSERT group actions
const ASSERT_ACTIONS_SCHEMA: &str = r#"
{
  "name": "assert_action",
  "description": "Execute a verification or assertion action",
  "parameters": {
    "type": "object",
    "required": ["id", "name", "action"],
    "properties": {
      "id": {
        "type": "string",
        "description": "Unique identifier for this step"
      },
      "name": {
        "type": "string",
        "description": "Human-readable description of this step"
      },
      "action": {
        "type": "string",
        "enum": ["assert_selector_state", "assert_text_in_element", "assert_url_matches", "verify_ui_change"],
        "description": "Specific assertion or verification action to perform"
      },
      "selector": {
        "type": "string",
        "description": "CSS selector for the target element"
      },
      "condition": {
        "type": "string",
        "description": "State or condition to assert"
      },
      "text": {
        "type": "string",
        "description": "Expected text"
      },
      "match_type": {
        "type": "string",
        "description": "Text or URL matching mode"
      },
      "url_pattern": {
        "type": "string",
        "description": "URL pattern to assert"
      },
      "value_equals": {
        "type": "string",
        "description": "Expected exact field value"
      },
      "value_contains": {
        "type": "string",
        "description": "Expected partial field value"
      },
      "url_includes": {
        "type": "string",
        "description": "Substring expected in the current URL"
      },
      "url_matches": {
        "type": "string",
        "description": "Regex expected to match the current URL"
      },
      "active_selector": {
        "type": "string",
        "description": "Selector expected to be the active element"
      },
      "count_at_least": {
        "type": "integer",
        "minimum": 0,
        "description": "Minimum matching element count"
      },
      "count_equals": {
        "type": "integer",
        "minimum": 0,
        "description": "Exact matching element count"
      },
      "all": {
        "type": "array",
        "items": { "type": "object" },
        "description": "All nested expectations must pass"
      },
      "any": {
        "type": "array",
        "items": { "type": "object" },
        "description": "Any nested expectation may pass"
      },
      "timeout_ms": {
        "type": "integer",
        "minimum": 0,
        "description": "Timeout in milliseconds"
      },
      "reasoning": {
        "type": "string",
        "description": "Brief explanation of why this assertion was chosen"
      }
    }
  }
}
"#;

/// Two-tier planning: JS_EVAL group actions
const JS_EVAL_ACTIONS_SCHEMA: &str = r#"
{
  "name": "js_eval_action",
  "description": "Execute JavaScript-based debugging or extraction",
  "parameters": {
    "type": "object",
    "required": ["id", "name", "action", "script"],
    "properties": {
      "id": {
        "type": "string",
        "description": "Unique identifier for this step"
      },
      "name": {
        "type": "string",
        "description": "Human-readable description of this step"
      },
      "action": {
        "type": "string",
        "enum": ["execute_javascript", "eval_main_world", "eval_isolated_world"],
        "description": "Specific JS evaluation action to perform"
      },
      "script": {
        "type": "string",
        "description": "JavaScript source to execute"
      },
      "args": {
        "type": "array",
        "description": "Arguments for the script"
      },
      "return_value": {
        "type": "boolean",
        "description": "Whether to return the script result"
      },
      "world": {
        "type": "string",
        "enum": ["isolated", "main"],
        "description": "Execution world for execute_javascript compatibility"
      },
      "timeout_ms": {
        "type": "integer",
        "minimum": 0,
        "description": "Timeout in milliseconds"
      },
      "reasoning": {
        "type": "string",
        "description": "Brief explanation of why JS evaluation is required"
      }
    }
  }
}
"#;

/// Two-tier planning: UTIL group actions
const UTIL_ACTIONS_SCHEMA: &str = r#"
{
  "name": "util_action",
  "description": "Execute debugging, inspection, or semantic utility actions",
  "parameters": {
    "type": "object",
    "required": ["id", "name", "action"],
    "properties": {
      "id": {
        "type": "string",
        "description": "Unique identifier for this step"
      },
      "name": {
        "type": "string",
        "description": "Human-readable description of this step"
      },
      "action": {
        "type": "string",
        "enum": ["inspect_element", "inspect_click_surface", "capture_ui_bundle", "read_field_value", "semantic_action"],
        "description": "Specific utility action to perform"
      },
      "selector": {
        "type": "string",
        "description": "CSS selector for the target element"
      },
      "frame_id": {
        "type": "string",
        "description": "Frame identifier when needed"
      },
      "include_ancestors": {
        "type": "boolean",
        "description": "Include ancestors when inspecting elements"
      },
      "include_shadow_path": {
        "type": "boolean",
        "description": "Include shadow path when inspecting elements"
      },
      "include_dom_snapshot": {
        "type": "boolean",
        "description": "Include DOM snapshot in UI bundle"
      },
      "include_screenshot": {
        "type": "boolean",
        "description": "Include screenshot in UI bundle"
      },
      "annotate": {
        "type": "boolean",
        "description": "Annotate screenshots when supported"
      },
      "max_elements": {
        "type": "integer",
        "minimum": 1,
        "description": "Max elements to include in UI bundle"
      },
      "value": {
        "type": "string",
        "description": "Text value for semantic_action type steps"
      },
      "key": {
        "type": "string",
        "description": "Key value for semantic_action keypress steps"
      },
      "semantic_action": {
        "type": "string",
        "description": "Underlying semantic action (click, type, press_key, hover)"
      },
      "step": {
        "type": "object",
        "description": "Explicit nested step for semantic_action"
      },
      "postcondition": {
        "type": "object",
        "description": "Postcondition to verify after semantic_action"
      },
      "postcondition_required": {
        "type": "boolean",
        "description": "Whether semantic_action requires postcondition success"
      },
      "timeout_ms": {
        "type": "integer",
        "minimum": 0,
        "description": "Timeout in milliseconds"
      },
      "reasoning": {
        "type": "string",
        "description": "Brief explanation of why this utility action was chosen"
      }
    }
  }
}
"#;

fn try_parse_json_from_text(text: &str) -> Option<Value> {
    fn extract_first_balanced_json(text: &str) -> Option<Value> {
        let bytes = text.as_bytes();

        // Scan for a JSON object/array start, then find the first balanced close while
        // respecting strings and escapes. This is intentionally permissive because CLI
        // wrappers and some models prepend/append prose around the JSON payload.
        let mut i = 0usize;
        while i < bytes.len() {
            let (open, close) = match bytes[i] {
                b'{' => (b'{', b'}'),
                b'[' => (b'[', b']'),
                _ => {
                    i += 1;
                    continue;
                }
            };

            let start = i;
            let mut depth: i32 = 0;
            let mut in_string = false;
            let mut escaped = false;
            let mut j = i;
            while j < bytes.len() {
                let b = bytes[j];
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if b == b'\\' {
                        escaped = true;
                    } else if b == b'"' {
                        in_string = false;
                    }
                } else {
                    if b == b'"' {
                        in_string = true;
                    } else if b == open {
                        depth += 1;
                    } else if b == close {
                        depth -= 1;
                        if depth == 0 {
                            // Candidate substring is bytes[start..=j]
                            if let Ok(v) = serde_json::from_slice::<Value>(&bytes[start..=j]) {
                                return Some(v);
                            }
                            break;
                        }
                    }
                }
                j += 1;
            }

            // This start didn't yield a parseable JSON value; keep scanning.
            i = start + 1;
        }

        None
    }

    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    // 1) If the whole string is valid JSON, use it.
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        // If the model returned a JSON *string* that itself contains JSON, unwrap it.
        if let Value::String(s) = &v {
            if let Some(inner) = try_parse_json_from_text(s) {
                return Some(inner);
            }
        }
        return Some(v);
    }

    // 2) Otherwise, try to extract the first balanced JSON object/array from the text.
    extract_first_balanced_json(text)
}

impl LLMClient {
    /// Create a new LLM client with a specific provider (for testing)
    pub fn with_provider(provider: Box<dyn LLMProvider>) -> Self {
        Self {
            provider: Arc::from(provider),
            temperature: 1.0, // Use 1.0 for better model compatibility
        }
    }

    pub fn new(config: &PlanConfig) -> PlanResult<Self> {
        // Determine provider type
        let provider_type = ProviderType::from_str(&config.llm_provider).ok_or_else(|| {
            PlanError::LLMError(format!("Unknown LLM provider: {}", config.llm_provider))
        })?;

        info!(
            "Initializing LLM client with provider: {:?}, model: {}",
            provider_type, config.model
        );

        // Create appropriate provider
        let provider: Arc<dyn LLMProvider> = match provider_type {
            ProviderType::OpenAI => {
                // For backward compatibility, try llm_api_key first, then openai_api_key
                let api_key = if !config.llm_api_key.is_empty() {
                    config.llm_api_key.clone()
                } else {
                    config.openai_api_key.clone()
                };

                Arc::new(OpenAIClient::new(
                    api_key,
                    config.model.clone(),
                    config.llm_timeout,
                )?)
            }
            ProviderType::Gemini => Arc::new(GeminiClient::new(
                config.llm_api_key.clone(),
                config.model.clone(),
                config.llm_timeout,
            )?),
            ProviderType::Anthropic => Arc::new(AnthropicClient::new(
                config.llm_api_key.clone(),
                config.model.clone(),
                config.llm_timeout,
            )?),
            ProviderType::Groq => Arc::new(GroqClient::new(
                config.llm_api_key.clone(),
                config.model.clone(),
                config.llm_timeout,
            )?),
            ProviderType::Dummy => {
                Arc::new(crate::dummy_client::DummyClient::new(config.model.clone()))
            }
            ProviderType::ClaudeCli => Arc::new(CliClient::new(
                CliKind::Claude,
                config.model.clone(),
                config.llm_timeout,
            )),
            ProviderType::GeminiCli => Arc::new(CliClient::new(
                CliKind::Gemini,
                config.model.clone(),
                config.llm_timeout,
            )),
            ProviderType::CodexCli => Arc::new(CliClient::new(
                CliKind::Codex,
                config.model.clone(),
                config.llm_timeout,
            )),
        };

        Ok(Self {
            provider,
            temperature: config.temperature,
        })
    }

    /// Simple constructor for external use with just API key (defaults to OpenAI)
    pub fn new_simple(api_key: String) -> PlanResult<Self> {
        // Get model from environment variable
        let model =
            std::env::var("OPENAI_MODEL_PLANNING").unwrap_or_else(|_| "gpt-4o-mini".to_string());

        let provider = Arc::new(OpenAIClient::new(api_key, model, 30)?);

        Ok(Self {
            provider,
            temperature: 0.7,
        })
    }

    /// Simple chat method for external use
    pub async fn chat(
        &self,
        messages: Vec<Value>,
        temperature: Option<f32>,
    ) -> PlanResult<LLMChatResponse> {
        debug!(
            "Simple chat request to LLM via {}",
            self.provider.provider_name()
        );

        let temp = temperature.unwrap_or(self.temperature);
        let content = self.provider.simple_chat(messages, Some(temp)).await?;

        Ok(LLMChatResponse { content })
    }

    /// Chat method that enforces JSON response format (uses provider's chat_completion)
    pub async fn chat_json(
        &self,
        messages: Vec<Value>,
        temperature: Option<f32>,
    ) -> PlanResult<Value> {
        debug!(
            "JSON chat request to LLM via {}",
            self.provider.provider_name()
        );

        let temp = temperature.unwrap_or(self.temperature);

        // Always use OpenAI Responses API when provider is OpenAI (newer models default to Responses)
        let is_openai = self.provider.provider_name() == "OpenAI";
        if is_openai {
            // Build one consolidated input entry per message for /v1/responses
            let mut input: Vec<Value> = Vec::new();
            for m in &messages {
                let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                let text = m.get("content").and_then(|c| c.as_str()).unwrap_or("");
                // Responses API expects 'input_text' blocks
                let effective_role = if role == "system" { "user" } else { role };
                input.push(json!({
                    "role": effective_role,
                    "content": [ {"type": "input_text", "text": text} ]
                }));
            }

            // Try Responses API; on 4xx parameter errors, fall back to chat completions
            let response = match self
                .provider
                .responses_completion(input.clone(), temp, None, None, None, None)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    let msg = format!("{}", e);
                    if msg.to_lowercase().contains("unsupported parameter")
                        || msg.to_lowercase().contains("responses api failed")
                        || msg.to_lowercase().contains("invalid_request_error")
                    {
                        debug!(
                            "Responses API error: {}. Falling back to chat completions.",
                            msg
                        );
                        // Build chat-style messages from input
                        let mut chat_msgs: Vec<Value> = Vec::new();
                        for item in &input {
                            let role = item.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                            let content_txt = item
                                .get("content")
                                .and_then(|c| c.as_array())
                                .and_then(|a| a.first())
                                .and_then(|b| b.get("text"))
                                .and_then(|t| t.as_str())
                                .unwrap_or("");
                            chat_msgs.push(json!({"role": role, "content": content_txt}));
                        }
                        // Add JSON instruction to last message
                        if let Some(last) = chat_msgs.last_mut() {
                            if let Some(c) = last
                                .get_mut("content")
                                .and_then(|v| v.as_str().map(|s| s.to_string()))
                            {
                                *last = json!({"role": last["role"].as_str().unwrap_or("user"),
                                    "content": format!("{}\n\nRespond ONLY with a valid JSON object.", c)});
                            }
                        }
                        // Call chat
                        let chat_resp = self
                            .provider
                            .chat_completion(chat_msgs, temp, None, None, None)
                            .await?;
                        // Wrap to mimic Responses output
                        json!({"output_text": chat_resp.get("choices").and_then(|c| c.get(0)).and_then(|c| c.get("message")).and_then(|m| m.get("content")).and_then(|s| s.as_str()).unwrap_or("")})
                    } else {
                        return Err(e);
                    }
                }
            };

            append_llm_raw_response_log("openai.responses response", &response);

            // Try to extract JSON from Responses result
            // 1) Prefer output_text if present
            if let Some(txt) = response.get("output_text").and_then(|v| v.as_str()) {
                if let Some(v) = try_parse_json_from_text(txt) {
                    return Ok(v);
                }
            }
            // 2) Search output array for message.content[].type == output_text
            if let Some(arr) = response.get("output").and_then(|v| v.as_array()) {
                for item in arr {
                    if item.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                        if let Some(txt) = item.get("text").and_then(|t| t.as_str()) {
                            if let Some(v) = try_parse_json_from_text(txt) {
                                return Ok(v);
                            }
                        }
                    }
                    if item.get("type").and_then(|t| t.as_str()) == Some("message") {
                        if let Some(parts) = item.get("content").and_then(|c| c.as_array()) {
                            for p in parts {
                                if p.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                                    if let Some(txt) = p.get("text").and_then(|t| t.as_str()) {
                                        if let Some(v) = try_parse_json_from_text(txt) {
                                            return Ok(v);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            return Err(PlanError::LLMError(
                "Failed to parse JSON from Responses API".to_string(),
            ));
        }

        // Fallback: Chat Completions (existing path)
        // Add JSON instruction to the last user message
        let mut modified_messages = messages.clone();
        if let Some(last_msg) = modified_messages.last_mut() {
            if let Some(content) = last_msg.get_mut("content").and_then(|c| c.as_str()) {
                let new_content = format!(
                    "{}\n\nIMPORTANT: You MUST respond with valid JSON only. No markdown, no explanations, just the JSON object.",
                    content
                );
                *last_msg.get_mut("content").unwrap() = json!(new_content);
            }
        }

        // Request a JSON-capable response via chat
        let response = self
            .provider
            .chat_completion(modified_messages.clone(), temp, None, None, None)
            .await?;

        append_llm_raw_response_log("openai.chat_completion response", &response);

        // Try tool calls first (if any)
        if let Some(tool_calls) = response
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("tool_calls"))
            .and_then(|t| t.as_array())
        {
            if let Some(first) = tool_calls.first() {
                if let Some(args_str) = first
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|a| a.as_str())
                {
                    if let Ok(val) = serde_json::from_str::<Value>(args_str) {
                        return Ok(val);
                    }
                }
            }
        }

        // Fallback to message content (if no tool call is returned)
        let content = response
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
            .to_string();

        if content.is_empty() {
            append_llm_debug_log(LLM_RAW_RESPONSE_LOG, |buffer| {
                let _ = writeln!(
                    buffer,
                    "{}",
                    serde_json::json!({
                        "timestamp": chrono::Utc::now().to_rfc3339(),
                        "level": "ERROR",
                        "component": "llm_raw",
                        "message": "empty content from LLM; full response logged above"
                    })
                );
            });
            error!("No JSON structure found. Raw content: \n{}", content);
            return Err(PlanError::LLMError(
                "No JSON structure found in response: empty content".to_string(),
            ));
        }

        if let Some(v) = try_parse_json_from_text(&content) {
            return Ok(v);
        }

        // Some providers (notably CLI wrappers) occasionally ignore the "JSON only" instruction.
        // Do one lightweight retry *in the same conversation* to avoid failing the whole run.
        // This is intentionally bounded to prevent loops/cost blowups.
        let mut retry_messages = modified_messages;
        retry_messages.push(json!({
            "role": "user",
            "content": "FORMAT ERROR: Your previous answer was not valid JSON. Respond again with ONLY valid JSON (no prose, no markdown)."
        }));

        let retry_response = self
            .provider
            .chat_completion(retry_messages, temp, None, None, None)
            .await?;

        let retry_content = retry_response
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
            .to_string();

        if let Some(v) = try_parse_json_from_text(&retry_content) {
            return Ok(v);
        }

        error!("Failed to parse JSON. Raw content: {}", content);
        Err(PlanError::LLMError(
            "Failed to parse JSON response: non-JSON content returned by provider".to_string(),
        ))
    }

    /// Plan the next step based on current state
    pub async fn plan_next_step(&self, messages: Vec<Value>) -> PlanResult<LLMResponse> {
        debug!("Requesting next step from LLM");

        let log_path =
            append_llm_message_log("FULL MESSAGES TO LLM (Planning)", Some(&messages), None);

        // Print summary to terminal
        println!(
            "\n {} {}",
            ansi_term::Colour::Cyan.bold().paint("Sending to LLM"),
            ansi_term::Colour::Yellow.dimmed().paint(format!(
                "[Planning] Size: {} chars",
                serde_json::to_string(&messages).unwrap_or_default().len()
            ))
        );
        if let Some(log_path) = &log_path {
            println!(
                "[NOTE] Debug log written to: {}",
                ansi_term::Colour::Blue.paint(log_path.display().to_string())
            );
        }

        let tools = vec![
            json!({
                "type": "function",
                "function": serde_json::from_str::<Value>(BROWSER_STEP_SCHEMA)
                    .map_err(PlanError::SerializationError)?
            }),
            json!({
                "type": "function",
                "function": serde_json::from_str::<Value>(COMPLETE_SCHEMA)
                    .map_err(PlanError::SerializationError)?
            }),
        ];

        let response_json = self
            .provider
            .chat_completion(
                messages.clone(),
                self.temperature,
                Some(tools),
                Some(json!("auto")),
                Some(1000),
            )
            .await?;

        append_llm_message_log(
            "[BOT] FULL LLM RESPONSE (Planning)",
            None,
            Some(&response_json),
        );

        // Print summary to terminal
        println!(
            "[BOT] {} {}",
            ansi_term::Colour::Green.bold().paint("Received from LLM"),
            ansi_term::Colour::Yellow.dimmed().paint(format!(
                "[Planning] Response size: {} chars",
                response_json.to_string().len()
            ))
        );

        self.parse_llm_response(response_json).await
    }

    /// Parse OpenAI response into LLMResponse
    async fn parse_llm_response(&self, response: Value) -> PlanResult<LLMResponse> {
        // Check for API errors first
        if let Some(error) = response.get("error") {
            let error_msg = error["message"].as_str().unwrap_or("Unknown API error");
            let error_type = error["type"].as_str().unwrap_or("unknown_error");

            // For invalid API key or authentication errors, fail immediately
            if error_type == "invalid_request_error" && error_msg.contains("API key") {
                return Err(PlanError::LLMError("Invalid OpenAI API key. Please check your OPENAI_API_KEY environment variable.".to_string()));
            }

            return Err(PlanError::LLMError(format!(
                "OpenAI API error: {}",
                error_msg
            )));
        }

        let choices = response["choices"]
            .as_array()
            .ok_or_else(|| PlanError::LLMError("No choices in response".to_string()))?;

        if choices.is_empty() {
            return Err(PlanError::LLMError("Empty choices array".to_string()));
        }

        let message = &choices[0]["message"];

        // Check if there are tool calls (new API format)
        if let Some(tool_calls) = message["tool_calls"].as_array() {
            if let Some(tool_call) = tool_calls.first() {
                let function = &tool_call["function"];
                let function_name = function["name"]
                    .as_str()
                    .ok_or_else(|| PlanError::LLMError("No function name".to_string()))?;

                let arguments_str = function["arguments"]
                    .as_str()
                    .ok_or_else(|| PlanError::LLMError("No function arguments".to_string()))?;

                let arguments: Value =
                    serde_json::from_str(arguments_str).map_err(PlanError::SerializationError)?;

                match function_name {
                    "browser_step" => {
                        let step = self.parse_browser_step(arguments)?;
                        Ok(LLMResponse {
                            is_complete: false,
                            next_step: Some(step),
                            extracted_data: None,
                            reasoning: None,
                        })
                    }
                    "complete" => Ok(LLMResponse {
                        is_complete: true,
                        next_step: None,
                        extracted_data: arguments.get("extracted_data").cloned(),
                        reasoning: arguments
                            .get("reasoning")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                    }),
                    _ => Err(PlanError::LLMError(format!(
                        "Unknown function: {}",
                        function_name
                    ))),
                }
            } else {
                Err(PlanError::LLMError("No tool calls found".to_string()))
            }
        } else {
            // No function call, check for message content
            let content = message["content"].as_str().unwrap_or("");
            Ok(LLMResponse {
                is_complete: true,
                next_step: None,
                extracted_data: None,
                reasoning: Some(content.to_string()),
            })
        }
    }

    /// Parse browser step from function arguments
    fn parse_browser_step(&self, args: Value) -> PlanResult<Step> {
        debug!(
            "Parsing browser step from arguments: {}",
            serde_json::to_string_pretty(&args)
                .unwrap_or_else(|_| "Failed to serialize args".to_string())
        );

        let id = args["id"]
            .as_str()
            .ok_or_else(|| PlanError::InvalidStep("Missing step id".to_string()))?
            .to_string();

        let name = args["name"]
            .as_str()
            .ok_or_else(|| PlanError::InvalidStep("Missing step name".to_string()))?
            .to_string();

        let step_type = args["type"]
            .as_str()
            .ok_or_else(|| PlanError::InvalidStep("Missing step type".to_string()))?;

        debug!("Parsing step: id={}, name={}, type={}", id, name, step_type);

        let kind = match step_type {
            "navigate_to_url" => {
                let url = args["url"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("Navigate step missing url".to_string())
                })?;

                //  URL VALIDATION: Fix malformed URLs from LLM
                let validated_url = if url.is_empty() {
                    return Err(PlanError::InvalidStep(
                        "Cannot navigate to empty URL".to_string(),
                    ));
                } else if url.starts_with("http://") || url.starts_with("https://") {
                    // Already has protocol, use as-is
                    url.to_string()
                } else if url.contains(".") && !url.contains("/") {
                    // Looks like a domain name, add https://
                    format!("https://{}", url)
                } else {
                    return Err(PlanError::InvalidStep(format!(
                        "Invalid URL format: '{}'",
                        url
                    )));
                };

                debug!(
                    "Creating navigate step to: {} (validated from: {})",
                    validated_url, url
                );
                rzn_core::StepKind::NavigateToUrl {
                    url: validated_url,
                    wait: Some("domcontentloaded".to_string()),
                }
            }
            "click_element" => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("Click step missing selector".to_string())
                })?;
                debug!("Creating click step with selector: {}", selector);
                rzn_core::StepKind::ClickElement {
                    selector: selector.to_string(),
                    frame_id: None,
                    random_offset: None,
                    timeout_ms: Some(5000),
                }
            }
            "fill_input_field" => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("Fill step missing selector".to_string())
                })?;
                let value = args["value"]
                    .as_str()
                    .ok_or_else(|| PlanError::InvalidStep("Fill step missing value".to_string()))?;
                debug!("Creating fill step: selector={}, value={}", selector, value);

                rzn_core::StepKind::FillInputField {
                    selector: selector.to_string(),
                    value: value.to_string(),
                    frame_id: None,
                    clear_first: Some(true),
                    simulate_typing: Some(true), // Enable realistic typing
                    delay_ms: Some(50),          // Realistic typing delay
                    timeout_ms: Some(10000),     // Increased timeout
                }
            }
            "press_key" => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("PressKey step missing selector".to_string())
                })?;
                let key = args["key"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("PressKey step missing key".to_string())
                })?;
                debug!(
                    "Creating press_key step: selector={}, key={}",
                    selector, key
                );
                rzn_core::StepKind::PressSpecialKey {
                    key: key.to_string(),
                    selector: Some(selector.to_string()),
                    frame_id: None,
                    timeout_ms: Some(5000),
                }
            }
            "hover_element" => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("Hover step missing selector".to_string())
                })?;
                debug!("Creating hover step with selector: {}", selector);
                rzn_core::StepKind::HoverElement {
                    selector: selector.to_string(),
                    frame_id: None,
                    random_offset: None,
                    timeout_ms: None,
                }
            }
            "wait_for_selector" => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("WaitForSelector step missing selector".to_string())
                })?;
                let timeout = args["timeout"].as_u64().unwrap_or(5000);
                debug!(
                    "Creating wait_for_selector step: selector={}, timeout={}",
                    selector, timeout
                );
                rzn_core::StepKind::WaitForElement {
                    selector: selector.to_string(),
                    frame_id: None,
                    condition: Some("visible".to_string()),
                    timeout_ms: Some(timeout as u32),
                }
            }
            "wait_timeout" => {
                let timeout = args["timeout"].as_u64().ok_or_else(|| {
                    PlanError::InvalidStep("WaitForTimeout step missing timeout".to_string())
                })?;
                debug!("Creating wait_for_timeout step: timeout={}", timeout);
                rzn_core::StepKind::WaitForTimeout {
                    timeout_ms: timeout as u32,
                }
            }
            "extract_structured_data" => {
                let item_selector = args["item_selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("Extract step missing item_selector".to_string())
                })?;

                let fields_array = args["fields"].as_array().ok_or_else(|| {
                    PlanError::InvalidStep("Extract step missing fields".to_string())
                })?;

                debug!(
                    "Creating extract step: item_selector={}, fields_count={}",
                    item_selector,
                    fields_array.len()
                );

                let fields: PlanResult<Vec<rzn_core::FieldSpec>> = fields_array
                    .iter()
                    .map(|field| {
                        let name = field["name"].as_str().ok_or_else(|| {
                            PlanError::InvalidStep("Field missing name".to_string())
                        })?;
                        let selector = field["selector"].as_str().ok_or_else(|| {
                            PlanError::InvalidStep("Field missing selector".to_string())
                        })?;
                        let attribute = field["attribute"].as_str().map(|s| s.to_string());
                        let post_processing: Vec<String> = field["post_processing"]
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .map(|v| v.as_str().unwrap_or("").to_string())
                                    .collect()
                            })
                            .unwrap_or_default();

                        debug!(
                            "  Field: name={}, selector={}, attribute={:?}, post_processing={:?}",
                            name, selector, attribute, post_processing
                        );

                        Ok(rzn_core::FieldSpec {
                            name: name.to_string(),
                            selector: selector.to_string(),
                            attribute,
                            post_processing,
                        })
                    })
                    .collect();

                rzn_core::StepKind::ExtractStructuredData {
                    item_selector: item_selector.to_string(),
                    limit: None,
                    fields: fields?,
                    frame_id: None,
                    extraction_type: None,
                }
            }
            "execute_javascript" => rzn_core::StepKind::ExecuteJavascript {
                script: args["script"]
                    .as_str()
                    .ok_or_else(|| {
                        PlanError::InvalidStep("ExecuteJavascript step missing script".to_string())
                    })?
                    .to_string(),
                args: args["args"].as_array().cloned(),
                return_value: args["return_value"].as_bool().unwrap_or(true),
                world: args["world"].as_str().map(|s| s.to_string()),
                timeout_ms: args
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .or_else(|| args.get("timeout").and_then(|v| v.as_u64()))
                    .map(|v| v as u32),
            },
            "eval_main_world" => rzn_core::StepKind::EvalMainWorld {
                script: args["script"]
                    .as_str()
                    .ok_or_else(|| {
                        PlanError::InvalidStep("EvalMainWorld step missing script".to_string())
                    })?
                    .to_string(),
                args: args["args"].as_array().cloned(),
                return_value: args["return_value"].as_bool().unwrap_or(true),
                timeout_ms: args
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .or_else(|| args.get("timeout").and_then(|v| v.as_u64()))
                    .map(|v| v as u32),
            },
            "eval_isolated_world" => rzn_core::StepKind::EvalIsolatedWorld {
                script: args["script"]
                    .as_str()
                    .ok_or_else(|| {
                        PlanError::InvalidStep("EvalIsolatedWorld step missing script".to_string())
                    })?
                    .to_string(),
                args: args["args"].as_array().cloned(),
                return_value: args["return_value"].as_bool().unwrap_or(true),
                timeout_ms: args
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .or_else(|| args.get("timeout").and_then(|v| v.as_u64()))
                    .map(|v| v as u32),
            },
            "inspect_element" => rzn_core::StepKind::InspectElement {
                selector: args["selector"]
                    .as_str()
                    .ok_or_else(|| {
                        PlanError::InvalidStep("InspectElement step missing selector".to_string())
                    })?
                    .to_string(),
                frame_id: args["frame_id"].as_str().map(|s| s.to_string()),
                include_ancestors: args["include_ancestors"].as_bool(),
                include_shadow_path: args["include_shadow_path"].as_bool(),
                timeout_ms: args
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .or_else(|| args.get("timeout").and_then(|v| v.as_u64()))
                    .map(|v| v as u32),
            },
            "inspect_click_surface" => rzn_core::StepKind::InspectClickSurface {
                selector: args["selector"]
                    .as_str()
                    .ok_or_else(|| {
                        PlanError::InvalidStep(
                            "InspectClickSurface step missing selector".to_string(),
                        )
                    })?
                    .to_string(),
                frame_id: args["frame_id"].as_str().map(|s| s.to_string()),
                timeout_ms: args
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .or_else(|| args.get("timeout").and_then(|v| v.as_u64()))
                    .map(|v| v as u32),
            },
            "capture_ui_bundle" => rzn_core::StepKind::CaptureUiBundle {
                selector: args["selector"].as_str().map(|s| s.to_string()),
                include_dom_snapshot: args["include_dom_snapshot"].as_bool(),
                include_screenshot: args["include_screenshot"].as_bool(),
                annotate: args["annotate"].as_bool(),
                max_elements: args["max_elements"].as_u64().map(|v| v as u32),
                timeout_ms: args
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .or_else(|| args.get("timeout").and_then(|v| v.as_u64()))
                    .map(|v| v as u32),
            },
            "verify_ui_change" => rzn_core::StepKind::VerifyUiChange {
                selector: args["selector"].as_str().map(|s| s.to_string()),
                condition: args["condition"].as_str().map(|s| s.to_string()),
                text: args["text"].as_str().map(|s| s.to_string()),
                match_type: args["match_type"].as_str().map(|s| s.to_string()),
                value_equals: args["value_equals"].as_str().map(|s| s.to_string()),
                value_contains: args["value_contains"].as_str().map(|s| s.to_string()),
                url_includes: args["url_includes"].as_str().map(|s| s.to_string()),
                url_matches: args["url_matches"].as_str().map(|s| s.to_string()),
                active_selector: args["active_selector"].as_str().map(|s| s.to_string()),
                count_at_least: args["count_at_least"].as_u64().map(|v| v as u32),
                count_equals: args["count_equals"].as_u64().map(|v| v as u32),
                all: args["all"].as_array().cloned(),
                any: args["any"].as_array().cloned(),
                timeout_ms: args
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .or_else(|| args.get("timeout").and_then(|v| v.as_u64()))
                    .map(|v| v as u32),
            },
            "read_field_value" => rzn_core::StepKind::ReadFieldValue {
                selector: args["selector"]
                    .as_str()
                    .ok_or_else(|| {
                        PlanError::InvalidStep("ReadFieldValue step missing selector".to_string())
                    })?
                    .to_string(),
                frame_id: args["frame_id"].as_str().map(|s| s.to_string()),
                timeout_ms: args
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .or_else(|| args.get("timeout").and_then(|v| v.as_u64()))
                    .map(|v| v as u32),
            },
            "semantic_action" => rzn_core::StepKind::SemanticAction {
                action: args["action"]
                    .as_str()
                    .ok_or_else(|| {
                        PlanError::InvalidStep("SemanticAction step missing action".to_string())
                    })?
                    .to_string(),
                selector: args["selector"].as_str().map(|s| s.to_string()),
                value: args["value"].as_str().map(|s| s.to_string()),
                key: args["key"].as_str().map(|s| s.to_string()),
                step: args.get("step").cloned(),
                postcondition: args.get("postcondition").cloned(),
                postcondition_required: args["postcondition_required"].as_bool(),
                timeout_ms: args
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .or_else(|| args.get("timeout").and_then(|v| v.as_u64()))
                    .map(|v| v as u32),
            },
            _ => {
                error!("Unknown step type: {}", step_type);
                return Err(PlanError::InvalidStep(format!(
                    "Unknown step type: {}",
                    step_type
                )));
            }
        };

        let step_def = Step { id, name, kind };
        debug!("Successfully created step: {:?}", step_def);
        Ok(step_def)
    }

    /// Two-tier planning: First tier - select action group
    pub async fn plan_group(&self, messages: Vec<Value>) -> PlanResult<GroupSelection> {
        debug!("Requesting action group selection from LLM");

        append_llm_message_log("FULL MESSAGES TO LLM (Tier 1)", Some(&messages), None);

        println!(
            " {} {}",
            ansi_term::Colour::Cyan.bold().paint("Sending to LLM"),
            ansi_term::Colour::Yellow.dimmed().paint(format!(
                "[Tier 1] Size: {} chars",
                serde_json::to_string(&messages).unwrap_or_default().len()
            ))
        );

        // Pretty print the message content
        println!("\n{}", "━".repeat(80));
        println!(
            " {} {}",
            ansi_term::Colour::Cyan
                .bold()
                .paint("MESSAGE TO LLM - TIER 1"),
            ansi_term::Colour::Yellow
                .dimmed()
                .paint(format!("[{}]", chrono::Local::now().format("%H:%M:%S")))
        );
        println!("{}", "─".repeat(60));

        for message in &messages {
            if let Some(role) = message.get("role").and_then(|r| r.as_str()) {
                println!(
                    "  {}: {}",
                    ansi_term::Colour::Purple.bold().paint("Role"),
                    ansi_term::Colour::Green.paint(role)
                );
            }

            if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
                let lines: Vec<&str> = content.lines().collect();
                for line in lines {
                    if line.starts_with("Goal:") {
                        println!(
                            "[TARGET] {}: {}",
                            ansi_term::Colour::Cyan.paint("Goal"),
                            ansi_term::Colour::Yellow
                                .bold()
                                .paint(line.strip_prefix("Goal:").unwrap_or("").trim())
                        );
                    } else if line.starts_with("Current State:") || line.starts_with("Current URL:")
                    {
                        println!(
                            " {}: {}",
                            ansi_term::Colour::Cyan.paint("State"),
                            ansi_term::Colour::Blue
                                .paint(line.split(':').nth(1).unwrap_or("").trim())
                        );
                    } else if line.starts_with("PAGE_HTML") {
                        println!(
                            " {}",
                            ansi_term::Colour::Cyan.paint("DOM Content: [provided]")
                        );
                    } else if line.starts_with("Execution history:") {
                        println!(
                            "📜 {}",
                            ansi_term::Colour::Cyan.paint("History: [provided]")
                        );
                    } else if line.starts_with("What should be") {
                        println!("❓ {}", ansi_term::Colour::Cyan.paint(line));
                    }
                }
            }
        }

        println!("{}", "━".repeat(80));

        let tools = vec![json!({
            "type": "function",
            "function": serde_json::from_str::<Value>(GROUP_SELECTION_SCHEMA)
                .map_err(PlanError::SerializationError)?
        })];

        let response_json = self
            .provider
            .chat_completion(
                messages.clone(),
                self.temperature,
                Some(tools),
                Some(json!({"type": "function", "function": {"name": "select_action_group"}})),
                Some(500),
            )
            .await?;

        append_llm_message_log(
            "[BOT] FULL LLM RESPONSE (Tier 1)",
            None,
            Some(&response_json),
        );

        println!(
            "[BOT] {} {}",
            ansi_term::Colour::Green.bold().paint("Received from LLM"),
            ansi_term::Colour::Yellow.dimmed().paint(format!(
                "[Tier 1] Response size: {} chars",
                response_json.to_string().len()
            ))
        );

        // Pretty print the response
        println!("\n{}", "━".repeat(80));
        println!(
            "[BOT] {} {}",
            ansi_term::Colour::Green
                .bold()
                .paint("LLM RESPONSE - TIER 1"),
            ansi_term::Colour::Yellow
                .dimmed()
                .paint(format!("[{}]", chrono::Local::now().format("%H:%M:%S")))
        );
        println!("{}", "─".repeat(60));

        if let Some(choices) = response_json.get("choices").and_then(|c| c.as_array()) {
            if let Some(choice) = choices.first() {
                if let Some(tool_calls) = choice
                    .get("message")
                    .and_then(|m| m.get("tool_calls"))
                    .and_then(|t| t.as_array())
                {
                    if let Some(tool_call) = tool_calls.first() {
                        if let Some(args_str) = tool_call
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|a| a.as_str())
                        {
                            if let Ok(args) = serde_json::from_str::<Value>(args_str) {
                                if let Some(group) = args.get("group").and_then(|g| g.as_str()) {
                                    println!(
                                        "[OK] {}: {}",
                                        ansi_term::Colour::Cyan.paint("Selected Family"),
                                        ansi_term::Colour::Yellow.bold().paint(group)
                                    );
                                }
                                if let Some(reasoning) =
                                    args.get("reasoning").and_then(|r| r.as_str())
                                {
                                    println!(
                                        "💭 {}: {}",
                                        ansi_term::Colour::Cyan.paint("Reasoning"),
                                        reasoning
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        println!("{}", "━".repeat(80));

        self.parse_group_selection(response_json).await
    }

    /// Two-tier planning: Second tier - select concrete action within group
    pub async fn plan_concrete_action(
        &self,
        group: &ActionGroup,
        messages: Vec<Value>,
    ) -> PlanResult<ConcreteStep> {
        debug!("Requesting concrete action for group: {:?}", group);

        append_llm_message_log("FULL MESSAGES TO LLM (Tier 2)", Some(&messages), None);

        println!(
            " {} {}",
            ansi_term::Colour::Cyan.bold().paint("Sending to LLM"),
            ansi_term::Colour::Yellow.dimmed().paint(format!(
                "[Tier 2] Size: {} chars",
                serde_json::to_string(&messages).unwrap_or_default().len()
            ))
        );

        // Pretty print the message content
        println!("\n{}", "━".repeat(80));
        println!(
            " {} {}",
            ansi_term::Colour::Cyan
                .bold()
                .paint("MESSAGE TO LLM - TIER 2"),
            ansi_term::Colour::Yellow
                .dimmed()
                .paint(format!("[{}]", chrono::Local::now().format("%H:%M:%S")))
        );
        println!("{}", "─".repeat(60));

        for message in &messages {
            if let Some(role) = message.get("role").and_then(|r| r.as_str()) {
                println!(
                    "  {}: {}",
                    ansi_term::Colour::Purple.bold().paint("Role"),
                    ansi_term::Colour::Green.paint(role)
                );
            }

            if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
                let lines: Vec<&str> = content.lines().take(10).collect(); // Limit lines
                for line in lines {
                    if line.starts_with("Goal:") {
                        println!(
                            "[TARGET] {}: {}",
                            ansi_term::Colour::Cyan.paint("Goal"),
                            ansi_term::Colour::Yellow
                                .bold()
                                .paint(line.strip_prefix("Goal:").unwrap_or("").trim())
                        );
                    } else if line.starts_with("Current State:") || line.starts_with("Current URL:")
                    {
                        println!(
                            " {}: {}",
                            ansi_term::Colour::Cyan.paint("State"),
                            ansi_term::Colour::Blue
                                .paint(line.split(':').nth(1).unwrap_or("").trim())
                        );
                    } else if line.starts_with("PAGE_HTML") {
                        println!(
                            " {}",
                            ansi_term::Colour::Cyan.paint("DOM Content: [provided]")
                        );
                        break; // Don't print the actual DOM
                    }
                }
            }
        }

        println!("{}", "━".repeat(80));

        let schema = match group {
            ActionGroup::Navigate => NAVIGATE_ACTIONS_SCHEMA,
            ActionGroup::Element => ELEMENT_ACTIONS_SCHEMA,
            ActionGroup::Scroll => SCROLL_ACTIONS_SCHEMA,
            ActionGroup::Wait => WAIT_ACTIONS_SCHEMA,
            ActionGroup::Data => DATA_ACTIONS_SCHEMA,
            ActionGroup::Assert => ASSERT_ACTIONS_SCHEMA,
            ActionGroup::JsEval => JS_EVAL_ACTIONS_SCHEMA,
            ActionGroup::Util => UTIL_ACTIONS_SCHEMA,
        };

        let tools = vec![json!({
            "type": "function",
            "function": serde_json::from_str::<Value>(schema)
                .map_err(PlanError::SerializationError)?
        })];

        let function_name = match group {
            ActionGroup::Navigate => "navigate_action",
            ActionGroup::Element => "element_action",
            ActionGroup::Scroll => "scroll_action",
            ActionGroup::Wait => "wait_action",
            ActionGroup::Data => "data_action",
            ActionGroup::Assert => "assert_action",
            ActionGroup::JsEval => "js_eval_action",
            ActionGroup::Util => "util_action",
        };

        let response_json = self
            .provider
            .chat_completion(
                messages.clone(),
                self.temperature,
                Some(tools),
                Some(json!({"type": "function", "function": {"name": function_name}})),
                Some(800),
            )
            .await?;

        append_llm_message_log(
            "[BOT] FULL LLM RESPONSE (Tier 2)",
            None,
            Some(&response_json),
        );

        println!(
            "[BOT] {} {}",
            ansi_term::Colour::Green.bold().paint("Received from LLM"),
            ansi_term::Colour::Yellow.dimmed().paint(format!(
                "[Tier 2] Response size: {} chars",
                response_json.to_string().len()
            ))
        );

        // Pretty print the response
        println!("\n{}", "━".repeat(80));
        println!(
            "[BOT] {} {}",
            ansi_term::Colour::Green
                .bold()
                .paint("LLM RESPONSE - TIER 2"),
            ansi_term::Colour::Yellow
                .dimmed()
                .paint(format!("[{}]", chrono::Local::now().format("%H:%M:%S")))
        );
        println!("{}", "─".repeat(60));

        // Parse and display the response
        if let Some(choices) = response_json.get("choices").and_then(|c| c.as_array()) {
            if let Some(choice) = choices.first() {
                if let Some(tool_calls) = choice
                    .get("message")
                    .and_then(|m| m.get("tool_calls"))
                    .and_then(|t| t.as_array())
                {
                    if let Some(tool_call) = tool_calls.first() {
                        if let Some(args_str) = tool_call
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|a| a.as_str())
                        {
                            if let Ok(args) = serde_json::from_str::<Value>(args_str) {
                                // Display action details
                                if let Some(action) = args.get("action").and_then(|a| a.as_str()) {
                                    println!(
                                        "[TARGET] {}: {}",
                                        ansi_term::Colour::Cyan.paint("Action"),
                                        ansi_term::Colour::Yellow.bold().paint(action)
                                    );
                                }
                                if let Some(name) = args.get("name").and_then(|n| n.as_str()) {
                                    println!(
                                        "[NOTE] {}: {}",
                                        ansi_term::Colour::Cyan.paint("Name"),
                                        name
                                    );
                                }
                                if let Some(url) = args.get("url").and_then(|u| u.as_str()) {
                                    println!(
                                        " {}: {}",
                                        ansi_term::Colour::Cyan.paint("URL"),
                                        ansi_term::Colour::Blue.underline().paint(url)
                                    );
                                }
                                if let Some(selector) =
                                    args.get("selector").and_then(|s| s.as_str())
                                {
                                    println!(
                                        "[TARGET] {}: {}",
                                        ansi_term::Colour::Cyan.paint("Selector"),
                                        ansi_term::Colour::Red.bold().paint(selector)
                                    );
                                }
                                if let Some(reasoning) =
                                    args.get("reasoning").and_then(|r| r.as_str())
                                {
                                    println!(
                                        "💭 {}: {}",
                                        ansi_term::Colour::Cyan.paint("Reasoning"),
                                        reasoning
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        println!("{}", "━".repeat(80));
        println!("{}", "─".repeat(60));

        self.parse_concrete_action(group, response_json).await
    }

    /// Two-tier planning: Orchestrate both tiers with intelligent fallbacks
    pub async fn plan_step(&self, messages: Vec<Value>) -> PlanResult<LLMResponse> {
        debug!("Starting two-tier planning");

        // DEBUG: Log summary info only (no content printing)
        let mut total_chars = 0;
        let mut dom_size = 0;
        let mut has_dom = false;

        for msg in messages.iter() {
            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                total_chars += content.len();
                if content.contains("PAGE_HTML") {
                    has_dom = true;
                    // Find DOM content size
                    if let Some(dom_start) = content.find("PAGE_HTML") {
                        let dom_section = &content[dom_start..];
                        dom_size = dom_section.len();
                    }
                }
            }
        }

        let estimated_tokens = total_chars / 4;
        println!(
            "\n {} {}",
            ansi_term::Colour::Cyan.bold().paint("Sending to LLM:"),
            ansi_term::Colour::Yellow.dimmed().paint(format!(
                "Total: {} chars (~{} tokens), DOM: {} chars",
                total_chars, estimated_tokens, dom_size
            ))
        );

        // Pretty print the message content for debugging
        println!("\n{}", "━".repeat(80));
        println!(
            " {} {}",
            ansi_term::Colour::Cyan
                .bold()
                .paint("MESSAGE TO LLM - PLANNING"),
            ansi_term::Colour::Yellow
                .dimmed()
                .paint(format!("[{}]", chrono::Local::now().format("%H:%M:%S")))
        );
        println!("{}", "─".repeat(60));

        for message in &messages {
            if let Some(role) = message.get("role").and_then(|r| r.as_str()) {
                println!(
                    "  {}: {}",
                    ansi_term::Colour::Purple.bold().paint("Role"),
                    ansi_term::Colour::Green.paint(role)
                );
            }

            if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
                let lines: Vec<&str> = content.lines().collect();
                let mut printed_lines = 0;
                for line in lines {
                    if printed_lines > 10 && line.starts_with("PAGE_HTML") {
                        println!(
                            " {} [DOM content truncated for display]",
                            ansi_term::Colour::Cyan.paint("DOM:")
                        );
                        break;
                    }

                    if line.starts_with("Goal:") {
                        println!(
                            "[TARGET] {}: {}",
                            ansi_term::Colour::Cyan.paint("Goal"),
                            ansi_term::Colour::Yellow
                                .bold()
                                .paint(line.strip_prefix("Goal:").unwrap_or("").trim())
                        );
                    } else if line.starts_with("Current State:") || line.starts_with("Current URL:")
                    {
                        println!(
                            " {}: {}",
                            ansi_term::Colour::Cyan.paint("State"),
                            ansi_term::Colour::Blue
                                .paint(line.split(':').nth(1).unwrap_or("").trim())
                        );
                    } else if line.starts_with("PAGE_HTML") {
                        let dom_info = line.split(',').next().unwrap_or(line);
                        println!(" {}", ansi_term::Colour::Cyan.paint(dom_info));
                    } else if line.starts_with("Execution history:") {
                        println!("📜 {}", ansi_term::Colour::Cyan.paint("History:"));
                    } else if line.starts_with("What should be") {
                        println!("❓ {}", ansi_term::Colour::Cyan.bold().paint(line));
                    } else if line.trim().starts_with(char::is_numeric) && line.contains(".") {
                        // History items
                        println!("  {}", ansi_term::Colour::Fixed(8).paint(line));
                    }
                    printed_lines += 1;
                }
            }
        }

        println!("{}", "━".repeat(80));

        //  ROBUSTNESS: Try tier planning with fallback to single-tier on failure
        match self.plan_group(messages.clone()).await {
            Ok(group_selection) => {
                debug!("Selected group: {:?}", group_selection.group);

                // Pretty print group selection
                println!("\n{}", "━".repeat(80));
                println!(
                    " {} {}",
                    ansi_term::Colour::Green
                        .bold()
                        .paint("LLM RESPONSE - TIER 1 (Group Selection)"),
                    ansi_term::Colour::Yellow
                        .dimmed()
                        .paint(format!("[{}]", chrono::Local::now().format("%H:%M:%S")))
                );
                println!("{}", "─".repeat(60));
                println!(
                    "[OK] {}: {}",
                    ansi_term::Colour::Cyan.paint("Selected Family"),
                    ansi_term::Colour::Yellow
                        .bold()
                        .paint(format!("{:?}", group_selection.group))
                );
                println!(
                    "💭 {}: {}",
                    ansi_term::Colour::Cyan.paint("Reasoning"),
                    group_selection.reasoning
                );
                println!("{}", "━".repeat(80));

                // Tier 2: Select concrete action within group
                match self
                    .plan_concrete_action(&group_selection.group, messages.clone())
                    .await
                {
                    Ok(concrete_step) => {
                        debug!("Selected concrete action: {}", concrete_step.step.name);

                        // Pretty print concrete action
                        println!("\n{}", "━".repeat(80));
                        println!(
                            " {} {}",
                            ansi_term::Colour::Green
                                .bold()
                                .paint("LLM RESPONSE - TIER 2 (Concrete Action)"),
                            ansi_term::Colour::Yellow
                                .dimmed()
                                .paint(format!("[{}]", chrono::Local::now().format("%H:%M:%S")))
                        );
                        println!("{}", "─".repeat(60));
                        println!(
                            "[TARGET] {}: {}",
                            ansi_term::Colour::Cyan.paint("Action"),
                            ansi_term::Colour::Yellow
                                .bold()
                                .paint(&concrete_step.step.name)
                        );

                        // Pretty print step details based on type
                        match &concrete_step.step.kind {
                            StepKind::FillInputField {
                                selector, value, ..
                            } => {
                                println!(
                                    "[NOTE] {}: {}",
                                    ansi_term::Colour::Cyan.paint("Type"),
                                    ansi_term::Colour::Purple.paint("FillInputField")
                                );
                                println!(
                                    "[TARGET] {}: {}",
                                    ansi_term::Colour::Cyan.paint("Selector"),
                                    ansi_term::Colour::Red.bold().paint(selector)
                                );
                                println!(
                                    "  {}: {}",
                                    ansi_term::Colour::Cyan.paint("Value"),
                                    ansi_term::Colour::Green.paint(value)
                                );
                            }
                            StepKind::ClickElement { selector, .. } => {
                                println!(
                                    "🖱️  {}: {}",
                                    ansi_term::Colour::Cyan.paint("Type"),
                                    ansi_term::Colour::Purple.paint("ClickElement")
                                );
                                println!(
                                    "[TARGET] {}: {}",
                                    ansi_term::Colour::Cyan.paint("Selector"),
                                    ansi_term::Colour::Red.bold().paint(selector)
                                );
                            }
                            StepKind::NavigateToUrl { url, .. } => {
                                println!(
                                    " {}: {}",
                                    ansi_term::Colour::Cyan.paint("Type"),
                                    ansi_term::Colour::Purple.paint("NavigateToUrl")
                                );
                                println!(
                                    " {}: {}",
                                    ansi_term::Colour::Cyan.paint("URL"),
                                    ansi_term::Colour::Blue.underline().paint(url)
                                );
                            }
                            _ => {
                                println!(
                                    "[LIST] {}: {:?}",
                                    ansi_term::Colour::Cyan.paint("Step Details"),
                                    concrete_step.step.kind
                                );
                            }
                        }

                        println!(
                            "💭 {}: {}",
                            ansi_term::Colour::Cyan.paint("Reasoning"),
                            concrete_step.reasoning
                        );
                        println!("{}", "━".repeat(80));

                        // Use the step exactly as the LLM generated it
                        Ok(LLMResponse {
                            is_complete: false,
                            next_step: Some(concrete_step.step),
                            extracted_data: None,
                            reasoning: Some(format!(
                                "Group: {} | Action: {}",
                                group_selection.reasoning, concrete_step.reasoning
                            )),
                        })
                    }
                    Err(e) => {
                        debug!(
                            "Two-tier concrete action failed, falling back to single-tier: {}",
                            e
                        );

                        println!("\n{}", "━".repeat(80));
                        println!(
                            "[WARNING]  {} {}",
                            ansi_term::Colour::Yellow
                                .bold()
                                .paint("TIER 2 FAILED - Falling back to single-tier"),
                            ansi_term::Colour::Red.paint(format!("Error: {}", e))
                        );
                        println!("{}", "━".repeat(80));

                        self.plan_next_step(messages).await
                    }
                }
            }
            Err(e) => {
                debug!(
                    "Two-tier group selection failed, falling back to single-tier: {}",
                    e
                );

                println!("\n{}", "━".repeat(80));
                println!(
                    "[WARNING]  {} {}",
                    ansi_term::Colour::Yellow
                        .bold()
                        .paint("TIER 1 FAILED - Falling back to single-tier"),
                    ansi_term::Colour::Red.paint(format!("Error: {}", e))
                );
                println!("{}", "━".repeat(80));

                self.plan_next_step(messages).await
            }
        }
    }

    /// Parse group selection response
    async fn parse_group_selection(&self, response: Value) -> PlanResult<GroupSelection> {
        let choices = response["choices"]
            .as_array()
            .ok_or_else(|| PlanError::LLMError("No choices in response".to_string()))?;

        if choices.is_empty() {
            return Err(PlanError::LLMError("Empty choices array".to_string()));
        }

        let message = &choices[0]["message"];
        let tool_calls = message["tool_calls"]
            .as_array()
            .ok_or_else(|| PlanError::LLMError("No tool calls in group selection".to_string()))?;

        let tool_call = tool_calls
            .first()
            .ok_or_else(|| PlanError::LLMError("No tool call found".to_string()))?;

        let function = &tool_call["function"];
        let arguments_str = function["arguments"]
            .as_str()
            .ok_or_else(|| PlanError::LLMError("No function arguments".to_string()))?;

        let arguments: Value =
            serde_json::from_str(arguments_str).map_err(PlanError::SerializationError)?;

        let group_str = arguments["group"]
            .as_str()
            .ok_or_else(|| PlanError::LLMError("No group in arguments".to_string()))?;

        let group = match group_str {
            "NAVIGATE" => ActionGroup::Navigate,
            "ELEMENT" => ActionGroup::Element,
            "SCROLL" => ActionGroup::Scroll,
            "WAIT" => ActionGroup::Wait,
            "DATA" => ActionGroup::Data,
            "ASSERT" => ActionGroup::Assert,
            "JS_EVAL" => ActionGroup::JsEval,
            "UTIL" => ActionGroup::Util,
            _ => return Err(PlanError::LLMError(format!("Unknown group: {}", group_str))),
        };

        let reasoning = arguments["reasoning"]
            .as_str()
            .ok_or_else(|| PlanError::LLMError("No reasoning in arguments".to_string()))?
            .to_string();

        Ok(GroupSelection { group, reasoning })
    }

    /// Parse concrete action response
    async fn parse_concrete_action(
        &self,
        group: &ActionGroup,
        response: Value,
    ) -> PlanResult<ConcreteStep> {
        let choices = response["choices"]
            .as_array()
            .ok_or_else(|| PlanError::LLMError("No choices in response".to_string()))?;

        if choices.is_empty() {
            return Err(PlanError::LLMError("Empty choices array".to_string()));
        }

        let message = &choices[0]["message"];
        let tool_calls = message["tool_calls"]
            .as_array()
            .ok_or_else(|| PlanError::LLMError("No tool calls in concrete action".to_string()))?;

        let tool_call = tool_calls
            .first()
            .ok_or_else(|| PlanError::LLMError("No tool call found".to_string()))?;

        let function = &tool_call["function"];
        let arguments_str = function["arguments"]
            .as_str()
            .ok_or_else(|| PlanError::LLMError("No function arguments".to_string()))?;

        let arguments: Value =
            serde_json::from_str(arguments_str).map_err(PlanError::SerializationError)?;

        let step = self.parse_two_tier_step(group, arguments.clone())?;
        let reasoning = arguments["reasoning"]
            .as_str()
            .unwrap_or("No reasoning provided")
            .to_string();

        Ok(ConcreteStep { step, reasoning })
    }

    /// Parse step from two-tier arguments
    fn parse_two_tier_step(&self, group: &ActionGroup, args: Value) -> PlanResult<Step> {
        let id = args["id"]
            .as_str()
            .ok_or_else(|| PlanError::InvalidStep("Missing step id".to_string()))?
            .to_string();

        let name = args["name"]
            .as_str()
            .ok_or_else(|| PlanError::InvalidStep("Missing step name".to_string()))?
            .to_string();

        let action = args["action"]
            .as_str()
            .ok_or_else(|| PlanError::InvalidStep("Missing action".to_string()))?;

        let kind = match (group, action) {
            (ActionGroup::Navigate, "navigate_to_url") => {
                let url = args["url"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("Navigate step missing url".to_string())
                })?;

                // Validate URL is not empty
                if url.trim().is_empty() {
                    return Err(PlanError::InvalidStep(
                        "Navigate step has empty url".to_string(),
                    ));
                }

                // URL validation and fixing
                let validated_url = if url.starts_with("http://") || url.starts_with("https://") {
                    url.to_string()
                } else if url.contains(".") && !url.contains("/") {
                    // Looks like a domain name, add https://
                    format!("https://{}", url)
                } else {
                    return Err(PlanError::InvalidStep(format!(
                        "Invalid URL format: '{}'",
                        url
                    )));
                };

                StepKind::NavigateToUrl {
                    url: validated_url,
                    wait: args["wait"]
                        .as_str()
                        .map(|s| s.to_string())
                        .or(Some("domcontentloaded".to_string())),
                }
            }
            (ActionGroup::Navigate, "get_current_url") => StepKind::GetCurrentUrl,
            (ActionGroup::Navigate, "open_new_tab") => {
                let url = args["url"].as_str().map(|s| s.to_string());
                StepKind::OpenNewTab { url }
            }
            (ActionGroup::Navigate, "switch_to_tab") => {
                let tab_id = args["tab_id"].as_u64().ok_or_else(|| {
                    PlanError::InvalidStep("SwitchToTab step missing tab_id".to_string())
                })?;
                StepKind::SwitchToTab {
                    tab_identifier: serde_json::Value::Number(serde_json::Number::from(tab_id)),
                }
            }
            (ActionGroup::Navigate, "close_current_tab") => {
                let tab_id = args["tab_id"].as_u64().unwrap_or(0);
                StepKind::CloseCurrentTab {
                    tab_identifier: serde_json::Value::Number(serde_json::Number::from(tab_id)),
                }
            }
            (ActionGroup::Element, "click_element") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("Click step missing selector".to_string())
                })?;
                StepKind::ClickElement {
                    selector: selector.to_string(),
                    frame_id: None,
                    random_offset: None,
                    timeout_ms: Some(5000),
                }
            }
            (ActionGroup::Element, "fill_input_field") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("Fill step missing selector".to_string())
                })?;
                let value = args["value"]
                    .as_str()
                    .ok_or_else(|| PlanError::InvalidStep("Fill step missing value".to_string()))?;
                StepKind::FillInputField {
                    selector: selector.to_string(),
                    value: value.to_string(),
                    frame_id: None,
                    clear_first: Some(true),
                    simulate_typing: Some(true),
                    delay_ms: Some(50),
                    timeout_ms: Some(5000),
                }
            }
            (ActionGroup::Element, "press_special_key") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("PressSpecialKey step missing selector".to_string())
                })?;
                let key = args["key"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("PressSpecialKey step missing key".to_string())
                })?;
                StepKind::PressSpecialKey {
                    key: key.to_string(),
                    selector: Some(selector.to_string()),
                    frame_id: None,
                    timeout_ms: Some(5000),
                }
            }
            (ActionGroup::Element, "hover_element") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("Hover step missing selector".to_string())
                })?;
                StepKind::HoverElement {
                    selector: selector.to_string(),
                    frame_id: None,
                    random_offset: None,
                    timeout_ms: Some(5000),
                }
            }
            (ActionGroup::Element, "dbl_click_element") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("DoubleClick step missing selector".to_string())
                })?;
                StepKind::DblClickElement {
                    selector: selector.to_string(),
                    frame_id: None,
                    random_offset: None,
                    timeout_ms: Some(5000),
                }
            }
            (ActionGroup::Element, "select_option_in_dropdown") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("SelectOption step missing selector".to_string())
                })?;
                let value = args["value"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("SelectOption step missing value".to_string())
                })?;
                StepKind::SelectOptionInDropdown {
                    selector: selector.to_string(),
                    value: value.to_string(),
                    frame_id: None,
                    timeout_ms: Some(5000),
                }
            }
            (ActionGroup::Element, "upload_file") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("UploadFile step missing selector".to_string())
                })?;
                let file_path = args["file_path"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("UploadFile step missing file_path".to_string())
                })?;
                StepKind::UploadFile {
                    selector: selector.to_string(),
                    file_path: file_path.to_string(),
                    frame_id: None,
                    timeout_ms: Some(5000),
                }
            }
            (ActionGroup::Element, "drag_and_drop") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("DragAndDrop step missing selector".to_string())
                })?;
                let target_selector = args["target_selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("DragAndDrop step missing target_selector".to_string())
                })?;
                StepKind::DragAndDrop {
                    source_selector: selector.to_string(),
                    target_selector: target_selector.to_string(),
                    frame_id: None,
                    timeout_ms: Some(5000),
                }
            }
            (ActionGroup::Wait, "wait") => {
                let milliseconds = args["milliseconds"].as_u64().ok_or_else(|| {
                    PlanError::InvalidStep("Wait step missing milliseconds".to_string())
                })?;
                StepKind::WaitForTimeout {
                    timeout_ms: milliseconds as u32,
                }
            }
            (ActionGroup::Wait, "wait_for_timeout") => {
                let timeout = args["timeout"].as_u64().ok_or_else(|| {
                    PlanError::InvalidStep("WaitForTimeout step missing timeout".to_string())
                })?;
                StepKind::WaitForTimeout {
                    timeout_ms: timeout as u32,
                }
            }
            (ActionGroup::Wait, "wait_for_selector") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("WaitForSelector step missing selector".to_string())
                })?;
                let condition = args["condition"].as_str().map(|s| s.to_string());
                StepKind::WaitForElement {
                    selector: selector.to_string(),
                    frame_id: None,
                    condition,
                    timeout_ms: args["timeout_ms"].as_u64().map(|t| t as u32),
                }
            }
            (ActionGroup::Wait, "wait_for_element") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("WaitForElement step missing selector".to_string())
                })?;
                let condition = args["condition"].as_str().map(|s| s.to_string());
                StepKind::WaitForElement {
                    selector: selector.to_string(),
                    frame_id: None,
                    condition,
                    timeout_ms: args["timeout_ms"].as_u64().map(|t| t as u32),
                }
            }
            (ActionGroup::Wait, "wait_for_navigation") => StepKind::WaitForNavigation {
                url_pattern: args["url_pattern"].as_str().map(|s| s.to_string()),
                timeout_ms: args["timeout_ms"].as_u64().map(|t| t as u32),
            },
            (ActionGroup::Wait, "wait_for_network_idle") => StepKind::WaitForNetworkIdle {
                idle_time_ms: args["idle_time_ms"].as_u64().unwrap_or(500) as u32,
                max_wait_ms: args["max_wait_ms"].as_u64().unwrap_or(30000) as u32,
            },
            (ActionGroup::Scroll, "scroll_window_to") => StepKind::ScrollWindowTo {
                x: args["x"].as_i64().map(|x| x as i32),
                y: args["y"].as_i64().map(|y| y as i32),
                direction: args["direction"].as_str().map(|s| s.to_string()),
            },
            (ActionGroup::Scroll, "scroll_element_into_view") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep(
                        "ScrollElementIntoView step missing selector".to_string(),
                    )
                })?;
                StepKind::ScrollElementIntoView {
                    selector: selector.to_string(),
                    frame_id: None,
                }
            }
            (ActionGroup::Scroll, "infinite_scroll") => {
                let item_selector = args["item_selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("InfiniteScroll step missing item_selector".to_string())
                })?;
                let target_count = args["target_count"].as_u64().ok_or_else(|| {
                    PlanError::InvalidStep("InfiniteScroll step missing target_count".to_string())
                })?;
                StepKind::InfiniteScroll {
                    item_selector: item_selector.to_string(),
                    target_count: target_count as u32,
                    frame_id: None,
                    max_cycles: args["max_cycles"].as_u64().unwrap_or(30) as u32,
                }
            }
            (ActionGroup::Data, "extract_structured_data") => {
                let item_selector = args["item_selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep(
                        "ExtractStructuredData step missing item_selector".to_string(),
                    )
                })?;
                let fields = args["fields"].as_array().ok_or_else(|| {
                    PlanError::InvalidStep("ExtractStructuredData step missing fields".to_string())
                })?;

                let field_specs =
                    fields
                        .iter()
                        .map(|field| {
                            let field_obj = field.as_object().ok_or_else(|| {
                                PlanError::InvalidStep("Invalid field object".to_string())
                            })?;

                            //  ROBUSTNESS: Safe field access with detailed error context
                            let name = field_obj.get("name").and_then(|v| v.as_str()).ok_or_else(
                                || {
                                    PlanError::InvalidStep(format!(
                                        "Field missing 'name' - available keys: {:?}",
                                        field_obj.keys().collect::<Vec<_>>()
                                    ))
                                },
                            )?;
                            let selector = field_obj
                                .get("selector")
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| {
                                    PlanError::InvalidStep(format!(
                                        "Field missing 'selector' - available keys: {:?}",
                                        field_obj.keys().collect::<Vec<_>>()
                                    ))
                                })?;

                            Ok(rzn_core::FieldSpec {
                                name: name.to_string(),
                                selector: selector.to_string(),
                                attribute: field_obj
                                    .get("attribute")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string()),
                                post_processing: field_obj
                                    .get("post_processing")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                            .collect()
                                    })
                                    .unwrap_or_default(),
                            })
                        })
                        .collect::<Result<Vec<_>, PlanError>>()?;

                StepKind::ExtractStructuredData {
                    item_selector: item_selector.to_string(),
                    limit: None,
                    fields: field_specs,
                    frame_id: None,
                    extraction_type: None,
                }
            }
            (ActionGroup::Data, "get_element_text") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("GetElementText step missing selector".to_string())
                })?;
                StepKind::GetElementText {
                    selector: selector.to_string(),
                    frame_id: None,
                }
            }
            (ActionGroup::Data, "get_element_attribute") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("GetElementAttribute step missing selector".to_string())
                })?;
                let attribute = args["attribute"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("GetElementAttribute step missing attribute".to_string())
                })?;
                StepKind::GetElementAttribute {
                    selector: selector.to_string(),
                    attribute: attribute.to_string(),
                    frame_id: None,
                }
            }
            (ActionGroup::Data, "get_element_value") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("GetElementValue step missing selector".to_string())
                })?;
                StepKind::GetElementValue {
                    selector: selector.to_string(),
                    frame_id: None,
                }
            }
            (ActionGroup::Data, "get_element_count") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("GetElementCount step missing selector".to_string())
                })?;
                StepKind::GetElementCount {
                    selector: selector.to_string(),
                    frame_id: None,
                }
            }
            (ActionGroup::Data, "take_screenshot") => StepKind::TakeScreenshot {
                full_page: args["full_page"].as_bool(),
                annotate: args["annotate"].as_bool(),
                annotate_max_labels: args["annotate_max_labels"].as_u64().map(|n| n as u32),
                annotate_max_elements: args["annotate_max_elements"].as_u64().map(|n| n as u32),
                quality: args["quality"].as_u64().map(|q| q as u8),
                format: args["format"].as_str().map(|s| s.to_string()),
            },
            (ActionGroup::Data, "get_page_source") => StepKind::GetPageSource,
            (ActionGroup::Assert, "assert_selector_state") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("AssertSelectorState step missing selector".to_string())
                })?;
                let condition = args["condition"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("AssertSelectorState step missing condition".to_string())
                })?;
                StepKind::AssertSelectorState {
                    selector: selector.to_string(),
                    condition: condition.to_string(),
                    frame_id: None,
                }
            }
            (ActionGroup::Assert, "assert_text_in_element") => {
                let selector = args["selector"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("AssertTextInElement step missing selector".to_string())
                })?;
                let text = args["text"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("AssertTextInElement step missing text".to_string())
                })?;
                StepKind::AssertTextInElement {
                    selector: selector.to_string(),
                    text: text.to_string(),
                    frame_id: None,
                    match_type: args["match_type"].as_str().map(|s| s.to_string()),
                }
            }
            (ActionGroup::Assert, "assert_url_matches") => {
                let url_pattern = args["url_pattern"].as_str().ok_or_else(|| {
                    PlanError::InvalidStep("AssertUrlMatches step missing url_pattern".to_string())
                })?;
                StepKind::AssertUrlMatches {
                    url_pattern: url_pattern.to_string(),
                    match_type: args["match_type"].as_str().map(|s| s.to_string()),
                }
            }
            (ActionGroup::Assert, "verify_ui_change") => StepKind::VerifyUiChange {
                selector: args["selector"].as_str().map(|s| s.to_string()),
                condition: args["condition"].as_str().map(|s| s.to_string()),
                text: args["text"].as_str().map(|s| s.to_string()),
                match_type: args["match_type"].as_str().map(|s| s.to_string()),
                value_equals: args["value_equals"].as_str().map(|s| s.to_string()),
                value_contains: args["value_contains"].as_str().map(|s| s.to_string()),
                url_includes: args["url_includes"].as_str().map(|s| s.to_string()),
                url_matches: args["url_matches"].as_str().map(|s| s.to_string()),
                active_selector: args["active_selector"].as_str().map(|s| s.to_string()),
                count_at_least: args["count_at_least"].as_u64().map(|v| v as u32),
                count_equals: args["count_equals"].as_u64().map(|v| v as u32),
                all: args["all"].as_array().cloned(),
                any: args["any"].as_array().cloned(),
                timeout_ms: args["timeout_ms"].as_u64().map(|v| v as u32),
            },
            (ActionGroup::JsEval, "execute_javascript") => StepKind::ExecuteJavascript {
                script: args["script"]
                    .as_str()
                    .ok_or_else(|| {
                        PlanError::InvalidStep("ExecuteJavascript step missing script".to_string())
                    })?
                    .to_string(),
                args: args["args"].as_array().cloned(),
                return_value: args["return_value"].as_bool().unwrap_or(true),
                world: args["world"].as_str().map(|s| s.to_string()),
                timeout_ms: args["timeout_ms"].as_u64().map(|v| v as u32),
            },
            (ActionGroup::JsEval, "eval_main_world") => StepKind::EvalMainWorld {
                script: args["script"]
                    .as_str()
                    .ok_or_else(|| {
                        PlanError::InvalidStep("EvalMainWorld step missing script".to_string())
                    })?
                    .to_string(),
                args: args["args"].as_array().cloned(),
                return_value: args["return_value"].as_bool().unwrap_or(true),
                timeout_ms: args["timeout_ms"].as_u64().map(|v| v as u32),
            },
            (ActionGroup::JsEval, "eval_isolated_world") => StepKind::EvalIsolatedWorld {
                script: args["script"]
                    .as_str()
                    .ok_or_else(|| {
                        PlanError::InvalidStep("EvalIsolatedWorld step missing script".to_string())
                    })?
                    .to_string(),
                args: args["args"].as_array().cloned(),
                return_value: args["return_value"].as_bool().unwrap_or(true),
                timeout_ms: args["timeout_ms"].as_u64().map(|v| v as u32),
            },
            (ActionGroup::Util, "inspect_element") => StepKind::InspectElement {
                selector: args["selector"]
                    .as_str()
                    .ok_or_else(|| {
                        PlanError::InvalidStep("InspectElement step missing selector".to_string())
                    })?
                    .to_string(),
                frame_id: args["frame_id"].as_str().map(|s| s.to_string()),
                include_ancestors: args["include_ancestors"].as_bool(),
                include_shadow_path: args["include_shadow_path"].as_bool(),
                timeout_ms: args["timeout_ms"].as_u64().map(|v| v as u32),
            },
            (ActionGroup::Util, "inspect_click_surface") => StepKind::InspectClickSurface {
                selector: args["selector"]
                    .as_str()
                    .ok_or_else(|| {
                        PlanError::InvalidStep(
                            "InspectClickSurface step missing selector".to_string(),
                        )
                    })?
                    .to_string(),
                frame_id: args["frame_id"].as_str().map(|s| s.to_string()),
                timeout_ms: args["timeout_ms"].as_u64().map(|v| v as u32),
            },
            (ActionGroup::Util, "capture_ui_bundle") => StepKind::CaptureUiBundle {
                selector: args["selector"].as_str().map(|s| s.to_string()),
                include_dom_snapshot: args["include_dom_snapshot"].as_bool(),
                include_screenshot: args["include_screenshot"].as_bool(),
                annotate: args["annotate"].as_bool(),
                max_elements: args["max_elements"].as_u64().map(|v| v as u32),
                timeout_ms: args["timeout_ms"].as_u64().map(|v| v as u32),
            },
            (ActionGroup::Util, "read_field_value") => StepKind::ReadFieldValue {
                selector: args["selector"]
                    .as_str()
                    .ok_or_else(|| {
                        PlanError::InvalidStep("ReadFieldValue step missing selector".to_string())
                    })?
                    .to_string(),
                frame_id: args["frame_id"].as_str().map(|s| s.to_string()),
                timeout_ms: args["timeout_ms"].as_u64().map(|v| v as u32),
            },
            (ActionGroup::Util, "semantic_action") => StepKind::SemanticAction {
                action: args["semantic_action"]
                    .as_str()
                    .or_else(|| args["method"].as_str())
                    .ok_or_else(|| {
                        PlanError::InvalidStep(
                            "SemanticAction step missing semantic_action".to_string(),
                        )
                    })?
                    .to_string(),
                selector: args["selector"].as_str().map(|s| s.to_string()),
                value: args["value"].as_str().map(|s| s.to_string()),
                key: args["key"].as_str().map(|s| s.to_string()),
                step: args.get("step").cloned(),
                postcondition: args.get("postcondition").cloned(),
                postcondition_required: args["postcondition_required"].as_bool(),
                timeout_ms: args["timeout_ms"].as_u64().map(|v| v as u32),
            },
            _ => {
                return Err(PlanError::InvalidStep(format!(
                    "Unsupported action {} for group {:?}",
                    action, group
                )))
            }
        };

        Ok(Step { id, name, kind })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_provider::LLMProvider;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct FlakyJsonProvider {
        calls: Mutex<u32>,
    }

    struct StaticResponsesProvider {
        response: Value,
    }

    #[async_trait]
    impl LLMProvider for FlakyJsonProvider {
        fn provider_name(&self) -> &str {
            "FlakyJsonProvider"
        }

        fn model_name(&self) -> &str {
            "test"
        }

        async fn chat_completion(
            &self,
            _messages: Vec<Value>,
            _temperature: f32,
            _tools: Option<Vec<Value>>,
            _tool_choice: Option<Value>,
            _max_tokens: Option<u32>,
        ) -> PlanResult<Value> {
            let mut guard = self.calls.lock().unwrap();
            *guard += 1;
            let n = *guard;

            let content = if n == 1 {
                // Intentionally non-JSON to force the bounded retry path.
                "I have completed the task.".to_string()
            } else {
                r#"{"ok": true, "step": {"cmd": "noop", "args": []}}"#.to_string()
            };

            Ok(json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": content
                    }
                }]
            }))
        }

        async fn simple_chat(
            &self,
            _messages: Vec<Value>,
            _temperature: Option<f32>,
        ) -> PlanResult<String> {
            Ok("{}".to_string())
        }
    }

    #[async_trait]
    impl LLMProvider for StaticResponsesProvider {
        fn provider_name(&self) -> &str {
            "OpenAI"
        }

        fn model_name(&self) -> &str {
            "test"
        }

        async fn chat_completion(
            &self,
            _messages: Vec<Value>,
            _temperature: f32,
            _tools: Option<Vec<Value>>,
            _tool_choice: Option<Value>,
            _max_tokens: Option<u32>,
        ) -> PlanResult<Value> {
            Err(PlanError::LLMError(
                "chat_completion should not be called in Responses tests".to_string(),
            ))
        }

        async fn simple_chat(
            &self,
            _messages: Vec<Value>,
            _temperature: Option<f32>,
        ) -> PlanResult<String> {
            Ok("{}".to_string())
        }

        async fn responses_completion(
            &self,
            _input: Vec<Value>,
            _temperature: f32,
            _tools: Option<Vec<Value>>,
            _tool_choice: Option<Value>,
            _response_format: Option<Value>,
            _max_tokens: Option<u32>,
        ) -> PlanResult<Value> {
            Ok(self.response.clone())
        }
    }

    fn chat_json_messages() -> Vec<Value> {
        vec![json!({
            "role": "user",
            "content": "Return only JSON."
        })]
    }

    #[test]
    fn fix_new_simple_bad_api_key_returns_err_without_panic() {
        let result = std::panic::catch_unwind(|| LLMClient::new_simple("not-an-openai-key".into()));

        assert!(result.is_ok(), "new_simple panicked on invalid key");
        match result.unwrap() {
            Err(PlanError::LLMError(message)) => {
                assert!(message.contains("Invalid OpenAI API key format"));
            }
            Err(err) => panic!("expected LLMError, got {err}"),
            Ok(_) => panic!("expected invalid key to fail"),
        }
    }

    #[tokio::test]
    async fn chat_json_retries_on_non_json_content() {
        let llm = LLMClient::with_provider(Box::new(FlakyJsonProvider {
            calls: Mutex::new(0),
        }));

        let v = llm
            .chat_json(chat_json_messages(), None)
            .await
            .expect("chat_json");
        assert_eq!(v.get("ok").and_then(|v| v.as_bool()), Some(true));
    }

    #[tokio::test]
    async fn chat_json_openai_responses_rejects_unparseable_text_without_envelope_leak() {
        let leak_sentinel = "resp_env_leak_sentinel";
        let llm = LLMClient::with_provider(Box::new(StaticResponsesProvider {
            response: json!({
                "id": leak_sentinel,
                "object": "response",
                "output_text": "not json",
                "output": [{
                    "type": "message",
                    "content": [{
                        "type": "output_text",
                        "text": "still not json"
                    }]
                }]
            }),
        }));

        match llm.chat_json(chat_json_messages(), None).await {
            Err(PlanError::LLMError(message)) => {
                assert_eq!(message, "Failed to parse JSON from Responses API");
            }
            Err(err) => panic!("expected LLMError, got {err}"),
            Ok(value) => {
                assert_ne!(
                    value.get("id").and_then(|v| v.as_str()),
                    Some(leak_sentinel),
                    "raw API envelope leaked"
                );
                panic!("expected unparseable model output to fail, got {value}");
            }
        }
    }

    #[tokio::test]
    async fn chat_json_openai_responses_parses_plain_output_text() {
        let llm = LLMClient::with_provider(Box::new(StaticResponsesProvider {
            response: json!({
                "output_text": r#"{"ok": true, "source": "plain"}"#
            }),
        }));

        let value = llm
            .chat_json(chat_json_messages(), None)
            .await
            .expect("chat_json");
        assert_eq!(value.get("ok").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(value.get("source").and_then(|v| v.as_str()), Some("plain"));
    }

    #[tokio::test]
    async fn chat_json_openai_responses_parses_fenced_output_text() {
        let llm = LLMClient::with_provider(Box::new(StaticResponsesProvider {
            response: json!({
                "output_text": "```json\n{\"ok\": true, \"source\": \"fenced\"}\n```"
            }),
        }));

        let value = llm
            .chat_json(chat_json_messages(), None)
            .await
            .expect("chat_json");
        assert_eq!(value.get("ok").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(value.get("source").and_then(|v| v.as_str()), Some("fenced"));
    }
}
