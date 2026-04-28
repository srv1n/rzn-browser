use crate::{PlanError, PlanResult, StepExecution};
use log::{debug, info, warn};
use rzn_core::Step;

/// Self-healing system for automatically fixing broken workflows
pub struct SelfHealer {
    max_attempts: u32,
}

impl SelfHealer {
    pub fn new(max_attempts: u32) -> Self {
        Self { max_attempts }
    }

    /// Attempt to heal a failed step
    pub async fn heal_step(&self, failed_step: &Step, error_message: &str) -> PlanResult<Step> {
        info!(
            "Attempting to heal failed step: {} - {}",
            failed_step.id, failed_step.name
        );
        debug!("Error message: {}", error_message);

        // For now, implement basic healing strategies
        // TODO: Integrate with LLM for intelligent healing

        let healed_step = self.apply_healing_strategies(failed_step, error_message)?;

        info!(
            "Successfully healed step: {} -> {}",
            failed_step.id, healed_step.id
        );
        Ok(healed_step)
    }

    /// Apply various healing strategies to fix the step
    fn apply_healing_strategies(&self, step: &Step, error: &str) -> PlanResult<Step> {
        // Strategy 1: Selector not found - try alternative selectors
        if error.contains("selector not found") || error.contains("element not found") {
            return self.heal_selector_not_found(step);
        }

        // Strategy 2: Timeout - increase wait time
        if error.contains("timeout") || error.contains("timed out") {
            return self.heal_timeout(step);
        }

        // Strategy 3: Element not clickable - try different approach
        if error.contains("not clickable") || error.contains("not interactable") {
            return self.heal_not_clickable(step);
        }

        // Strategy 4: Navigation failed - retry with different approach
        if error.contains("navigation") || error.contains("failed to navigate") {
            return self.heal_navigation_failed(step);
        }

        // Default: return original step with modified ID
        let mut healed = step.clone();
        healed.id = format!("{}_healed", step.id);
        healed.name = format!("{} (healed)", step.name);
        Ok(healed)
    }

    /// Heal selector not found errors by trying alternative selectors
    fn heal_selector_not_found(&self, step: &Step) -> PlanResult<Step> {
        let mut healed = step.clone();
        healed.id = format!("{}_healed_selector", step.id);
        healed.name = format!("{} (healed selector)", step.name);

        match &step.kind {
            rzn_core::StepKind::ClickElement {
                selector,
                random_offset,
                timeout_ms,
                frame_id: _,
            } => {
                // Try more generic selectors
                let alternative_selector = self.generate_alternative_selector(selector);
                healed.kind = rzn_core::StepKind::ClickElement {
                    selector: alternative_selector,
                    frame_id: None,
                    random_offset: *random_offset,
                    timeout_ms: *timeout_ms,
                };
            }
            rzn_core::StepKind::FillInputField {
                selector,
                value,
                clear_first,
                simulate_typing,
                delay_ms,
                timeout_ms,
                frame_id: _,
            } => {
                let alternative_selector = self.generate_alternative_selector(selector);
                healed.kind = rzn_core::StepKind::FillInputField {
                    selector: alternative_selector,
                    value: value.clone(),
                    frame_id: None,
                    clear_first: *clear_first,
                    simulate_typing: *simulate_typing,
                    delay_ms: *delay_ms,
                    timeout_ms: *timeout_ms,
                };
            }
            rzn_core::StepKind::WaitForElement {
                selector,
                condition,
                timeout_ms,
                frame_id: _,
            } => {
                let alternative_selector = self.generate_alternative_selector(selector);
                healed.kind = rzn_core::StepKind::WaitForElement {
                    selector: alternative_selector,
                    frame_id: None,
                    condition: condition.clone(),
                    timeout_ms: *timeout_ms,
                };
            }
            _ => {
                return Err(PlanError::HealingFailed { attempts: 1 });
            }
        }

        Ok(healed)
    }

    /// Heal timeout errors by increasing wait times
    fn heal_timeout(&self, step: &Step) -> PlanResult<Step> {
        let mut healed = step.clone();
        healed.id = format!("{}_healed_timeout", step.id);
        healed.name = format!("{} (healed timeout)", step.name);

        match &step.kind {
            rzn_core::StepKind::WaitForElement {
                selector,
                condition,
                timeout_ms,
                frame_id: _,
            } => {
                let new_timeout = timeout_ms.map(|t| t * 2).or(Some(10000)); // Double timeout or set to 10s
                healed.kind = rzn_core::StepKind::WaitForElement {
                    selector: selector.clone(),
                    frame_id: None,
                    condition: condition.clone(),
                    timeout_ms: new_timeout,
                };
            }
            rzn_core::StepKind::WaitForTimeout { timeout_ms } => {
                healed.kind = rzn_core::StepKind::WaitForTimeout {
                    timeout_ms: timeout_ms * 2, // Double the timeout
                };
            }
            _ => {
                // Add a wait before the original step
                return Ok(Step {
                    id: format!("{}_with_wait", step.id),
                    name: format!("Wait then {}", step.name),
                    kind: rzn_core::StepKind::WaitForTimeout { timeout_ms: 2000 },
                });
            }
        }

        Ok(healed)
    }

    /// Heal not clickable errors by trying alternative approaches
    fn heal_not_clickable(&self, step: &Step) -> PlanResult<Step> {
        let mut healed = step.clone();
        healed.id = format!("{}_healed_clickable", step.id);
        healed.name = format!("{} (healed clickable)", step.name);

        match &step.kind {
            rzn_core::StepKind::ClickElement {
                selector,
                random_offset,
                timeout_ms,
                frame_id: _,
            } => {
                // Try hovering first, then clicking
                healed.kind = rzn_core::StepKind::HoverElement {
                    selector: selector.clone(),
                    frame_id: None,
                    random_offset: *random_offset,
                    timeout_ms: *timeout_ms,
                };
            }
            _ => {
                return Err(PlanError::HealingFailed { attempts: 1 });
            }
        }

        Ok(healed)
    }

    /// Heal navigation failures
    fn heal_navigation_failed(&self, step: &Step) -> PlanResult<Step> {
        let mut healed = step.clone();
        healed.id = format!("{}_healed_nav", step.id);
        healed.name = format!("{} (healed navigation)", step.name);

        match &step.kind {
            rzn_core::StepKind::NavigateToUrl { url, wait: _ } => {
                // Try with different wait condition
                healed.kind = rzn_core::StepKind::NavigateToUrl {
                    url: url.clone(),
                    wait: Some("networkidle".to_string()),
                };
            }
            _ => {
                return Err(PlanError::HealingFailed { attempts: 1 });
            }
        }

        Ok(healed)
    }

    /// Generate alternative CSS selectors
    fn generate_alternative_selector(&self, original: &str) -> String {
        // Simple heuristics for generating alternative selectors
        if original.starts_with('#') {
            // If it's an ID selector, try a more generic approach
            let id = &original[1..];
            format!("[id='{}'], [id*='{}']", id, id)
        } else if original.starts_with('.') {
            // If it's a class selector, try partial matching
            let class = &original[1..];
            format!("[class*='{}']", class)
        } else if original.contains("button") {
            // For button selectors, try multiple approaches
            format!("button, [role='button'], input[type='button'], input[type='submit']")
        } else if original.contains("input") {
            // For input selectors, try by type and name
            format!("input, [type='text'], [type='email'], [type='password']")
        } else {
            // Default: try a more generic version
            format!(
                "{}, *[class*='{}']",
                original,
                original.replace(['#', '.', '[', ']'], "")
            )
        }
    }

    /// Heal a workflow by fixing multiple failed steps
    pub async fn heal_workflow(
        &self,
        failed_executions: &[StepExecution],
    ) -> PlanResult<Vec<Step>> {
        let mut healed_steps = Vec::new();

        for execution in failed_executions {
            if let crate::ExecutionResult::Error { message, .. } = &execution.result {
                match self.heal_step(&execution.step, message).await {
                    Ok(healed_step) => {
                        healed_steps.push(healed_step);
                    }
                    Err(e) => {
                        warn!("Failed to heal step {}: {}", execution.step.id, e);
                        // Continue with other steps
                    }
                }
            }
        }

        if healed_steps.is_empty() {
            return Err(PlanError::HealingFailed {
                attempts: self.max_attempts,
            });
        }

        Ok(healed_steps)
    }
}
