use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

/// Core error types for the RZN browser automation framework
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RznError {
    /// Network-related errors (timeouts, connection failures)
    Network(NetworkError),

    /// DOM-related errors (element not found, stale element)
    Dom(DomError),

    /// Permission-related errors (debugger access denied, extension blocked)
    Permission(PermissionError),

    /// Validation errors (invalid parameters, malformed data)
    Validation(ValidationError),

    /// Execution errors (action failed, script error)
    Execution(ExecutionError),

    /// System errors (broker unavailable, transport failure)
    System(SystemError),
}

/// Network-related error details
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkError {
    pub kind: NetworkErrorKind,
    pub url: Option<String>,
    pub message: String,
    pub duration_ms: Option<u64>,
    pub retry_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkErrorKind {
    Timeout,
    ConnectionRefused,
    DnsFailure,
    SslError,
    HttpError { status_code: u16 },
    Unknown,
}

/// DOM-related error details
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DomError {
    pub kind: DomErrorKind,
    pub selector: Option<String>,
    pub element_info: Option<ElementInfo>,
    pub message: String,
    pub screenshot_path: Option<String>,
    pub dom_snapshot: Option<String>,
    pub suggested_selectors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DomErrorKind {
    ElementNotFound,
    StaleElement,
    ElementNotInteractable,
    ElementObscured,
    MultipleElementsFound,
    InvalidSelector,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ElementInfo {
    pub tag_name: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub text: Option<String>,
    pub attributes: HashMap<String, String>,
}

/// Permission-related error details
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PermissionError {
    pub kind: PermissionErrorKind,
    pub resource: String,
    pub message: String,
    pub required_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionErrorKind {
    DebuggerAccessDenied,
    ExtensionBlocked,
    PageAccessDenied,
    ClipboardAccessDenied,
    NotificationDenied,
}

/// Validation error details
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationError {
    pub kind: ValidationErrorKind,
    pub field: Option<String>,
    pub value: Option<serde_json::Value>,
    pub message: String,
    pub expected: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationErrorKind {
    MissingRequiredField,
    InvalidType,
    InvalidFormat,
    OutOfRange,
    SchemaViolation,
}

/// Execution error details
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionError {
    pub kind: ExecutionErrorKind,
    pub step_id: Option<String>,
    pub action: Option<String>,
    pub message: String,
    pub script_error: Option<ScriptError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionErrorKind {
    ActionFailed,
    ScriptError,
    Timeout,
    UserCancelled,
    StepSkipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScriptError {
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub stack_trace: Option<String>,
}

/// System error details
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemError {
    pub kind: SystemErrorKind,
    pub component: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SystemErrorKind {
    BrokerUnavailable,
    TransportFailure,
    ExtensionNotResponding,
    ChromeNotFound,
    InvalidConfiguration,
}

/// Error context that can be attached to any error
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ErrorContext {
    pub timestamp: String,
    pub workflow_id: Option<String>,
    pub step_index: Option<usize>,
    pub browser_info: Option<BrowserInfo>,
    pub environment: HashMap<String, String>,
    pub breadcrumbs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserInfo {
    pub user_agent: String,
    pub viewport: ViewportInfo,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewportInfo {
    pub width: u32,
    pub height: u32,
    pub device_pixel_ratio: f32,
}

/// Retry strategy for different error types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryStrategy {
    pub max_attempts: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_factor: f32,
    pub jitter: bool,
}

impl Default for RetryStrategy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay_ms: 1000,
            max_delay_ms: 30000,
            backoff_factor: 2.0,
            jitter: true,
        }
    }
}

impl RetryStrategy {
    /// Calculate delay for exponential backoff
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        let base_delay = self.initial_delay_ms as f32 * self.backoff_factor.powi(attempt as i32);
        let delay = base_delay.min(self.max_delay_ms as f32) as u64;

        if self.jitter {
            // Add random jitter up to 10% of delay
            let jitter = (delay as f32 * 0.1 * rand::random::<f32>()) as u64;
            Duration::from_millis(delay + jitter)
        } else {
            Duration::from_millis(delay)
        }
    }

    /// Network errors: exponential backoff
    pub fn for_network() -> Self {
        Self {
            max_attempts: 5,
            initial_delay_ms: 2000,
            max_delay_ms: 60000,
            backoff_factor: 2.0,
            jitter: true,
        }
    }

    /// DOM errors: immediate retry with CDP fallback
    pub fn for_dom() -> Self {
        Self {
            max_attempts: 2,
            initial_delay_ms: 500,
            max_delay_ms: 2000,
            backoff_factor: 1.5,
            jitter: false,
        }
    }

    /// Permission errors: no automatic retry (user intervention required)
    pub fn for_permission() -> Self {
        Self {
            max_attempts: 1,
            initial_delay_ms: 0,
            max_delay_ms: 0,
            backoff_factor: 1.0,
            jitter: false,
        }
    }

    /// Validation errors: no retry
    pub fn for_validation() -> Self {
        Self {
            max_attempts: 1,
            initial_delay_ms: 0,
            max_delay_ms: 0,
            backoff_factor: 1.0,
            jitter: false,
        }
    }

    /// Execution errors: limited retry
    pub fn for_execution() -> Self {
        Self {
            max_attempts: 2,
            initial_delay_ms: 1000,
            max_delay_ms: 5000,
            backoff_factor: 2.0,
            jitter: true,
        }
    }
}

/// Error recovery suggestions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoverySuggestion {
    pub action: RecoveryAction,
    pub description: String,
    pub automated: bool,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    RetryWithBackoff,
    UseCdpFallback,
    UpdateSelector,
    PromptUser,
    SkipStep,
    RestartWorkflow,
    CheckPermissions,
    UpdateConfiguration,
}

impl RznError {
    /// Get retry strategy for this error type
    pub fn retry_strategy(&self) -> RetryStrategy {
        match self {
            RznError::Network(_) => RetryStrategy::for_network(),
            RznError::Dom(_) => RetryStrategy::for_dom(),
            RznError::Permission(_) => RetryStrategy::for_permission(),
            RznError::Validation(_) => RetryStrategy::for_validation(),
            RznError::Execution(_) => RetryStrategy::for_execution(),
            RznError::System(_) => RetryStrategy::default(),
        }
    }

    /// Get recovery suggestions for this error
    pub fn recovery_suggestions(&self) -> Vec<RecoverySuggestion> {
        match self {
            RznError::Network(e) => Self::network_recovery_suggestions(e),
            RznError::Dom(e) => Self::dom_recovery_suggestions(e),
            RznError::Permission(e) => Self::permission_recovery_suggestions(e),
            RznError::Validation(e) => Self::validation_recovery_suggestions(e),
            RznError::Execution(e) => Self::execution_recovery_suggestions(e),
            RznError::System(e) => Self::system_recovery_suggestions(e),
        }
    }

    fn network_recovery_suggestions(error: &NetworkError) -> Vec<RecoverySuggestion> {
        let mut suggestions = vec![];

        match error.kind {
            NetworkErrorKind::Timeout => {
                suggestions.push(RecoverySuggestion {
                    action: RecoveryAction::RetryWithBackoff,
                    description: "Retry with exponential backoff".to_string(),
                    automated: true,
                    confidence: 0.9,
                });
            }
            NetworkErrorKind::ConnectionRefused => {
                suggestions.push(RecoverySuggestion {
                    action: RecoveryAction::CheckPermissions,
                    description: "Check if the site is accessible".to_string(),
                    automated: false,
                    confidence: 0.7,
                });
            }
            NetworkErrorKind::SslError => {
                suggestions.push(RecoverySuggestion {
                    action: RecoveryAction::UpdateConfiguration,
                    description: "Review SSL certificate settings".to_string(),
                    automated: false,
                    confidence: 0.6,
                });
            }
            _ => {}
        }

        suggestions
    }

    fn dom_recovery_suggestions(error: &DomError) -> Vec<RecoverySuggestion> {
        let mut suggestions = vec![];

        match error.kind {
            DomErrorKind::ElementNotFound => {
                if !error.suggested_selectors.is_empty() {
                    suggestions.push(RecoverySuggestion {
                        action: RecoveryAction::UpdateSelector,
                        description: format!(
                            "Try alternative selectors: {:?}",
                            error.suggested_selectors
                        ),
                        automated: true,
                        confidence: 0.8,
                    });
                }

                suggestions.push(RecoverySuggestion {
                    action: RecoveryAction::UseCdpFallback,
                    description: "Use Chrome DevTools Protocol for element search".to_string(),
                    automated: true,
                    confidence: 0.7,
                });
            }
            DomErrorKind::StaleElement => {
                suggestions.push(RecoverySuggestion {
                    action: RecoveryAction::RetryWithBackoff,
                    description: "Re-query element and retry".to_string(),
                    automated: true,
                    confidence: 0.9,
                });
            }
            DomErrorKind::ElementNotInteractable => {
                suggestions.push(RecoverySuggestion {
                    action: RecoveryAction::SkipStep,
                    description: "Skip this interaction and continue".to_string(),
                    automated: false,
                    confidence: 0.5,
                });
            }
            _ => {}
        }

        suggestions
    }

    fn permission_recovery_suggestions(error: &PermissionError) -> Vec<RecoverySuggestion> {
        vec![
            RecoverySuggestion {
                action: RecoveryAction::PromptUser,
                description: error.required_action.clone(),
                automated: false,
                confidence: 1.0,
            },
            RecoverySuggestion {
                action: RecoveryAction::CheckPermissions,
                description: "Review browser permissions and extension settings".to_string(),
                automated: false,
                confidence: 0.8,
            },
        ]
    }

    fn validation_recovery_suggestions(error: &ValidationError) -> Vec<RecoverySuggestion> {
        vec![RecoverySuggestion {
            action: RecoveryAction::UpdateConfiguration,
            description: format!("Fix validation error: {}", error.message),
            automated: false,
            confidence: 1.0,
        }]
    }

    fn execution_recovery_suggestions(error: &ExecutionError) -> Vec<RecoverySuggestion> {
        let mut suggestions = vec![];

        match error.kind {
            ExecutionErrorKind::Timeout => {
                suggestions.push(RecoverySuggestion {
                    action: RecoveryAction::RetryWithBackoff,
                    description: "Retry with longer timeout".to_string(),
                    automated: true,
                    confidence: 0.7,
                });
            }
            ExecutionErrorKind::ScriptError => {
                if let Some(script_err) = &error.script_error {
                    suggestions.push(RecoverySuggestion {
                        action: RecoveryAction::UpdateConfiguration,
                        description: format!(
                            "Fix script error at line {}",
                            script_err.line.unwrap_or(0)
                        ),
                        automated: false,
                        confidence: 0.9,
                    });
                }
            }
            ExecutionErrorKind::UserCancelled => {
                suggestions.push(RecoverySuggestion {
                    action: RecoveryAction::RestartWorkflow,
                    description: "Restart workflow from beginning".to_string(),
                    automated: false,
                    confidence: 0.6,
                });
            }
            _ => {}
        }

        suggestions
    }

    fn system_recovery_suggestions(error: &SystemError) -> Vec<RecoverySuggestion> {
        let mut suggestions = vec![];

        match error.kind {
            SystemErrorKind::BrokerUnavailable => {
                suggestions.push(RecoverySuggestion {
                    action: RecoveryAction::RestartWorkflow,
                    description: "Restart broker and retry".to_string(),
                    automated: false,
                    confidence: 0.8,
                });
            }
            SystemErrorKind::ExtensionNotResponding => {
                suggestions.push(RecoverySuggestion {
                    action: RecoveryAction::CheckPermissions,
                    description: "Check if extension is enabled and has permissions".to_string(),
                    automated: false,
                    confidence: 0.9,
                });
            }
            _ => {}
        }

        suggestions
    }

    /// Create a user-friendly error message
    pub fn user_message(&self) -> String {
        match self {
            RznError::Network(e) => format!("Network error: {}", e.message),
            RznError::Dom(e) => format!("Element error: {}", e.message),
            RznError::Permission(e) => format!("Permission required: {}", e.message),
            RznError::Validation(e) => format!("Invalid input: {}", e.message),
            RznError::Execution(e) => format!("Execution failed: {}", e.message),
            RznError::System(e) => format!("System error: {}", e.message),
        }
    }
}

impl fmt::Display for RznError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.user_message())
    }
}

impl std::error::Error for RznError {}

/// Result type for RZN operations
pub type RznResult<T> = Result<T, RznError>;

/// Helper to add random dependency for jitter calculation
mod rand {
    pub fn random<T>() -> T
    where
        T: RandomValue,
    {
        T::random()
    }

    pub trait RandomValue {
        fn random() -> Self;
    }

    impl RandomValue for f32 {
        fn random() -> Self {
            // Simple pseudo-random for jitter
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            ((now % 1000) as f32) / 1000.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_strategy_calculation() {
        let strategy = RetryStrategy::for_network();

        // Test exponential backoff
        let delay1 = strategy.calculate_delay(0);
        let delay2 = strategy.calculate_delay(1);
        let delay3 = strategy.calculate_delay(2);

        assert!(delay1.as_millis() >= 2000);
        assert!(delay2.as_millis() >= delay1.as_millis());
        assert!(delay3.as_millis() >= delay2.as_millis());
        assert!(delay3.as_millis() <= 60000);
    }

    #[test]
    fn test_error_serialization() {
        let error = RznError::Dom(DomError {
            kind: DomErrorKind::ElementNotFound,
            selector: Some("#submit-button".to_string()),
            element_info: None,
            message: "Submit button not found".to_string(),
            screenshot_path: None,
            dom_snapshot: None,
            suggested_selectors: vec!["button[type='submit']".to_string()],
        });

        let json = serde_json::to_string(&error).unwrap();
        let deserialized: RznError = serde_json::from_str(&json).unwrap();

        assert_eq!(error, deserialized);
    }

    #[test]
    fn test_recovery_suggestions() {
        let error = RznError::Network(NetworkError {
            kind: NetworkErrorKind::Timeout,
            url: Some("https://example.com".to_string()),
            message: "Request timed out".to_string(),
            duration_ms: Some(30000),
            retry_count: 1,
        });

        let suggestions = error.recovery_suggestions();
        assert!(!suggestions.is_empty());
        assert!(suggestions
            .iter()
            .any(|s| matches!(s.action, RecoveryAction::RetryWithBackoff)));
    }
}
