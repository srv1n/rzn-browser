use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Wait strategies for handling dynamic content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitStrategy {
    /// Wait for a specific selector to appear
    pub wait_for_selector: Option<String>,

    /// Wait for specific text to appear in the page
    pub wait_for_text: Option<String>,

    /// Wait for DOM to stabilize (no changes for X ms)
    pub wait_for_stability: Option<u64>,

    /// Maximum time to wait in milliseconds
    pub max_wait: Option<u64>,

    /// Check interval in milliseconds
    pub check_interval: Option<u64>,

    /// Observe DOM changes after action
    pub observe_after_action: bool,

    /// Observation duration in milliseconds
    pub observation_duration: Option<u64>,
}

impl Default for WaitStrategy {
    fn default() -> Self {
        Self {
            wait_for_selector: None,
            wait_for_text: None,
            wait_for_stability: None,
            max_wait: Some(5000),
            check_interval: Some(100),
            observe_after_action: true,
            observation_duration: Some(500),
        }
    }
}

impl WaitStrategy {
    /// Create a wait strategy for navigation
    pub fn for_navigation() -> Self {
        Self {
            wait_for_stability: Some(500),
            max_wait: Some(10000),
            observe_after_action: true,
            observation_duration: Some(1000),
            ..Default::default()
        }
    }

    /// Create a wait strategy for form submission
    pub fn for_form_submit() -> Self {
        Self {
            wait_for_stability: Some(300),
            max_wait: Some(5000),
            observe_after_action: true,
            observation_duration: Some(800),
            ..Default::default()
        }
    }

    /// Create a wait strategy for clicking buttons
    pub fn for_click() -> Self {
        Self {
            wait_for_stability: Some(200),
            max_wait: Some(3000),
            observe_after_action: true,
            observation_duration: Some(500),
            ..Default::default()
        }
    }

    /// Create a wait strategy for input fields
    pub fn for_input() -> Self {
        Self {
            wait_for_stability: Some(100),
            max_wait: Some(2000),
            observe_after_action: true,
            observation_duration: Some(300),
            ..Default::default()
        }
    }

    /// Create a wait strategy for specific selector
    pub fn for_selector(selector: &str, timeout: u64) -> Self {
        Self {
            wait_for_selector: Some(selector.to_string()),
            max_wait: Some(timeout),
            observe_after_action: true,
            ..Default::default()
        }
    }
}

/// Context-aware wait strategies based on action type and site
#[derive(Debug, Clone)]
pub struct SmartWaitStrategy {
    /// Base wait strategy
    pub base: WaitStrategy,

    /// Site-specific overrides
    pub site_overrides: Vec<SiteOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteOverride {
    /// Hostname pattern (e.g., "example.com", "*.example.com")
    pub hostname_pattern: String,

    /// Action type pattern (e.g., "fill_input_field", "click_*")
    pub action_pattern: String,

    /// Override wait strategy for this site/action combo
    pub strategy: WaitStrategy,
}

impl Default for SmartWaitStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl SmartWaitStrategy {
    pub fn new() -> Self {
        Self {
            base: WaitStrategy::default(),
            // Intentionally empty by default: avoid baking in domain-tuned behavior.
            // Workflows can override via explicit timeouts, or callers can populate `site_overrides`.
            site_overrides: vec![],
        }
    }

    /// Get the appropriate wait strategy for a given URL and action
    pub fn get_strategy(&self, url: &str, action_type: &str) -> WaitStrategy {
        // Extract hostname from URL
        let hostname = url
            .split("//")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .unwrap_or("");

        // Find matching override
        for override_rule in &self.site_overrides {
            if self.matches_pattern(&override_rule.hostname_pattern, hostname)
                && self.matches_pattern(&override_rule.action_pattern, action_type)
            {
                return override_rule.strategy.clone();
            }
        }

        // Return action-specific default
        match action_type {
            "navigate_to_url" => WaitStrategy::for_navigation(),
            "submit_input" | "press_special_key" => WaitStrategy::for_form_submit(),
            "click_element" | "dbl_click_element" => WaitStrategy::for_click(),
            "fill_input_field" | "type_text" => WaitStrategy::for_input(),
            _ => self.base.clone(),
        }
    }

    /// Simple pattern matching (supports * wildcard)
    fn matches_pattern(&self, pattern: &str, value: &str) -> bool {
        if pattern == "*" {
            return true;
        }

        if pattern.starts_with('*') {
            value.ends_with(&pattern[1..])
        } else if pattern.ends_with('*') {
            value.starts_with(&pattern[..pattern.len() - 1])
        } else {
            pattern == value
        }
    }
}

/// Information about DOM changes after an action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DOMObservation {
    pub has_significant_changes: bool,
    pub new_interactive_elements: usize,
    pub dom_stabilized: bool,
    pub observation_duration_ms: u64,
    pub changes_count: usize,
}

impl DOMObservation {
    /// Determine if we should wait longer based on observations
    pub fn needs_additional_wait(&self) -> Option<Duration> {
        if !self.dom_stabilized {
            // DOM still changing, wait more
            Some(Duration::from_millis(300))
        } else if self.new_interactive_elements > 5 {
            // Many new elements appeared, give them time to settle
            Some(Duration::from_millis(200))
        } else if self.new_interactive_elements > 0 {
            // Some new elements, short wait
            Some(Duration::from_millis(100))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smart_wait_strategy() {
        let strategy = SmartWaitStrategy::new();

        // Defaults are action-based, not domain-tuned.
        let wait = strategy.get_strategy("https://example.com", "fill_input_field");
        assert_eq!(wait.wait_for_stability, Some(100));
        assert_eq!(wait.observation_duration, Some(300));

        let wait = strategy.get_strategy("https://example.com", "submit_input");
        assert_eq!(wait.max_wait, Some(5000));

        // Test generic navigation
        let wait = strategy.get_strategy("https://example.com", "navigate_to_url");
        assert_eq!(wait.wait_for_stability, Some(500));
        assert_eq!(wait.max_wait, Some(10000));
    }

    #[test]
    fn test_pattern_matching() {
        let strategy = SmartWaitStrategy::new();

        assert!(strategy.matches_pattern("*example.com", "example.com"));
        assert!(strategy.matches_pattern("*example.com", "www.example.com"));
        assert!(strategy.matches_pattern("*example.com", "m.example.com"));
        assert!(!strategy.matches_pattern("*example.com", "example.co.uk"));

        assert!(strategy.matches_pattern("example.*", "example.com"));
        assert!(strategy.matches_pattern("example.*", "example.co.uk"));
        assert!(!strategy.matches_pattern("example.*", "sample.com"));
    }
}
