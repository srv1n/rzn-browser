//! Mode selection logic for RZN Browser Native
//!
//! Chooses between Light mode (no CDP) and Pro mode (CDP on-demand) based on
//! site characteristics, action requirements, and failure patterns.

use crate::element_ref::{InputRung, ResultEnvelope, TargetSpec};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Execution mode for RZN Browser Native
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionMode {
    /// Light mode: No CDP, DOM/Scripted events only
    Light,
    /// Pro mode: CDP available for cross-origin and complex actions
    Pro,
}

impl ExecutionMode {
    /// Get the maximum input rung available in this mode
    pub fn max_input_rung(self) -> InputRung {
        match self {
            Self::Light => InputRung::Scripted,
            Self::Pro => InputRung::Cdp,
        }
    }

    /// Check if this mode supports cross-origin actions
    pub fn supports_cross_origin(self) -> bool {
        match self {
            Self::Light => false,
            Self::Pro => true,
        }
    }
}

/// Reasons for escalating to Pro mode
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EscalationReason {
    /// Cross-origin iframe detected
    CrossOriginRequired,
    /// File upload action required
    FileUploadRequired,
    /// Shadow DOM manipulation needed
    ShadowDomRequired,
    /// CSP blocking standard events
    CspBlocking,
    /// Action failed in lower rungs
    ActionFailure,
    /// User explicitly requested Pro mode
    UserRequested,
    /// Complex automation pattern detected
    ComplexAutomation,
}

impl EscalationReason {
    pub fn priority(&self) -> u8 {
        match self {
            Self::CrossOriginRequired => 10,
            Self::FileUploadRequired => 9,
            Self::ShadowDomRequired => 8,
            Self::CspBlocking => 7,
            Self::ActionFailure => 6,
            Self::ComplexAutomation => 5,
            Self::UserRequested => 3,
        }
    }
}

/// Tracks execution history and performance metrics
#[derive(Debug, Clone, Default)]
pub struct ExecutionHistory {
    /// Success/failure counts by mode
    light_successes: u32,
    light_failures: u32,
    pro_successes: u32,
    pro_failures: u32,

    /// Recent escalation reasons
    recent_escalations: Vec<EscalationReason>,

    /// Sites that consistently fail in Light mode
    problematic_domains: HashSet<String>,

    /// Action types that consistently need Pro mode
    pro_required_actions: HashSet<String>,
}

impl ExecutionHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a successful execution
    pub fn record_success(&mut self, mode: ExecutionMode) {
        match mode {
            ExecutionMode::Light => self.light_successes += 1,
            ExecutionMode::Pro => self.pro_successes += 1,
        }
    }

    /// Record a failed execution
    pub fn record_failure(&mut self, mode: ExecutionMode, domain: &str, action_type: &str) {
        match mode {
            ExecutionMode::Light => {
                self.light_failures += 1;

                // Track problematic domains
                if self.get_light_failure_rate() > 0.5 {
                    self.problematic_domains.insert(domain.to_string());
                }
            }
            ExecutionMode::Pro => self.pro_failures += 1,
        }

        // Track actions that consistently fail in Light mode
        if mode == ExecutionMode::Light {
            self.pro_required_actions.insert(action_type.to_string());
        }
    }

    /// Record an escalation to Pro mode
    pub fn record_escalation(&mut self, reason: EscalationReason) {
        self.recent_escalations.push(reason);

        // Keep only recent escalations (last 50)
        if self.recent_escalations.len() > 50 {
            self.recent_escalations
                .drain(0..self.recent_escalations.len() - 50);
        }
    }

    /// Get Light mode failure rate
    pub fn get_light_failure_rate(&self) -> f64 {
        let total = self.light_successes + self.light_failures;
        if total == 0 {
            0.0
        } else {
            self.light_failures as f64 / total as f64
        }
    }

    /// Check if domain is problematic in Light mode
    pub fn is_problematic_domain(&self, domain: &str) -> bool {
        self.problematic_domains.contains(domain)
    }

    /// Check if action type requires Pro mode
    pub fn action_requires_pro(&self, action_type: &str) -> bool {
        self.pro_required_actions.contains(action_type)
    }

    /// Get common escalation reasons
    pub fn get_common_escalations(&self) -> Vec<EscalationReason> {
        let mut counts: HashMap<EscalationReason, u32> = HashMap::new();
        for reason in &self.recent_escalations {
            *counts.entry(reason.clone()).or_insert(0) += 1;
        }

        let mut sorted: Vec<_> = counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1)); // Sort by count descending

        sorted
            .into_iter()
            .map(|(reason, _)| reason)
            .take(5)
            .collect()
    }
}

/// Main mode selector that chooses execution mode
pub struct ModeSelector {
    /// Execution history and metrics
    history: ExecutionHistory,

    /// Current execution mode
    current_mode: ExecutionMode,

    /// Default mode preference
    default_mode: ExecutionMode,

    /// Force Pro mode for all actions (debug/override)
    force_pro_mode: bool,
}

impl Default for ModeSelector {
    fn default() -> Self {
        Self::new()
    }
}

impl ModeSelector {
    pub fn new() -> Self {
        Self {
            history: ExecutionHistory::new(),
            current_mode: ExecutionMode::Light,
            default_mode: ExecutionMode::Light,
            force_pro_mode: std::env::var("RZN_FORCE_PRO_MODE").is_ok(),
        }
    }

    /// Set default mode preference
    pub fn set_default_mode(&mut self, mode: ExecutionMode) {
        self.default_mode = mode;
        debug!("Default execution mode set to {:?}", mode);
    }

    /// Get current execution mode
    pub fn current_mode(&self) -> ExecutionMode {
        if self.force_pro_mode {
            ExecutionMode::Pro
        } else {
            self.current_mode
        }
    }

    /// Select mode for a given domain and action
    pub fn select_mode(
        &mut self,
        domain: &str,
        action_type: &str,
        target: &TargetSpec,
    ) -> ExecutionMode {
        if self.force_pro_mode {
            info!("Forcing Pro mode (RZN_FORCE_PRO_MODE set)");
            return ExecutionMode::Pro;
        }

        // Check escalation triggers based on target characteristics + observed failures.
        // Avoid domain-tuned rules: no baked-in per-site heuristics.
        let escalation_reasons = self.check_escalation_triggers(domain, action_type, target);

        let selected_mode = if escalation_reasons.is_empty() {
            self.default_mode
        } else {
            // Escalation required
            let highest_priority = escalation_reasons
                .iter()
                .max_by_key(|r| r.priority())
                .unwrap();

            info!("Escalating to Pro mode: {:?}", highest_priority);
            for reason in &escalation_reasons {
                self.history.record_escalation(reason.clone());
            }

            ExecutionMode::Pro
        };

        // Update current mode
        self.current_mode = selected_mode;

        debug!(
            "Selected mode {:?} for domain {} action {}",
            selected_mode, domain, action_type
        );

        selected_mode
    }

    /// Check for escalation triggers
    fn check_escalation_triggers(
        &self,
        domain: &str,
        action_type: &str,
        target: &TargetSpec,
    ) -> Vec<EscalationReason> {
        let mut reasons = Vec::new();

        // Cross-origin requirement
        if target.requires_cross_origin_handling(0) {
            reasons.push(EscalationReason::CrossOriginRequired);
        }

        // File upload actions
        if action_type.contains("file") || action_type.contains("upload") {
            reasons.push(EscalationReason::FileUploadRequired);
        }

        // Historical failures
        if self.history.is_problematic_domain(domain) {
            reasons.push(EscalationReason::ActionFailure);
        }

        // Action-specific requirements
        if self.history.action_requires_pro(action_type) {
            reasons.push(EscalationReason::ActionFailure);
        }

        reasons
    }

    /// Record execution result
    pub fn record_result<T>(
        &mut self,
        domain: &str,
        action_type: &str,
        result: &ResultEnvelope<T>,
    ) {
        if result.success {
            self.history.record_success(self.current_mode);
        } else {
            self.history
                .record_failure(self.current_mode, domain, action_type);

            // If Light mode failed, consider escalation for future actions
            if self.current_mode == ExecutionMode::Light {
                warn!(
                    "Light mode failed for {} on {}: {:?}",
                    action_type, domain, result.error
                );
            }
        }
    }

    /// Force escalation to Pro mode
    pub fn escalate_to_pro(&mut self, reason: EscalationReason) {
        info!("Manual escalation to Pro mode: {:?}", reason);
        self.current_mode = ExecutionMode::Pro;
        self.history.record_escalation(reason);
    }

    /// Reset to default mode (usually called at workflow start)
    pub fn reset_to_default(&mut self) {
        self.current_mode = self.default_mode;
        debug!("Reset to default mode: {:?}", self.default_mode);
    }

    /// Get mode selection statistics
    pub fn get_statistics(&self) -> ModeStatistics {
        ModeStatistics {
            light_success_rate: {
                let total = self.history.light_successes + self.history.light_failures;
                if total == 0 {
                    1.0
                } else {
                    self.history.light_successes as f64 / total as f64
                }
            },
            pro_success_rate: {
                let total = self.history.pro_successes + self.history.pro_failures;
                if total == 0 {
                    1.0
                } else {
                    self.history.pro_successes as f64 / total as f64
                }
            },
            total_escalations: self.history.recent_escalations.len(),
            common_escalation_reasons: self.history.get_common_escalations(),
            problematic_domains: self.history.problematic_domains.len(),
            current_mode: self.current_mode,
        }
    }
}

/// Statistics about mode selection and performance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeStatistics {
    pub light_success_rate: f64,
    pub pro_success_rate: f64,
    pub total_escalations: usize,
    pub common_escalation_reasons: Vec<EscalationReason>,
    pub problematic_domains: usize,
    pub current_mode: ExecutionMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escalation_priority() {
        assert!(
            EscalationReason::CrossOriginRequired.priority()
                > EscalationReason::ActionFailure.priority()
        );
        assert!(
            EscalationReason::FileUploadRequired.priority()
                > EscalationReason::UserRequested.priority()
        );
    }

    #[test]
    fn test_mode_selection() {
        let mut selector = ModeSelector::new();

        // Simple case - should use Light mode
        let mode = selector.select_mode("example.com", "click", &TargetSpec::from_css("button"));
        assert_eq!(mode, ExecutionMode::Light);

        // Cross-origin case - should escalate to Pro
        let cross_origin_target = TargetSpec::from_css("input").with_frame(1);
        let mode = selector.select_mode("example.com", "fill", &cross_origin_target);
        assert_eq!(mode, ExecutionMode::Pro);
    }

    #[test]
    fn test_execution_history() {
        let mut history = ExecutionHistory::new();

        // Record some failures
        history.record_failure(ExecutionMode::Light, "example.com", "click");
        history.record_failure(ExecutionMode::Light, "example.com", "click");
        history.record_success(ExecutionMode::Light);

        assert!(history.get_light_failure_rate() > 0.5);
        assert!(history.is_problematic_domain("example.com"));
    }

    #[test]
    fn test_execution_mode_capabilities() {
        assert_eq!(ExecutionMode::Light.max_input_rung(), InputRung::Scripted);
        assert_eq!(ExecutionMode::Pro.max_input_rung(), InputRung::Cdp);

        assert!(!ExecutionMode::Light.supports_cross_origin());
        assert!(ExecutionMode::Pro.supports_cross_origin());
    }
}
