use chrono::{DateTime, Utc};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

/// Tracks consecutive failures and coordinates recovery strategies
#[derive(Debug, Clone)]
pub struct FailureTracker {
    /// Number of consecutive failures without a successful step
    pub consecutive_failures: u32,
    /// Last detected error category
    pub last_error_category: Option<ErrorCategory>,
    /// Count of recovery attempts per strategy
    pub recovery_attempts: HashMap<RecoveryStrategy, u32>,
    /// History of recent errors for pattern detection
    pub error_history: VecDeque<(ErrorCategory, String, DateTime<Utc>)>,
    /// Maximum error history size
    pub max_history_size: usize,
}

/// Categories of errors for targeted recovery
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCategory {
    /// CSS selector not found in DOM
    SelectorNotFound,
    /// Action performed but DOM didn't change (e.g., click had no effect)
    ActionNoEffect,
    /// Navigation took too long to complete
    NavigationTimeout,
    /// CAPTCHA or popup detected blocking interaction
    CaptchaOrPopup,
    /// SPA hot reload detected (DOM structure changed without navigation)
    FrameSwap,
    /// Authentication wall detected (login overlay)
    AuthWall,
    /// Element exists but not interactable
    ElementNotInteractable,
    /// Network error or resource failed to load
    NetworkError,
    /// Unknown or unclassified error
    Unknown,
}

/// Recovery strategies in order of escalation
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RecoveryStrategy {
    /// Switch to native input methods (Enigo)
    EscalateToNative,
    /// Request fresh DOM snapshot with full analysis
    RefreshFullDom,
    /// Suggest different action family (e.g., Handle for popups)
    SuggestHandleFamily,
    /// Try broader/more generic selector
    BroaderSelector,
    /// Wait longer for dynamic content
    ExtendedWait,
    /// Request user intervention
    RequestUserIntervention,
    /// Abort the workflow
    AbortWorkflow,
}

/// Result of analyzing an error
#[derive(Debug, Clone)]
pub struct ErrorAnalysis {
    pub category: ErrorCategory,
    pub confidence: f32,
    pub details: String,
    pub dom_indicators: Vec<String>,
}

/// Recovery action to take
#[derive(Debug, Clone)]
pub struct RecoveryAction {
    pub strategy: RecoveryStrategy,
    pub llm_prefix: String,
    pub modifications: HashMap<String, String>,
    pub reasoning: String,
}

impl FailureTracker {
    pub fn new() -> Self {
        Self {
            consecutive_failures: 0,
            last_error_category: None,
            recovery_attempts: HashMap::new(),
            error_history: VecDeque::new(),
            max_history_size: 10,
        }
    }

    /// Record a successful step execution
    pub fn record_success(&mut self) {
        info!("[OK] Step succeeded - resetting failure counter");
        self.consecutive_failures = 0;
        // Don't clear recovery attempts - we might need them again
    }

    /// Record a failed step and determine recovery strategy
    pub fn record_failure(
        &mut self,
        error_message: &str,
        dom_content: Option<&str>,
    ) -> RecoveryAction {
        self.consecutive_failures += 1;

        // Analyze the error
        let analysis = self.analyze_error(error_message, dom_content);

        // Record in history
        self.error_history.push_back((
            analysis.category.clone(),
            error_message.to_string(),
            Utc::now(),
        ));

        // Trim history if needed
        while self.error_history.len() > self.max_history_size {
            self.error_history.pop_front();
        }

        self.last_error_category = Some(analysis.category.clone());

        // Determine recovery strategy based on consecutive failures and error type
        let recovery = self.determine_recovery_strategy(&analysis);

        // Track recovery attempt
        *self
            .recovery_attempts
            .entry(recovery.strategy.clone())
            .or_insert(0) += 1;

        warn!(
            "[ERROR] Failure #{}: {} -> Recovery: {:?}",
            self.consecutive_failures, analysis.details, recovery.strategy
        );

        recovery
    }

    /// Analyze error message and DOM to categorize the failure
    pub fn analyze_error(&self, error_message: &str, dom_content: Option<&str>) -> ErrorAnalysis {
        let error_lower = error_message.to_lowercase();

        // Check for selector not found
        if error_lower.contains("selector not found")
            || error_lower.contains("element not found")
            || error_lower.contains("no element matching")
        {
            return ErrorAnalysis {
                category: ErrorCategory::SelectorNotFound,
                confidence: 0.95,
                details: "Element selector not found in current DOM".to_string(),
                dom_indicators: vec![],
            };
        }

        // Check for timeout errors
        if error_lower.contains("timeout") || error_lower.contains("timed out") {
            if error_lower.contains("navigation") {
                return ErrorAnalysis {
                    category: ErrorCategory::NavigationTimeout,
                    confidence: 0.9,
                    details: "Navigation operation timed out".to_string(),
                    dom_indicators: vec![],
                };
            }
            // Generic timeout might be element wait
            return ErrorAnalysis {
                category: ErrorCategory::ActionNoEffect,
                confidence: 0.7,
                details: "Operation timed out waiting for expected result".to_string(),
                dom_indicators: vec![],
            };
        }

        // Check for interaction errors
        if error_lower.contains("not clickable")
            || error_lower.contains("not interactable")
            || error_lower.contains("intercepted")
        {
            return ErrorAnalysis {
                category: ErrorCategory::ElementNotInteractable,
                confidence: 0.85,
                details: "Element exists but cannot be interacted with".to_string(),
                dom_indicators: vec![],
            };
        }

        // Analyze DOM for auth walls and popups
        if let Some(dom) = dom_content {
            let dom_lower = dom.to_lowercase();

            // Check for auth walls
            if self.detect_auth_wall(&dom_lower) {
                return ErrorAnalysis {
                    category: ErrorCategory::AuthWall,
                    confidence: 0.8,
                    details: "Authentication wall detected blocking interaction".to_string(),
                    dom_indicators: vec![
                        "login modal detected".to_string(),
                        "sign-in overlay present".to_string(),
                    ],
                };
            }

            // Check for CAPTCHA
            if self.detect_captcha(&dom_lower) {
                return ErrorAnalysis {
                    category: ErrorCategory::CaptchaOrPopup,
                    confidence: 0.9,
                    details: "CAPTCHA or verification popup detected".to_string(),
                    dom_indicators: vec![
                        "captcha element found".to_string(),
                        "verification required".to_string(),
                    ],
                };
            }

            // Check for popups
            if self.detect_popup(&dom_lower) {
                return ErrorAnalysis {
                    category: ErrorCategory::CaptchaOrPopup,
                    confidence: 0.75,
                    details: "Modal popup detected blocking interaction".to_string(),
                    dom_indicators: vec![
                        "modal overlay active".to_string(),
                        "popup dialog visible".to_string(),
                    ],
                };
            }
        }

        // Check for network errors
        if error_lower.contains("network")
            || error_lower.contains("connection")
            || error_lower.contains("failed to fetch")
        {
            return ErrorAnalysis {
                category: ErrorCategory::NetworkError,
                confidence: 0.85,
                details: "Network error occurred during operation".to_string(),
                dom_indicators: vec![],
            };
        }

        // Default to unknown
        ErrorAnalysis {
            category: ErrorCategory::Unknown,
            confidence: 0.5,
            details: format!("Unclassified error: {}", error_message),
            dom_indicators: vec![],
        }
    }

    /// Determine recovery strategy based on failure count and error type
    fn determine_recovery_strategy(&self, analysis: &ErrorAnalysis) -> RecoveryAction {
        // Implement the architect's specified algorithm
        match self.consecutive_failures {
            1 => {
                // First failure: Try native input escalation
                RecoveryAction {
                    strategy: RecoveryStrategy::EscalateToNative,
                    llm_prefix: "[WARNING] action_failed: Escalating to native input methods for better reliability".to_string(),
                    modifications: {
                        let mut mods = HashMap::new();
                        mods.insert("use_native".to_string(), "true".to_string());
                        mods
                    },
                    reasoning: "First failure - trying native input methods which are more reliable".to_string(),
                }
            }
            2 => {
                // Second failure: Refresh DOM and get full analysis
                RecoveryAction {
                    strategy: RecoveryStrategy::RefreshFullDom,
                    llm_prefix: "[WARNING] dom_stale: Refreshing DOM analysis - page structure may have changed".to_string(),
                    modifications: {
                        let mut mods = HashMap::new();
                        mods.insert("force_dom_refresh".to_string(), "true".to_string());
                        mods.insert("full_analysis".to_string(), "true".to_string());
                        mods
                    },
                    reasoning: "Second failure - DOM might be stale, requesting fresh analysis".to_string(),
                }
            }
            3 => {
                // Third failure: Check for popups/overlays
                match &analysis.category {
                    ErrorCategory::CaptchaOrPopup | ErrorCategory::AuthWall => {
                        RecoveryAction {
                            strategy: RecoveryStrategy::SuggestHandleFamily,
                            llm_prefix: format!(
                                "[WARNING] {}: Detected blocking overlay - consider using Handle family actions",
                                match &analysis.category {
                                    ErrorCategory::CaptchaOrPopup => "captcha_popup",
                                    ErrorCategory::AuthWall => "auth_wall",
                                    _ => "overlay",
                                }
                            ),
                            modifications: {
                                let mut mods = HashMap::new();
                                mods.insert("action_family".to_string(), "Handle".to_string());
                                mods.insert("scan_for_overlays".to_string(), "true".to_string());
                                mods
                            },
                            reasoning: "Third failure with overlay detected - suggesting Handle family".to_string(),
                        }
                    }
                    ErrorCategory::SelectorNotFound => {
                        RecoveryAction {
                            strategy: RecoveryStrategy::BroaderSelector,
                            llm_prefix: "[WARNING] selector_not_found: Element missing after 3 attempts - try broader selector".to_string(),
                            modifications: {
                                let mut mods = HashMap::new();
                                mods.insert("selector_strategy".to_string(), "broad".to_string());
                                mods.insert("use_text_matching".to_string(), "true".to_string());
                                mods
                            },
                            reasoning: "Selector consistently not found - suggesting broader matching".to_string(),
                        }
                    }
                    _ => {
                        RecoveryAction {
                            strategy: RecoveryStrategy::ExtendedWait,
                            llm_prefix: "[WARNING] action_timeout: Page may be loading slowly - extending wait times".to_string(),
                            modifications: {
                                let mut mods = HashMap::new();
                                mods.insert("wait_multiplier".to_string(), "3".to_string());
                                mods.insert("check_loading_indicators".to_string(), "true".to_string());
                                mods
                            },
                            reasoning: "Generic third failure - trying extended waits".to_string(),
                        }
                    }
                }
            }
            4..=5 => {
                // Fourth/Fifth failure: Request user intervention
                RecoveryAction {
                    strategy: RecoveryStrategy::RequestUserIntervention,
                    llm_prefix: format!(
                        "[WARNING] stuck_need_help: {} consecutive failures - {}",
                        self.consecutive_failures, analysis.details
                    ),
                    modifications: {
                        let mut mods = HashMap::new();
                        mods.insert("request_user_help".to_string(), "true".to_string());
                        mods.insert("pause_execution".to_string(), "true".to_string());
                        mods
                    },
                    reasoning: format!(
                        "Multiple failures ({}) - requesting user intervention",
                        self.consecutive_failures
                    ),
                }
            }
            _ => {
                // 6+ failures: Abort
                RecoveryAction {
                    strategy: RecoveryStrategy::AbortWorkflow,
                    llm_prefix: format!(
                        "[WARNING] abort_stuck: {} consecutive failures - unable to proceed",
                        self.consecutive_failures
                    ),
                    modifications: {
                        let mut mods = HashMap::new();
                        mods.insert("abort".to_string(), "true".to_string());
                        mods
                    },
                    reasoning: "Too many consecutive failures - aborting workflow".to_string(),
                }
            }
        }
    }

    /// Detect authentication walls in DOM
    pub fn detect_auth_wall(&self, dom_lower: &str) -> bool {
        let auth_indicators = [
            "sign in",
            "log in",
            "signin",
            "login",
            "authentication required",
            "please authenticate",
            "create account",
            "register now",
            "modal-login",
            "auth-modal",
            "login-overlay",
            "signin-prompt",
            "auth-wall",
        ];

        let high_z_index = dom_lower.contains("z-index: 9")
            || dom_lower.contains("z-index:9")
            || dom_lower.contains("z-index: 10")
            || dom_lower.contains("z-index:10");

        let has_auth_text = auth_indicators
            .iter()
            .any(|&indicator| dom_lower.contains(indicator));
        let has_overlay = dom_lower.contains("overlay") || dom_lower.contains("modal");

        has_auth_text && (high_z_index || has_overlay)
    }

    /// Detect CAPTCHA elements in DOM
    pub fn detect_captcha(&self, dom_lower: &str) -> bool {
        let captcha_indicators = [
            "captcha",
            "recaptcha",
            "hcaptcha",
            "verify you're human",
            "verify you are human",
            "robot verification",
            "security check",
            "challenge-form",
            "cf-challenge",
        ];

        captcha_indicators
            .iter()
            .any(|&indicator| dom_lower.contains(indicator))
    }

    /// Detect generic popups in DOM
    pub fn detect_popup(&self, dom_lower: &str) -> bool {
        let popup_indicators = [
            "popup",
            "modal",
            "dialog",
            "lightbox",
            "overlay",
            "position: fixed",
            "position:fixed",
        ];

        let dismiss_indicators = ["close", "dismiss", "cancel", "×", "✕", "x-button"];

        let has_popup = popup_indicators.iter().any(|&ind| dom_lower.contains(ind));
        let has_dismiss = dismiss_indicators
            .iter()
            .any(|&ind| dom_lower.contains(ind));

        has_popup && has_dismiss
    }

    /// Get error pattern summary for LLM context
    pub fn get_error_summary(&self) -> String {
        if self.error_history.is_empty() {
            return "No recent errors".to_string();
        }

        let mut summary = format!(
            "Error Summary (last {} errors, {} consecutive failures):\n",
            self.error_history.len(),
            self.consecutive_failures
        );

        for (i, (category, message, timestamp)) in self.error_history.iter().enumerate() {
            summary.push_str(&format!(
                "  {}. {:?} at {} - {}\n",
                i + 1,
                category,
                timestamp.format("%H:%M:%S"),
                if message.len() > 50 {
                    format!("{}...", &message[..50])
                } else {
                    message.clone()
                }
            ));
        }

        // Add recovery attempts summary
        if !self.recovery_attempts.is_empty() {
            summary.push_str("\nRecovery attempts:\n");
            for (strategy, count) in &self.recovery_attempts {
                summary.push_str(&format!("  {:?}: {} times\n", strategy, count));
            }
        }

        summary
    }

    /// Check if we should abort based on error patterns
    pub fn should_abort(&self) -> bool {
        // Abort if too many consecutive failures
        if self.consecutive_failures > 6 {
            return true;
        }

        // Abort if same error repeats too many times
        if self.error_history.len() >= 5 {
            let recent_categories: Vec<_> = self
                .error_history
                .iter()
                .rev()
                .take(5)
                .map(|(cat, _, _)| cat)
                .collect();

            // If last 5 errors are all the same category
            if recent_categories.windows(2).all(|w| w[0] == w[1]) {
                return true;
            }
        }

        // Abort if we've tried user intervention multiple times
        if self
            .recovery_attempts
            .get(&RecoveryStrategy::RequestUserIntervention)
            .unwrap_or(&0)
            > &2
        {
            return true;
        }

        false
    }
}

/// DOM change detection for frame swaps and dynamic updates
#[derive(Debug, Clone)]
pub struct DomChangeDetector {
    pub last_structure_hash: Option<u64>,
    pub last_viewport_hash: Option<u64>,
    pub last_url: Option<String>,
}

impl DomChangeDetector {
    pub fn new() -> Self {
        Self {
            last_structure_hash: None,
            last_viewport_hash: None,
            last_url: None,
        }
    }

    /// Calculate structure hash for DOM
    pub fn calculate_structure_hash(dom: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Extract structural elements only
        let structural_elements = [
            "div", "section", "main", "nav", "header", "footer", "article",
        ];
        let mut structure = String::new();

        for element in &structural_elements {
            let count = dom.matches(&format!("<{}", element)).count();
            structure.push_str(&format!("{}:{},", element, count));
        }

        let mut hasher = DefaultHasher::new();
        structure.hash(&mut hasher);
        hasher.finish()
    }

    /// Calculate viewport hash for visible content
    pub fn calculate_viewport_hash(dom: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Simple heuristic: hash visible text content
        // In real implementation, would use viewport coordinates
        let visible_text: String = dom
            .split('<')
            .filter_map(|s| s.split('>').nth(1))
            .filter(|s| !s.trim().is_empty() && s.len() < 100)
            .take(50)
            .collect::<Vec<_>>()
            .join(" ");

        let mut hasher = DefaultHasher::new();
        visible_text.hash(&mut hasher);
        hasher.finish()
    }

    /// Detect frame swap or major DOM change
    pub fn detect_frame_swap(&mut self, dom: &str, current_url: &str) -> Option<ErrorCategory> {
        let structure_hash = Self::calculate_structure_hash(dom);
        let viewport_hash = Self::calculate_viewport_hash(dom);

        let result = if let (Some(last_struct), Some(last_url)) =
            (self.last_structure_hash, &self.last_url)
        {
            if last_url == current_url {
                // Calculate percentage difference safely
                let diff = if structure_hash > last_struct {
                    structure_hash - last_struct
                } else {
                    last_struct - structure_hash
                };

                // Same URL but >10% structure change = frame swap
                if diff > (last_struct / 10) {
                    Some(ErrorCategory::FrameSwap)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Update state
        self.last_structure_hash = Some(structure_hash);
        self.last_viewport_hash = Some(viewport_hash);
        self.last_url = Some(current_url.to_string());

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_failure_escalation() {
        let mut tracker = FailureTracker::new();

        // First failure
        let action1 = tracker.record_failure("Element not found", None);
        assert_eq!(action1.strategy, RecoveryStrategy::EscalateToNative);
        assert_eq!(tracker.consecutive_failures, 1);

        // Second failure
        let action2 = tracker.record_failure("Still not found", None);
        assert_eq!(action2.strategy, RecoveryStrategy::RefreshFullDom);
        assert_eq!(tracker.consecutive_failures, 2);

        // Success resets counter
        tracker.record_success();
        assert_eq!(tracker.consecutive_failures, 0);
    }

    #[test]
    fn test_error_categorization() {
        let tracker = FailureTracker::new();

        let analysis = tracker.analyze_error("Selector not found: .submit-button", None);
        assert_eq!(analysis.category, ErrorCategory::SelectorNotFound);
        assert!(analysis.confidence > 0.9);

        let analysis = tracker.analyze_error("Element not clickable at point", None);
        assert_eq!(analysis.category, ErrorCategory::ElementNotInteractable);
    }

    #[test]
    fn test_auth_wall_detection() {
        let tracker = FailureTracker::new();

        let dom_with_auth = r#"
            <div class="modal-overlay" style="z-index: 9999">
                <div class="auth-modal">
                    <h2>Please sign in to continue</h2>
                    <button>Log In</button>
                </div>
            </div>
        "#;

        assert!(tracker.detect_auth_wall(&dom_with_auth.to_lowercase()));

        let dom_without_auth = r#"
            <div class="content">
                <h1>Welcome</h1>
                <p>Some content here</p>
            </div>
        "#;

        assert!(!tracker.detect_auth_wall(&dom_without_auth.to_lowercase()));
    }

    #[test]
    fn test_dom_change_detection() {
        let mut detector = DomChangeDetector::new();

        let dom1 = "<div><section>Content 1</section></div>";
        let dom2 = "<main><article>Completely different</article></main>";

        // First check establishes baseline
        assert!(detector
            .detect_frame_swap(dom1, "https://example.com")
            .is_none());

        // Major structure change with same URL = frame swap
        assert_eq!(
            detector.detect_frame_swap(dom2, "https://example.com"),
            Some(ErrorCategory::FrameSwap)
        );
    }
}
