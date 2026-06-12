use crate::PlanResult;
use log::{debug, info, warn};
use rzn_core::{Step, StepKind};

/// Sanitizes planned steps to remove problematic selectors before execution
pub struct PlanSanitizer;

impl PlanSanitizer {
    pub fn new() -> Self {
        Self
    }

    /// Sanitize a single step, returning None if the step should be dropped
    pub fn sanitize_step(&self, step: &Step) -> PlanResult<Option<Step>> {
        match &step.kind {
            StepKind::NavigateToUrl { url, wait } => {
                // Validate navigate_to_url has a valid URL
                if url.trim().is_empty() {
                    warn!(
                        "🚫 Plan sanitizer: Blocking navigate_to_url with empty URL - {}",
                        step.name
                    );
                    info!("   [TIP] Reason: navigate_to_url requires a valid URL");
                    return Ok(None);
                }

                // Validate URL format
                if !url.starts_with("http://") && !url.starts_with("https://") && !url.contains(".")
                {
                    warn!("🚫 Plan sanitizer: Blocking navigate_to_url with invalid URL format - {} ({})", step.name, url);
                    info!("   [TIP] Reason: URL must be valid (e.g., https://example.com)");
                    return Ok(None);
                }

                // Fix common URL issues
                let fixed_url = if !url.starts_with("http://") && !url.starts_with("https://") {
                    format!("https://{}", url)
                } else {
                    url.clone()
                };

                if fixed_url != *url {
                    info!(
                        " Plan sanitizer: Fixed URL format - {} -> {}",
                        url, fixed_url
                    );
                    let mut sanitized_step = step.clone();
                    sanitized_step.kind = StepKind::NavigateToUrl {
                        url: fixed_url,
                        wait: wait.clone().or(Some("domcontentloaded".to_string())),
                    };
                    return Ok(Some(sanitized_step));
                }
            }
            StepKind::ExtractStructuredData {
                item_selector,
                limit,
                fields,
                frame_id,
                extraction_type,
            } => {
                // Block any extraction targeting IFRAME elements
                if item_selector.to_uppercase().starts_with("IFRAME")
                    || item_selector.to_lowercase().contains("iframe")
                    || item_selector.to_lowercase().contains("frame")
                {
                    warn!(
                        "🚫 Plan sanitizer: Blocking IFRAME extraction step - {} ({})",
                        step.name, item_selector
                    );
                    info!("   [TIP] Reason: IFRAME elements are cross-origin and inaccessible");

                    // Replace with a safer alternative if possible
                    if let Some(alternative) = self.suggest_iframe_alternative(item_selector) {
                        info!("    Replacing with alternative selector: {}", alternative);
                        let mut sanitized_step = step.clone();
                        sanitized_step.kind = StepKind::ExtractStructuredData {
                            item_selector: alternative,
                            limit: *limit,
                            fields: fields.clone(),
                            frame_id: frame_id.clone(),
                            extraction_type: extraction_type.clone(),
                        };
                        sanitized_step.name = format!("{}  (sanitized)", step.name);
                        return Ok(Some(sanitized_step));
                    } else {
                        // Drop the step entirely
                        return Ok(None);
                    }
                }

                // Check fields for IFRAME selectors too
                let mut sanitized_fields = fields.clone();
                let mut has_iframe_fields = false;

                for field in &mut sanitized_fields {
                    if field.selector.to_uppercase().starts_with("IFRAME")
                        || field.selector.to_lowercase().contains("iframe")
                        || field.selector.to_lowercase().contains("frame")
                    {
                        warn!(
                            "🚫 Plan sanitizer: Blocking IFRAME field selector - {} ({})",
                            field.name, field.selector
                        );
                        has_iframe_fields = true;

                        // Try to suggest alternative
                        if let Some(alternative) = self.suggest_iframe_alternative(&field.selector)
                        {
                            info!(
                                "    Replacing field '{}' selector with: {}",
                                field.name, alternative
                            );
                            field.selector = alternative;
                            has_iframe_fields = false; // We fixed it
                        }
                    }
                }

                if has_iframe_fields {
                    warn!("🚫 Plan sanitizer: Dropping extraction step due to unfixable IFRAME field selectors");
                    return Ok(None);
                }

                // Return sanitized version if we made changes
                if sanitized_fields != *fields {
                    let mut sanitized_step = step.clone();
                    sanitized_step.kind = StepKind::ExtractStructuredData {
                        item_selector: item_selector.clone(),
                        limit: *limit,
                        fields: sanitized_fields,
                        frame_id: frame_id.clone(),
                        extraction_type: extraction_type.clone(),
                    };
                    sanitized_step.name = format!("{}  (sanitized)", step.name);
                    return Ok(Some(sanitized_step));
                }
            }

            StepKind::ClickElement { selector, .. } => {
                // Block clicking on IFRAME elements
                if selector.to_uppercase().starts_with("IFRAME")
                    || selector.to_lowercase().contains("iframe")
                    || selector.to_lowercase().contains("frame")
                {
                    warn!(
                        "🚫 Plan sanitizer: Blocking IFRAME click step - {} ({})",
                        step.name, selector
                    );
                    return Ok(None);
                }
            }

            StepKind::FillInputField { selector, .. } => {
                // Block filling inputs inside IFRAME elements
                if selector.to_uppercase().starts_with("IFRAME")
                    || selector.to_lowercase().contains("iframe")
                    || selector.to_lowercase().contains("frame")
                {
                    warn!(
                        "🚫 Plan sanitizer: Blocking IFRAME input step - {} ({})",
                        step.name, selector
                    );
                    return Ok(None);
                }
            }

            StepKind::WaitForElement { selector, .. } => {
                // Block waiting for IFRAME elements
                if selector.to_uppercase().starts_with("IFRAME")
                    || selector.to_lowercase().contains("iframe")
                    || selector.to_lowercase().contains("frame")
                {
                    warn!(
                        "🚫 Plan sanitizer: Blocking IFRAME wait step - {} ({})",
                        step.name, selector
                    );
                    return Ok(None);
                }
            }

            _ => {
                // Other step types are fine
            }
        }

        // Step is clean, return as-is
        Ok(Some(step.clone()))
    }

    /// Sanitize a list of steps, filtering out problematic ones
    pub fn sanitize_steps(&self, steps: &[Step]) -> PlanResult<Vec<Step>> {
        let mut sanitized_steps = Vec::new();
        let mut dropped_count = 0;

        for step in steps {
            match self.sanitize_step(step)? {
                Some(sanitized_step) => {
                    debug!(
                        "[OK] Plan sanitizer: Step '{}' passed sanitization",
                        step.name
                    );
                    sanitized_steps.push(sanitized_step);
                }
                None => {
                    dropped_count += 1;
                    info!("  Plan sanitizer: Dropped problematic step - {}", step.name);
                }
            }
        }

        if dropped_count > 0 {
            info!(
                " Plan sanitizer: Dropped {} problematic steps, {} steps remain",
                dropped_count,
                sanitized_steps.len()
            );
        }

        Ok(sanitized_steps)
    }

    /// Suggest alternative selectors for common IFRAME patterns
    fn suggest_iframe_alternative(&self, iframe_selector: &str) -> Option<String> {
        let looks_like_iframe = iframe_selector.to_lowercase().contains("iframe")
            || iframe_selector.to_lowercase().contains("frame");
        if !looks_like_iframe {
            return None;
        }

        // Generic best-effort fallback: prefer main content containers.
        Some("main, [role=\"main\"], #main, .main-content, .content, body".to_string())
    }
}

impl Default for PlanSanitizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iframe_extraction_blocking() {
        let sanitizer = PlanSanitizer::new();

        let iframe_step = Step {
            id: "test_extract".to_string(),
            name: "Extract from iframe".to_string(),
            kind: StepKind::ExtractStructuredData {
                item_selector: "IFRAME .result".to_string(),
                limit: None,
                fields: vec![],
                frame_id: None,
                extraction_type: None,
            },
        };

        let result = sanitizer.sanitize_step(&iframe_step).unwrap();
        // Should return None (step dropped) or Some with alternative selector
        if let Some(sanitized) = result {
            if let StepKind::ExtractStructuredData { item_selector, .. } = &sanitized.kind {
                assert!(!item_selector.to_uppercase().contains("IFRAME"));
            }
        }
    }

    #[test]
    fn test_safe_step_passthrough() {
        let sanitizer = PlanSanitizer::new();

        let safe_step = Step {
            id: "test_extract".to_string(),
            name: "Extract from div".to_string(),
            kind: StepKind::ExtractStructuredData {
                item_selector: ".search-results .result".to_string(),
                limit: None,
                fields: vec![],
                frame_id: None,
                extraction_type: None,
            },
        };

        let result = sanitizer.sanitize_step(&safe_step).unwrap();
        assert!(result.is_some());

        let sanitized = result.unwrap();
        assert_eq!(sanitized.id, safe_step.id);
    }

    #[test]
    fn test_iframe_alternative_suggestions() {
        let sanitizer = PlanSanitizer::new();

        assert_eq!(
            sanitizer.suggest_iframe_alternative("iframe#foo"),
            Some("main, [role=\"main\"], #main, .main-content, .content, body".to_string())
        );

        assert_eq!(
            sanitizer.suggest_iframe_alternative("iframe[src*=\"sport\"]"),
            Some("main, [role=\"main\"], #main, .main-content, .content, body".to_string())
        );

        assert_eq!(sanitizer.suggest_iframe_alternative(".safe-selector"), None);
    }
}
