use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PlannerMode {
    Bootstrap, // Initial state, understanding user intent
    Search,    // Searching for information
    Results,   // Processing search results
    Form,      // Filling out forms
    Browse,    // General browsing
    Complete,  // Task completed
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannerState {
    pub mode: PlannerMode,
    pub context: HashMap<String, String>,
    pub correlation_id: String,
    pub step_count: usize,
    pub max_steps: usize,
}

impl PlannerState {
    pub fn new(correlation_id: String) -> Self {
        Self {
            mode: PlannerMode::Bootstrap,
            context: HashMap::new(),
            correlation_id,
            step_count: 0,
            max_steps: 20,
        }
    }

    pub fn transition(&mut self, new_mode: PlannerMode) {
        log::info!(
            "[{}] State transition: {:?} -> {:?}",
            self.correlation_id,
            self.mode,
            new_mode
        );
        self.mode = new_mode;
        self.step_count += 1;
    }

    pub fn is_complete(&self) -> bool {
        self.mode == PlannerMode::Complete || self.step_count >= self.max_steps
    }

    pub fn get_system_prompt(&self) -> &'static str {
        match self.mode {
            PlannerMode::Bootstrap => {
                "You are starting a new task. Analyze what needs to be done and navigate to the appropriate website."
            }
            PlannerMode::Search => {
                "You are performing a search. Type in the search box and press Enter. NEVER construct search URLs."
            }
            PlannerMode::Results => {
                "Search results are displayed. Extract the relevant information or click on a result."
            }
            PlannerMode::Form => {
                "You are filling out a form. Complete all required fields and submit."
            }
            PlannerMode::Browse => {
                "You are browsing a website. Navigate to find the information you need."
            }
            PlannerMode::Complete => {
                "Task is complete. Summarize what was accomplished."
            }
        }
    }

    pub fn get_allowed_tools(&self) -> Vec<&'static str> {
        // Safety tools are allowed in all active modes.
        // These are deterministic and help recover from common interstitials
        // (cookie banners, login modals, simple captchas).
        const SAFETY_TOOLS: &[&str] = &[
            "detect_popups",
            "dismiss_popups",
            "wait_for_no_popups",
            "handle_captcha",
            "request_user_intervention",
        ];

        match self.mode {
            // Allow typing/pressing in Bootstrap too when starting on a search page
            // Bootstrap: start from a blank tab. Only navigate or brief wait are allowed.
            PlannerMode::Bootstrap => {
                let mut v = vec!["navigate", "wait", "complete"];
                v.extend_from_slice(SAFETY_TOOLS);
                v
            }
            PlannerMode::Search => {
                let mut v = vec![
                    "type",
                    "press_key",
                    "press",
                    "type_and_submit",
                    "wait",
                    "complete",
                ];
                v.extend_from_slice(SAFETY_TOOLS);
                v
            }
            // Results: primarily extract/click/scroll. Allow type_and_submit only for search box refinement.
            PlannerMode::Results => {
                let mut v = vec![
                    "extract",
                    "extract_auto_list",
                    "click",
                    "scroll",
                    "wait",
                    "type_and_submit",
                    "complete",
                ];
                v.extend_from_slice(SAFETY_TOOLS);
                v
            }
            PlannerMode::Form => {
                let mut v = vec![
                    "type",
                    "click",
                    "press_key",
                    "type_and_submit",
                    "batch_actions",
                    "wait",
                    "complete",
                ];
                v.extend_from_slice(SAFETY_TOOLS);
                v
            }
            PlannerMode::Browse => {
                let mut v = vec![
                    "click",
                    "scroll",
                    "extract",
                    "extract_auto_list",
                    "navigate",
                    "batch_actions",
                    "wait",
                    "complete",
                ];
                v.extend_from_slice(SAFETY_TOOLS);
                v
            }
            PlannerMode::Complete => vec!["complete"],
        }
    }

    pub fn update_context(&mut self, key: String, value: String) {
        self.context.insert(key, value);
    }

    pub fn infer_next_mode(&mut self, current_url: &str, dom_summary: &str) -> PlannerMode {
        // Parse URL for robust host/path checks
        let parsed = url::Url::parse(current_url);
        let url_lower = current_url.to_lowercase();
        let (_host, path) = match parsed {
            Ok(u) => (
                u.host_str().unwrap_or("").to_lowercase(),
                u.path().to_lowercase(),
            ),
            Err(_) => ("".to_string(), url_lower.clone()),
        };

        // Search results pages
        if path.contains("/search")
            || url_lower.contains("?q=")
            || url_lower.contains("&q=")
            || path.contains("/results")
            || dom_summary.contains("search results")
        {
            return PlannerMode::Results;
        }

        // Also check generic landing pages with search affordances
        if (path == "/" || path.contains("/home") || path.contains("/index"))
            && (dom_summary.contains("search") || dom_summary.contains("query"))
        {
            return PlannerMode::Search;
        }

        // Forms
        if dom_summary.contains("form")
            || dom_summary.contains("input")
            || dom_summary.contains("login")
            || dom_summary.contains("sign")
        {
            return PlannerMode::Form;
        }

        // Default
        PlannerMode::Browse
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_transitions() {
        let mut state = PlannerState::new("test-123".to_string());
        assert_eq!(state.mode, PlannerMode::Bootstrap);

        state.transition(PlannerMode::Search);
        assert_eq!(state.mode, PlannerMode::Search);
        assert_eq!(state.step_count, 1);
    }

    #[test]
    fn test_allowed_tools() {
        let state = PlannerState::new("test-123".to_string());
        let tools = state.get_allowed_tools();
        assert!(tools.contains(&"navigate"));
        assert!(!tools.contains(&"type"));
    }

    #[test]
    fn test_mode_inference() {
        let mut state = PlannerState::new("test-123".to_string());

        let next = state.infer_next_mode("https://example.com", "search box visible");
        assert_eq!(next, PlannerMode::Search);

        let next = state.infer_next_mode("https://example.com/search?q=test", "results shown");
        assert_eq!(next, PlannerMode::Results);
    }
}
