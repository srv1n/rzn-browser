use crate::{PlanError, PlanResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Action metadata for validation and execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionMetadata {
    /// Action identifier (e.g., "click", "type", "navigate")
    pub action_type: String,
    /// Human-readable description
    pub description: String,
    /// Required parameters
    pub required_params: Vec<String>,
    /// Optional parameters
    pub optional_params: Vec<String>,
    /// Whether this action modifies the page state
    pub modifies_page: bool,
    /// Whether this action requires element selection
    pub requires_element: bool,
    /// Validation function name
    pub validator: Option<String>,
}

/// Registry for available browser actions
pub struct ActionRegistry {
    actions: HashMap<String, ActionMetadata>,
}

impl ActionRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            actions: HashMap::new(),
        };
        registry.register_default_actions();
        registry
    }

    /// Register default browser actions
    fn register_default_actions(&mut self) {
        // Navigation actions
        self.register(ActionMetadata {
            action_type: "navigate".to_string(),
            description: "Navigate to a URL".to_string(),
            required_params: vec!["url".to_string()],
            optional_params: vec!["wait_for".to_string()],
            modifies_page: true,
            requires_element: false,
            validator: Some("validate_url".to_string()),
        });

        // Click actions
        self.register(ActionMetadata {
            action_type: "click".to_string(),
            description: "Click on an element".to_string(),
            required_params: vec!["index".to_string()],
            optional_params: vec!["wait_after".to_string()],
            modifies_page: true,
            requires_element: true,
            validator: Some("validate_element_index".to_string()),
        });

        // Type actions
        self.register(ActionMetadata {
            action_type: "type".to_string(),
            description: "Type text into an input field".to_string(),
            required_params: vec!["index".to_string(), "text".to_string()],
            optional_params: vec!["clear_first".to_string(), "press_enter".to_string()],
            modifies_page: true,
            requires_element: true,
            validator: Some("validate_type_action".to_string()),
        });

        // Wait actions
        self.register(ActionMetadata {
            action_type: "wait".to_string(),
            description: "Wait for a specified duration".to_string(),
            required_params: vec!["seconds".to_string()],
            optional_params: vec![],
            modifies_page: false,
            requires_element: false,
            validator: Some("validate_wait_duration".to_string()),
        });

        // Scroll actions
        self.register(ActionMetadata {
            action_type: "scroll".to_string(),
            description: "Scroll the page or element".to_string(),
            required_params: vec!["direction".to_string()],
            optional_params: vec!["amount".to_string(), "index".to_string()],
            modifies_page: false,
            requires_element: false,
            validator: Some("validate_scroll_params".to_string()),
        });

        // Extract actions
        self.register(ActionMetadata {
            action_type: "extract".to_string(),
            description: "Extract text from elements".to_string(),
            required_params: vec![],
            optional_params: vec!["indices".to_string(), "attribute".to_string()],
            modifies_page: false,
            requires_element: false,
            validator: None,
        });

        // Screenshot actions
        self.register(ActionMetadata {
            action_type: "screenshot".to_string(),
            description: "Take a screenshot of the page".to_string(),
            required_params: vec![],
            optional_params: vec!["full_page".to_string()],
            modifies_page: false,
            requires_element: false,
            validator: None,
        });

        // Special key actions
        self.register(ActionMetadata {
            action_type: "key".to_string(),
            description: "Press a special key".to_string(),
            required_params: vec!["key".to_string()],
            optional_params: vec!["modifiers".to_string()],
            modifies_page: true,
            requires_element: false,
            validator: Some("validate_key_action".to_string()),
        });
    }

    /// Register a new action
    pub fn register(&mut self, metadata: ActionMetadata) {
        self.actions.insert(metadata.action_type.clone(), metadata);
    }

    /// Get action metadata
    pub fn get(&self, action_type: &str) -> Option<&ActionMetadata> {
        self.actions.get(action_type)
    }

    /// Validate an action
    pub fn validate_action(&self, action_type: &str, params: &Value) -> PlanResult<()> {
        let metadata = self.get(action_type).ok_or_else(|| {
            PlanError::Validation(format!("Unknown action type: {}", action_type))
        })?;

        let params_obj = params.as_object().ok_or_else(|| {
            PlanError::Validation("Action parameters must be an object".to_string())
        })?;

        // Check required parameters
        for required in &metadata.required_params {
            if !params_obj.contains_key(required) {
                return Err(PlanError::Validation(format!(
                    "Missing required parameter '{}' for action '{}'",
                    required, action_type
                )));
            }
        }

        // Run specific validator if defined
        if let Some(validator_name) = &metadata.validator {
            self.run_validator(validator_name, params)?;
        }

        Ok(())
    }

    /// Run specific validator for an action
    fn run_validator(&self, validator_name: &str, params: &Value) -> PlanResult<()> {
        match validator_name {
            "validate_url" => {
                let url = params["url"]
                    .as_str()
                    .ok_or_else(|| PlanError::Validation("URL must be a string".to_string()))?;

                // Basic URL validation
                if !url.starts_with("http://")
                    && !url.starts_with("https://")
                    && !url.starts_with("file://")
                {
                    return Err(PlanError::Validation(format!(
                        "Invalid URL protocol: {}",
                        url
                    )));
                }
                Ok(())
            }
            "validate_element_index" => {
                let index = params["index"].as_u64().ok_or_else(|| {
                    PlanError::Validation("Element index must be a number".to_string())
                })?;

                if index > 9999 {
                    return Err(PlanError::Validation(format!(
                        "Element index {} seems too large",
                        index
                    )));
                }
                Ok(())
            }
            "validate_type_action" => {
                let text = params["text"]
                    .as_str()
                    .ok_or_else(|| PlanError::Validation("Text must be a string".to_string()))?;

                if text.is_empty() {
                    return Err(PlanError::Validation("Text cannot be empty".to_string()));
                }
                Ok(())
            }
            "validate_wait_duration" => {
                let seconds = params["seconds"].as_f64().ok_or_else(|| {
                    PlanError::Validation("Wait duration must be a number".to_string())
                })?;

                if seconds < 0.0 || seconds > 60.0 {
                    return Err(PlanError::Validation(format!(
                        "Wait duration must be between 0 and 60 seconds, got {}",
                        seconds
                    )));
                }
                Ok(())
            }
            "validate_scroll_params" => {
                let direction = params["direction"].as_str().ok_or_else(|| {
                    PlanError::Validation("Scroll direction must be a string".to_string())
                })?;

                let valid_directions = ["up", "down", "left", "right", "top", "bottom"];
                if !valid_directions.contains(&direction) {
                    return Err(PlanError::Validation(format!(
                        "Invalid scroll direction: {}. Must be one of: {:?}",
                        direction, valid_directions
                    )));
                }
                Ok(())
            }
            "validate_key_action" => {
                let key = params["key"]
                    .as_str()
                    .ok_or_else(|| PlanError::Validation("Key must be a string".to_string()))?;

                let valid_keys = [
                    "Enter",
                    "Tab",
                    "Escape",
                    "Backspace",
                    "Delete",
                    "ArrowUp",
                    "ArrowDown",
                    "ArrowLeft",
                    "ArrowRight",
                    "Home",
                    "End",
                    "PageUp",
                    "PageDown",
                    "Space",
                ];

                if !valid_keys.contains(&key) && key.len() != 1 {
                    return Err(PlanError::Validation(format!(
                        "Invalid key: {}. Must be a single character or one of: {:?}",
                        key, valid_keys
                    )));
                }
                Ok(())
            }
            _ => Ok(()), // Unknown validator, skip
        }
    }

    /// Get all available actions
    pub fn list_actions(&self) -> Vec<&ActionMetadata> {
        self.actions.values().collect()
    }

    /// Convert action from LLM format to Step format
    pub fn convert_to_step(&self, action_type: &str, params: &Value) -> PlanResult<rzn_core::Step> {
        // Validate first
        self.validate_action(action_type, params)?;

        // Convert based on action type
        match action_type {
            "navigate" => Ok(rzn_core::Step {
                id: format!("navigate_{}", uuid::Uuid::new_v4()),
                name: format!(
                    "Navigate to {}",
                    params["url"].as_str().unwrap_or("unknown")
                ),
                kind: rzn_core::StepKind::NavigateToUrl {
                    url: params["url"].as_str().unwrap().to_string(),
                    wait: params
                        .get("wait_for")
                        .and_then(|w| w.as_str())
                        .map(|s| s.to_string()),
                },
            }),
            "click" => {
                let index = params["index"].as_u64().unwrap() as usize;
                Ok(rzn_core::Step {
                    id: format!("click_{}", uuid::Uuid::new_v4()),
                    name: format!("Click element [{}]", index),
                    kind: rzn_core::StepKind::ClickElement {
                        selector: format!("[data-highlight-index=\"{}\"]", index),
                        frame_id: None,
                        random_offset: Some(true),
                        timeout_ms: params
                            .get("wait_after")
                            .and_then(|w| w.as_f64())
                            .map(|s| (s * 1000.0) as u32),
                    },
                })
            }
            "type" => {
                let index = params["index"].as_u64().unwrap() as usize;
                let text = params["text"].as_str().unwrap().to_string();
                let clear_first = params
                    .get("clear_first")
                    .and_then(|c| c.as_bool())
                    .unwrap_or(true);
                let press_enter = params
                    .get("press_enter")
                    .and_then(|p| p.as_bool())
                    .unwrap_or(false);

                let kind = if press_enter {
                    rzn_core::StepKind::SubmitInput {
                        selector: format!("[data-highlight-index=\"{}\"]", index),
                        text: text.clone(),
                        frame_id: None,
                    }
                } else {
                    rzn_core::StepKind::FillInputField {
                        selector: format!("[data-highlight-index=\"{}\"]", index),
                        value: text.clone(),
                        frame_id: None,
                        clear_first: Some(clear_first),
                        simulate_typing: Some(true),
                        delay_ms: Some(100),
                        timeout_ms: Some(5000),
                    }
                };

                Ok(rzn_core::Step {
                    id: format!("type_{}", uuid::Uuid::new_v4()),
                    name: format!("Type text into element [{}]", index),
                    kind,
                })
            }
            "wait" => {
                let seconds = params["seconds"].as_f64().unwrap();
                Ok(rzn_core::Step {
                    id: format!("wait_{}", uuid::Uuid::new_v4()),
                    name: format!("Wait for {} seconds", seconds),
                    kind: rzn_core::StepKind::WaitForTimeout {
                        timeout_ms: (seconds * 1000.0) as u32,
                    },
                })
            }
            "scroll" => {
                let direction = params["direction"].as_str().unwrap();
                let (x, y) = match direction {
                    "down" => (None, Some(300)),
                    "up" => (None, Some(-300)),
                    "right" => (Some(300), None),
                    "left" => (Some(-300), None),
                    "top" => (Some(0), Some(0)),
                    "bottom" => (None, Some(99999)),
                    _ => (None, None),
                };

                Ok(rzn_core::Step {
                    id: format!("scroll_{}", uuid::Uuid::new_v4()),
                    name: format!("Scroll {}", direction),
                    kind: rzn_core::StepKind::ScrollWindowTo {
                        x,
                        y,
                        direction: Some(direction.to_string()),
                    },
                })
            }
            "key" => {
                let key = params["key"].as_str().unwrap().to_string();
                Ok(rzn_core::Step {
                    id: format!("key_{}", uuid::Uuid::new_v4()),
                    name: format!("Press {} key", key),
                    kind: rzn_core::StepKind::PressSpecialKey {
                        key,
                        selector: None,
                        frame_id: None,
                        timeout_ms: None,
                    },
                })
            }
            "screenshot" => Ok(rzn_core::Step {
                id: format!("screenshot_{}", uuid::Uuid::new_v4()),
                name: "Take screenshot".to_string(),
                kind: rzn_core::StepKind::TakeScreenshot {
                    full_page: params.get("full_page").and_then(|f| f.as_bool()),
                    annotate: params.get("annotate").and_then(|v| v.as_bool()),
                    annotate_max_labels: params
                        .get("annotate_max_labels")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32),
                    annotate_max_elements: params
                        .get("annotate_max_elements")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32),
                    quality: None,
                    format: None,
                },
            }),
            "extract" => {
                // For now, just get page source
                Ok(rzn_core::Step {
                    id: format!("extract_{}", uuid::Uuid::new_v4()),
                    name: "Extract page content".to_string(),
                    kind: rzn_core::StepKind::GetPageSource,
                })
            }
            _ => Err(PlanError::Validation(format!(
                "Unknown action type: {}",
                action_type
            ))),
        }
    }
}

impl Default for ActionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_action_registry() {
        let registry = ActionRegistry::new();

        // Test getting action metadata
        let click_action = registry.get("click").unwrap();
        assert_eq!(click_action.action_type, "click");
        assert!(click_action.requires_element);

        // Test validation - valid action
        let valid_params = json!({
            "index": 0
        });
        assert!(registry.validate_action("click", &valid_params).is_ok());

        // Test validation - missing required param
        let invalid_params = json!({});
        assert!(registry.validate_action("click", &invalid_params).is_err());

        // Test validation - invalid action type
        assert!(registry
            .validate_action("invalid_action", &valid_params)
            .is_err());
    }

    #[test]
    fn test_convert_to_step() {
        let registry = ActionRegistry::new();

        // Test navigate conversion
        let navigate_params = json!({
            "url": "https://google.com"
        });
        let step = registry
            .convert_to_step("navigate", &navigate_params)
            .unwrap();
        match step.kind {
            rzn_core::StepKind::NavigateToUrl { url, .. } => {
                assert_eq!(url, "https://google.com");
            }
            _ => panic!("Wrong step kind"),
        }

        // Test click conversion with index
        let click_params = json!({
            "index": 5
        });
        let step = registry.convert_to_step("click", &click_params).unwrap();
        match step.kind {
            rzn_core::StepKind::ClickElement { selector, .. } => {
                assert_eq!(selector, "[data-highlight-index=\"5\"]");
            }
            _ => panic!("Wrong step kind"),
        }
    }
}
