use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionError {
    pub action_type: String,
    pub error_code: String,
    pub error_message: String,
    pub context: HashMap<String, serde_json::Value>,
    pub recovery_suggestions: Vec<RecoverySuggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoverySuggestion {
    pub strategy: String,
    pub description: String,
    pub alternative_action: Option<AlternativeAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlternativeAction {
    pub action_type: String,
    pub parameters: serde_json::Value,
    pub rationale: String,
}

/// Handles errors from action execution and suggests recovery strategies
pub struct ActionErrorHandler;

impl ActionErrorHandler {
    pub fn new() -> Self {
        Self
    }

    /// Analyze an error and suggest recovery strategies
    pub fn analyze_error(&self, error: &ActionError) -> Vec<RecoverySuggestion> {
        let mut suggestions = Vec::new();

        match error.error_code.as_str() {
            "SELECTOR_NOT_FOUND" => {
                suggestions.push(RecoverySuggestion {
                    strategy: "wait_and_retry".to_string(),
                    description: "Wait for the element to appear and retry".to_string(),
                    alternative_action: Some(AlternativeAction {
                        action_type: "wait".to_string(),
                        parameters: serde_json::json!({"seconds": 2}),
                        rationale: "Element may not be loaded yet".to_string(),
                    }),
                });
                suggestions.push(RecoverySuggestion {
                    strategy: "alternative_selector".to_string(),
                    description: "Search for element by text content instead".to_string(),
                    alternative_action: None,
                });
                suggestions.push(RecoverySuggestion {
                    strategy: "scroll_first".to_string(),
                    description: "Scroll down to reveal more elements".to_string(),
                    alternative_action: Some(AlternativeAction {
                        action_type: "scroll".to_string(),
                        parameters: serde_json::json!({"direction": "down"}),
                        rationale: "Element may be below the fold".to_string(),
                    }),
                });
            }

            "NAVIGATION_ERROR" => {
                suggestions.push(RecoverySuggestion {
                    strategy: "fix_url".to_string(),
                    description: "Ensure URL has proper protocol (https://)".to_string(),
                    alternative_action: None,
                });
                suggestions.push(RecoverySuggestion {
                    strategy: "retry".to_string(),
                    description: "Network issue may be temporary".to_string(),
                    alternative_action: None,
                });
            }

            "TIMEOUT" => {
                suggestions.push(RecoverySuggestion {
                    strategy: "adjust_timeout".to_string(),
                    description: "Page may be slow to load".to_string(),
                    alternative_action: None,
                });
                suggestions.push(RecoverySuggestion {
                    strategy: "verify_connection".to_string(),
                    description: "Ensure internet connection is stable".to_string(),
                    alternative_action: None,
                });
            }

            "ELEMENT_NOT_CLICKABLE" => {
                suggestions.push(RecoverySuggestion {
                    strategy: "wait_clickable".to_string(),
                    description: "Element exists but not interactive yet".to_string(),
                    alternative_action: Some(AlternativeAction {
                        action_type: "wait".to_string(),
                        parameters: serde_json::json!({"seconds": 1}),
                        rationale: "Element may be loading or covered".to_string(),
                    }),
                });
                suggestions.push(RecoverySuggestion {
                    strategy: "scroll_to_view".to_string(),
                    description: "Element may be outside viewport".to_string(),
                    alternative_action: None,
                });
            }

            _ => {
                // Generic suggestions
                suggestions.push(RecoverySuggestion {
                    strategy: "generic_retry".to_string(),
                    description: "Retry the action after a short delay".to_string(),
                    alternative_action: Some(AlternativeAction {
                        action_type: "wait".to_string(),
                        parameters: serde_json::json!({"seconds": 1}),
                        rationale: "Give the page time to stabilize".to_string(),
                    }),
                });
            }
        }

        suggestions
    }

    /// Create an ActionError from execution results
    pub fn create_error(
        action_type: String,
        error_code: String,
        error_message: String,
        context: HashMap<String, serde_json::Value>,
    ) -> ActionError {
        ActionError {
            action_type,
            error_code,
            error_message,
            context,
            recovery_suggestions: Vec::new(),
        }
    }

    /// Check if an error is recoverable
    pub fn is_recoverable(&self, error: &ActionError) -> bool {
        // Most errors are potentially recoverable
        match error.error_code.as_str() {
            "INVALID_ACTION" | "VALIDATION_ERROR" => false,
            _ => true,
        }
    }

    /// Get retry delay suggestion based on error type
    pub fn get_retry_delay(&self, error: &ActionError, attempt: u32) -> Option<u64> {
        match error.error_code.as_str() {
            "TIMEOUT" => Some(5000 * attempt as u64), // Exponential backoff
            "SELECTOR_NOT_FOUND" => Some(2000),
            "ELEMENT_NOT_CLICKABLE" => Some(1000),
            "NAVIGATION_ERROR" => Some(3000),
            _ => Some(1000), // Default 1 second
        }
    }
}

impl Default for ActionErrorHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_analysis() {
        let handler = ActionErrorHandler::new();

        let error = ActionError {
            action_type: "click".to_string(),
            error_code: "SELECTOR_NOT_FOUND".to_string(),
            error_message: "Element not found".to_string(),
            context: HashMap::new(),
            recovery_suggestions: Vec::new(),
        };

        let suggestions = handler.analyze_error(&error);
        assert!(!suggestions.is_empty());
        assert!(suggestions.iter().any(|s| s.strategy == "wait_and_retry"));
    }

    #[test]
    fn test_retry_delay() {
        let handler = ActionErrorHandler::new();

        let timeout_error = ActionError {
            action_type: "navigate".to_string(),
            error_code: "TIMEOUT".to_string(),
            error_message: "Navigation timeout".to_string(),
            context: HashMap::new(),
            recovery_suggestions: Vec::new(),
        };

        assert_eq!(handler.get_retry_delay(&timeout_error, 1), Some(5000));
        assert_eq!(handler.get_retry_delay(&timeout_error, 2), Some(10000));
    }
}
