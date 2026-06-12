use super::planner_fsm::{PlannerMode, PlannerState};
use serde_json::Value;
use url::Url;

pub struct PolicyValidator {
    correlation_id: String,
}

impl PolicyValidator {
    pub fn new(correlation_id: String) -> Self {
        Self { correlation_id }
    }

    pub fn validate_action(&self, action: &Value, state: &PlannerState) -> Result<(), String> {
        let cmd = action
            .get("cmd")
            .and_then(|c| c.as_str())
            .ok_or("Missing cmd field")?;

        // Allow explicit error reports from the LLM in any mode.
        if cmd == "error" {
            return Ok(());
        }

        // Check if tool is allowed in current mode
        let allowed_tools = state.get_allowed_tools();
        if !allowed_tools.contains(&cmd) {
            return Err(format!(
                "Tool '{}' not allowed in mode {:?}. Allowed: {:?}",
                cmd, state.mode, allowed_tools
            ));
        }

        // Specific validation rules
        match cmd {
            "navigate" => self.validate_navigate(action, state),
            "type" => self.validate_type(action, state),
            "press_key" => self.validate_press_key(action, state),
            "type_and_submit" => self.validate_type_and_submit(action, state),
            "click" => self.validate_click(action, state),
            "batch_actions" => self.validate_batch_actions(action, state),
            _ => Ok(()),
        }
    }

    fn validate_navigate(&self, action: &Value, state: &PlannerState) -> Result<(), String> {
        let args = action
            .get("args")
            .and_then(|a| a.as_array())
            .ok_or("Navigate requires args array")?;

        if args.is_empty() {
            return Err("Navigate requires URL argument".to_string());
        }

        let url = args[0].as_str().unwrap_or("");

        // Check for UI-required sites that should never use URL construction
        if let Some(violation) = self.check_ui_required_site(url, state) {
            log::error!(
                "[{}] BLOCKED: URL construction on UI-required site: {}",
                self.correlation_id,
                url
            );
            return Err(violation);
        }

        Ok(())
    }

    fn validate_click(&self, _action: &Value, state: &PlannerState) -> Result<(), String> {
        // For extraction-focused tasks on a results page, prefer extraction before navigation
        if state.mode == PlannerMode::Results {
            if let Some(pref) = state.context.get("prefer_extract_first") {
                if pref == "true" {
                    let extracted =
                        state.context.get("extracted").map(|v| v.as_str()) == Some("true");
                    let ax_attempted =
                        state.context.get("ax_attempted").map(|v| v.as_str()) == Some("true");
                    let extract_attempted =
                        state.context.get("extract_attempted").map(|v| v.as_str()) == Some("true");
                    if !extracted && !ax_attempted && !extract_attempted {
                        return Err("Click temporarily disallowed: extract results first (prefer_extract_first)".to_string());
                    }
                }
            }
        }
        Ok(())
    }

    /// Check if URL attempts to bypass UI on sites that require user interaction
    fn check_ui_required_site(&self, url: &str, state: &PlannerState) -> Option<String> {
        // Avoid domain-tuned rules. Prefer generic signals that a URL is trying to
        // bypass the UI (constructed search/result URLs, deep links into sensitive flows).
        let enforce = matches!(
            state.mode,
            PlannerMode::Bootstrap | PlannerMode::Search | PlannerMode::Form | PlannerMode::Results
        );
        if !enforce {
            return None;
        }

        let url_lower = url.to_lowercase();
        let parsed = Url::parse(url).ok();
        let (host, path, query_pairs_len, has_fragment) = if let Some(u) = &parsed {
            let host = u.host_str().unwrap_or("").to_lowercase();
            let path = u.path().to_lowercase();
            let qp_len = u.query_pairs().count();
            (host, path, qp_len, u.fragment().is_some())
        } else {
            (
                "".to_string(),
                url_lower.clone(),
                0,
                url_lower.contains('#'),
            )
        };

        // Search URL heuristic: common query keys + a results-ish path or root path.
        let mut has_search_key = false;
        if let Some(u) = &parsed {
            for (k, v) in u.query_pairs() {
                if v.is_empty() {
                    continue;
                }
                match k.to_lowercase().as_ref() {
                    "q" | "query" | "search" | "term" | "k" | "keyword" | "keywords"
                    | "search_query" => {
                        has_search_key = true;
                        break;
                    }
                    _ => {}
                }
            }
        } else {
            has_search_key = url_lower.contains("?q=")
                || url_lower.contains("&q=")
                || url_lower.contains("?query=")
                || url_lower.contains("&query=");
        }

        let looks_like_results_path =
            path.contains("/search") || path.contains("/results") || path.contains("/find");
        let looks_like_root_search = path == "/" && has_search_key;

        if has_search_key
            && (looks_like_results_path || looks_like_root_search || query_pairs_len > 3)
        {
            return Some(
                "POLICY VIOLATION: Do not construct search/result URLs. Navigate to the site and use the visible search UI."
                    .to_string(),
            );
        }

        // Sensitive flows: discourage deep-linking with parameters/fragments when the URL
        // suggests auth/checkout/payment/banking semantics.
        let sensitive_hint = host.contains("bank")
            || host.contains("finance")
            || path.contains("login")
            || path.contains("signin")
            || path.contains("checkout")
            || path.contains("payment")
            || path.contains("/pay");
        if sensitive_hint && (query_pairs_len > 0 || has_fragment) {
            return Some(
                "POLICY VIOLATION: Sensitive flows should be driven via the site's UI (navigate to the entry page and use forms)."
                    .to_string(),
            );
        }

        // Generic patterns that suggest form bypassing
        if state.mode == PlannerMode::Search || state.mode == PlannerMode::Form {
            // Multiple query parameters often indicate form bypassing
            let query_param_count = url.matches('&').count();
            if query_param_count > 3 && (url_lower.contains("?q=") || url_lower.contains("&q=")) {
                return Some("POLICY VIOLATION: Complex search URLs suggest form bypassing. Use the site's search interface.".to_string());
            }

            // Common form bypass patterns
            let bypass_patterns = [
                "?search=", "&search=", "?query=", "&query=", "?term=", "&term=",
            ];
            for pattern in bypass_patterns {
                if url_lower.contains(pattern) {
                    return Some("POLICY VIOLATION: Use the site's search form instead of constructing search URLs.".to_string());
                }
            }
        }

        None
    }

    fn validate_type(&self, action: &Value, state: &PlannerState) -> Result<(), String> {
        let args = action
            .get("args")
            .and_then(|a| a.as_array())
            .ok_or("Type requires args array")?;

        if args.len() < 2 {
            return Err("Type requires selector and text arguments".to_string());
        }

        // In Search mode, typing should be followed by press_key
        if state.mode == PlannerMode::Search {
            log::info!(
                "[{}] Type action in Search mode - should be followed by press_key",
                self.correlation_id
            );
        }

        Ok(())
    }

    fn validate_press_key(&self, action: &Value, _state: &PlannerState) -> Result<(), String> {
        let args = action
            .get("args")
            .and_then(|a| a.as_array())
            .ok_or("press_key requires args array")?;

        if args.is_empty() {
            return Err("press_key requires key argument".to_string());
        }

        let key = args[0].as_str().unwrap_or("");
        let valid_keys = [
            "Enter",
            "Tab",
            "Escape",
            "ArrowUp",
            "ArrowDown",
            "ArrowLeft",
            "ArrowRight",
        ];

        if !valid_keys.contains(&key) {
            return Err(format!(
                "Invalid key: {}. Valid keys: {:?}",
                key, valid_keys
            ));
        }

        Ok(())
    }

    fn validate_type_and_submit(&self, action: &Value, state: &PlannerState) -> Result<(), String> {
        let args = action
            .get("args")
            .and_then(|a| a.as_array())
            .ok_or("type_and_submit requires args array")?;

        if args.len() < 2 {
            return Err("type_and_submit requires selector and text arguments".to_string());
        }

        let selector = args[0].as_str().unwrap_or("");
        let text = args[1].as_str().unwrap_or("");

        // Basic validation
        if selector.is_empty() {
            return Err("type_and_submit requires non-empty selector".to_string());
        }
        if text.is_empty() {
            return Err("type_and_submit requires non-empty text".to_string());
        }

        // Validate this is appropriate for the current mode
        match state.mode {
            PlannerMode::Search => {
                log::info!(
                    "[{}] type_and_submit in Search mode - efficient search pattern",
                    self.correlation_id
                );
                // In Search mode, this should typically target search boxes
                if !selector.contains("search")
                    && !selector.contains("q")
                    && !selector.contains("input")
                {
                    log::warn!(
                        "[{}] type_and_submit selector '{}' doesn't look like search input",
                        self.correlation_id,
                        selector
                    );
                }
            }
            PlannerMode::Form => {
                log::info!(
                    "[{}] type_and_submit in Form mode - form submission pattern",
                    self.correlation_id
                );
            }
            PlannerMode::Results => {
                // Allow refinement in results ONLY if targeting the visible search box
                let sel_lower = selector.to_lowercase();
                let looks_like_search = sel_lower.contains("name='q'")
                    || sel_lower.contains("name=\"q\"")
                    || sel_lower.contains("textarea")
                    || sel_lower.contains("type='search'")
                    || sel_lower.contains("type=\"search\"")
                    || sel_lower.contains("search");

                if looks_like_search {
                    log::info!(
                        "[{}] type_and_submit permitted in Results mode for search box refinement (selector: '{}')",
                        self.correlation_id,
                        selector
                    );
                } else {
                    return Err(format!(
                        "type_and_submit not allowed in mode {:?} for selector '{}'. Use extract/click/scroll/wait or target the search box.",
                        state.mode,
                        selector
                    ));
                }
            }
            _ => {
                return Err(format!(
                    "type_and_submit not allowed in mode {:?}. Use in Search or Form mode.",
                    state.mode
                ));
            }
        }

        Ok(())
    }

    fn validate_batch_actions(&self, action: &Value, state: &PlannerState) -> Result<(), String> {
        let steps = action
            .get("steps")
            .and_then(|v| v.as_array())
            .ok_or("batch_actions missing steps array")?;

        if steps.is_empty() || steps.len() > 12 {
            return Err("batch_actions must have 1..12 steps".to_string());
        }

        // Validate this is appropriate for the current mode
        match state.mode {
            PlannerMode::Form | PlannerMode::Browse => {
                log::info!(
                    "[{}] batch_actions in {:?} mode - efficient multi-step pattern",
                    self.correlation_id,
                    state.mode
                );
            }
            _ => {
                return Err(format!(
                    "batch_actions not allowed in mode {:?}. Use in Form or Browse mode.",
                    state.mode
                ));
            }
        }

        // Validate each step
        for (i, step) in steps.iter().enumerate() {
            let op = step
                .get("op")
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("batch_actions step {} missing op field", i))?;

            // Only allow safe atomic operations (no navigation)
            match op {
                "click" | "insert_text" | "press_key" | "wait_selector" | "scroll_by" => {}
                _ => {
                    return Err(format!(
                        "batch_actions step {} has invalid op '{}'. Allowed: click, insert_text, press_key, wait_selector, scroll_by",
                        i, op
                    ));
                }
            }

            // Validate step has required fields for its operation
            match op {
                "click" => {
                    if step.get("selector").is_none() && step.get("encodedId").is_none() {
                        return Err(format!(
                            "batch_actions step {} (click) requires selector or encodedId",
                            i
                        ));
                    }
                }
                "insert_text" => {
                    if step.get("selector").is_none() && step.get("encodedId").is_none() {
                        return Err(format!(
                            "batch_actions step {} (insert_text) requires selector or encodedId",
                            i
                        ));
                    }
                    if step.get("text").is_none() {
                        return Err(format!(
                            "batch_actions step {} (insert_text) requires text field",
                            i
                        ));
                    }
                }
                "press_key" => {
                    if step.get("key").is_none() {
                        return Err(format!(
                            "batch_actions step {} (press_key) requires key field",
                            i
                        ));
                    }
                }
                "wait_selector" => {
                    if step.get("waitSelector").is_none() {
                        return Err(format!(
                            "batch_actions step {} (wait_selector) requires waitSelector field",
                            i
                        ));
                    }
                }
                "scroll_by" => {
                    // dx and dy are optional, defaults will be used
                }
                _ => unreachable!(),
            }
        }

        log::info!(
            "[{}] Validated batch_actions with {} steps",
            self.correlation_id,
            steps.len()
        );

        Ok(())
    }

    pub fn validate_batch(&self, actions: &[Value], state: &PlannerState) -> Result<(), String> {
        // Validate each action individually first
        for (i, action) in actions.iter().enumerate() {
            self.validate_action(action, state)
                .map_err(|e| format!("Action {}: {}", i, e))?;
        }

        // Special validation for type + press_key combo (only if 2 actions)
        if actions.len() == 2 {
            if let (Some("type"), Some("press_key")) = (
                actions[0].get("cmd").and_then(|c| c.as_str()),
                actions[1].get("cmd").and_then(|c| c.as_str()),
            ) {
                // Verify it's Enter key for search
                if let Some(key) = actions[1]
                    .get("args")
                    .and_then(|a| a.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|k| k.as_str())
                {
                    if key == "Enter" {
                        log::info!(
                            "[{}] Valid search pattern detected: type + press_key Enter",
                            self.correlation_id
                        );
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_block_constructed_search_url() {
        let validator = PolicyValidator::new("test-123".to_string());
        let state = PlannerState::new("test-123".to_string());

        let action = json!({
            "cmd": "navigate",
            "args": ["https://example.com/search?q=test"]
        });

        let result = validator.validate_action(&action, &state);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("POLICY VIOLATION"));
    }

    #[test]
    fn test_allow_homepage() {
        let validator = PolicyValidator::new("test-123".to_string());
        let state = PlannerState::new("test-123".to_string());

        let action = json!({
            "cmd": "navigate",
            "args": ["https://example.com"]
        });

        let result = validator.validate_action(&action, &state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_type_press_combo() {
        let validator = PolicyValidator::new("test-123".to_string());
        let mut state = PlannerState::new("test-123".to_string());
        state.transition(PlannerMode::Search);

        let actions = vec![
            json!({"cmd": "type", "args": ["input[name='q']", "test query"]}),
            json!({"cmd": "press_key", "args": ["Enter"]}),
        ];

        let result = validator.validate_batch(&actions, &state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_block_amazon_search_url() {
        let validator = PolicyValidator::new("test-123".to_string());
        let state = PlannerState::new("test-123".to_string());

        let action = json!({
            "cmd": "navigate",
            "args": ["https://example.com/search?k=laptop"]
        });

        let result = validator.validate_action(&action, &state);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("POLICY VIOLATION"));
    }

    #[test]
    fn test_block_financial_site_params() {
        let validator = PolicyValidator::new("test-123".to_string());
        let state = PlannerState::new("test-123".to_string());

        let action = json!({
            "cmd": "navigate",
            "args": ["https://bank.example.com/accounts?type=checking"]
        });

        let result = validator.validate_action(&action, &state);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Sensitive flows"));
    }

    #[test]
    fn test_block_social_search_url() {
        let validator = PolicyValidator::new("test-123".to_string());
        let state = PlannerState::new("test-123".to_string());

        let action = json!({
            "cmd": "navigate",
            "args": ["https://example.com/results?search_query=tutorial"]
        });

        let result = validator.validate_action(&action, &state);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("POLICY VIOLATION"));
    }

    #[test]
    fn test_allow_homepage_navigation() {
        let validator = PolicyValidator::new("test-123".to_string());
        let state = PlannerState::new("test-123".to_string());

        let action = json!({
            "cmd": "navigate",
            "args": ["https://example.com"]
        });

        let result = validator.validate_action(&action, &state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_block_complex_search_patterns() {
        let validator = PolicyValidator::new("test-123".to_string());
        let mut state = PlannerState::new("test-123".to_string());
        state.transition(PlannerMode::Search);

        let action = json!({
            "cmd": "navigate",
            "args": ["https://example.com/search?q=test&category=all&sort=price&filter=new"]
        });

        let result = validator.validate_action(&action, &state);
        // In Search mode, 'navigate' is not an allowed tool, so the policy should
        // block the action before URL content checks.
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not allowed in mode"));
    }

    #[test]
    fn test_validate_type_and_submit_search_mode() {
        let validator = PolicyValidator::new("test-123".to_string());
        let mut state = PlannerState::new("test-123".to_string());
        state.transition(PlannerMode::Search);

        let action = json!({
            "cmd": "type_and_submit",
            "args": ["input[name='q']", "test query"]
        });

        let result = validator.validate_action(&action, &state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_type_and_submit_form_mode() {
        let validator = PolicyValidator::new("test-123".to_string());
        let mut state = PlannerState::new("test-123".to_string());
        state.transition(PlannerMode::Form);

        let action = json!({
            "cmd": "type_and_submit",
            "args": ["textarea[name='comment']", "This is my comment"]
        });

        let result = validator.validate_action(&action, &state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_block_type_and_submit_wrong_mode() {
        let validator = PolicyValidator::new("test-123".to_string());
        let mut state = PlannerState::new("test-123".to_string());
        state.transition(PlannerMode::Results);

        let action = json!({
            "cmd": "type_and_submit",
            "args": ["input", "should not work"]
        });

        let result = validator.validate_action(&action, &state);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not allowed in mode"));
    }

    #[test]
    fn test_validate_batch_actions_form_mode() {
        let validator = PolicyValidator::new("test-123".to_string());
        let mut state = PlannerState::new("test-123".to_string());
        state.transition(PlannerMode::Form);

        let action = json!({
            "cmd": "batch_actions",
            "steps": [
                {"op": "click", "selector": "input[name='email']"},
                {"op": "insert_text", "selector": "input[name='email']", "text": "test@example.com"},
                {"op": "press_key", "key": "Tab"},
                {"op": "insert_text", "selector": "input[name='password']", "text": "password123"}
            ]
        });

        let result = validator.validate_action(&action, &state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_batch_actions_browse_mode() {
        let validator = PolicyValidator::new("test-123".to_string());
        let mut state = PlannerState::new("test-123".to_string());
        state.transition(PlannerMode::Browse);

        let action = json!({
            "cmd": "batch_actions",
            "steps": [
                {"op": "scroll_by", "dy": 300},
                {"op": "wait_selector", "waitSelector": ".load-more"},
                {"op": "click", "selector": ".load-more"}
            ]
        });

        let result = validator.validate_action(&action, &state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_block_batch_actions_wrong_mode() {
        let validator = PolicyValidator::new("test-123".to_string());
        let mut state = PlannerState::new("test-123".to_string());
        state.transition(PlannerMode::Search);

        let action = json!({
            "cmd": "batch_actions",
            "steps": [{"op": "click", "selector": "button"}]
        });

        let result = validator.validate_action(&action, &state);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not allowed in mode"));
    }

    #[test]
    fn test_batch_actions_invalid_op() {
        let validator = PolicyValidator::new("test-123".to_string());
        let mut state = PlannerState::new("test-123".to_string());
        state.transition(PlannerMode::Form);

        let action = json!({
            "cmd": "batch_actions",
            "steps": [{"op": "navigate", "url": "https://example.com"}]
        });

        let result = validator.validate_action(&action, &state);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid op 'navigate'"));
    }
}
