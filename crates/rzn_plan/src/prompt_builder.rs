use crate::broker_client::DomSnapshot;
use crate::dom_whitelist::DomWhitelist;
use crate::failure_cache::FailureCache;
use crate::security_prompts::{
    self, wrap_untrusted_content, wrap_user_request, COMMON_SECURITY_RULES,
};
use crate::{DomContext, ExecutionResult, StepExecution};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Compact snapshot element for LLM consumption
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompactElement {
    /// Encoded ID for element selection (e.g., "btn_1", "inp_2")
    encoded_id: String,

    /// Element tag name
    tag: String,

    /// Primary selector for element
    selector: String,

    /// Text content (truncated)
    text: Option<String>,

    /// Element role from AX tree
    role: Option<String>,

    /// Element name from AX tree
    name: Option<String>,

    /// Distance from viewport center (for sorting)
    viewport_distance: f32,

    /// Key attributes
    attrs: Option<std::collections::HashMap<String, String>>,

    /// Frame information for cross-origin awareness
    frame: Option<String>,

    /// Element type (button, input, link, etc)
    element_type: Option<String>,

    /// Interaction hints
    actions: Option<Vec<String>>,
}

/// Compact frame context
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompactFrame {
    id: String,
    url: String,
    origin: String,
    accessible: bool,
    element_count: usize,
}

/// Compact snapshot for LLM consumption (2-8KB)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompactSnapshot {
    url: String,
    title: String,
    viewport: CompactViewport,
    elements: Vec<CompactElement>,
    frames: Vec<CompactFrame>,
    size_kb: f32,
    timestamp: u64,
    compression_level: String,
    memory_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompactViewport {
    width: u32,
    height: u32,
    scroll_x: u32,
    scroll_y: u32,
}

/// Builds prompts for LLM planning sessions
pub struct PromptBuilder {
    enable_family_handshake: bool,
    max_untrusted_chars: usize,
}

impl PromptBuilder {
    pub fn new() -> Self {
        let max_untrusted_chars = std::env::var("RZN_MAX_UNTRUSTED_CHARS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v >= 1_000)
            .unwrap_or(30_000);

        Self {
            enable_family_handshake: true,
            max_untrusted_chars,
        }
    }

    pub fn with_family_handshake(enable: bool) -> Self {
        let mut out = Self::new();
        out.enable_family_handshake = enable;
        out
    }

    /// Build planning prompt messages for LLM
    pub fn build_planning_prompt(
        &self,
        goal: &str,
        current_dom: &str,
        current_url: &str,
        history: &[StepExecution],
    ) -> Vec<Value> {
        let mut messages = vec![json!({
            "role": "system",
            "content": self.build_dom_aware_system_prompt()
        })];

        // Add goal wrapped in security tags
        let mut user_content = format!("{}\n", wrap_user_request(&format!("Goal: {}", goal)));

        // Add current session state
        if current_url.is_empty() || current_url == "about:blank" {
            user_content.push_str(
                "Current State: No active tab - MUST start with navigate_to_url WITH A VALID URL\n",
            );
            user_content.push_str("IMPORTANT: You must provide a complete URL like 'https://google.com' for navigate_to_url\n");
        } else {
            user_content.push_str(&format!("Current URL: {}\n", current_url));
        }

        // Add sanitized HTML wrapped in security tags
        if !current_dom.is_empty() {
            let limited_dom = limit_string(current_dom, self.max_untrusted_chars);
            let was_truncated = limited_dom.len() < current_dom.len();

            // Check if DOM is too large
            let dom_size = limited_dom.len();
            let estimated_tokens = dom_size / 4;

            // Log warning if DOM is large
            if dom_size > 50_000 {
                eprintln!("[WARNING]  WARNING: DOM size is {} bytes (~{} tokens) - this may exceed LLM limits!", 
                    dom_size, estimated_tokens);
                eprintln!("   Consider implementing more aggressive DOM reduction");
            }

            // Check if this looks like raw HTML (contains script tags)
            if limited_dom.contains("<script") || limited_dom.contains("window.") {
                eprintln!(" ERROR: Raw HTML is being sent to LLM instead of reduced DOM!");
                eprintln!("   DOM contains <script> tags or JavaScript code");
                eprintln!("   This should be a structured outline, not raw HTML!");
            }

            user_content.push_str(&format!(
                "\nPAGE_HTML (sanitized, {} bytes{}, ~{} tokens):\n{}\n",
                dom_size,
                if was_truncated { " - truncated" } else { "" },
                estimated_tokens,
                wrap_untrusted_content(&limited_dom)
            ));
        }

        // Add execution history
        if !history.is_empty() {
            user_content.push_str("\nExecution history:\n");
            for (i, execution) in history.iter().enumerate() {
                let result_summary = match &execution.result {
                    ExecutionResult::Success {
                        payload: Some(data),
                    } => {
                        if let Some(array) = data.as_array() {
                            if array.is_empty() {
                                "[WARNING]  Success but extracted 0 items - selectors may be incorrect".to_string()
                            } else if array.len() > 50 {
                                format!("[WARNING]  SELECTOR TOO BROAD: extracted {} items (way too many!) - need more specific selector", array.len())
                            } else {
                                format!("[OK] Success - extracted {} items", array.len())
                            }
                        } else {
                            "[OK] Success".to_string()
                        }
                    }
                    ExecutionResult::Success { payload: None } => "[OK] Success".to_string(),
                    ExecutionResult::Error { message, .. } => format!("[ERROR] Error: {}", message),
                };

                user_content.push_str(&format!(
                    "{}. {} ({}): {}\n",
                    i + 1,
                    execution.step.name,
                    execution.step.id,
                    result_summary
                ));

                // Add generic guidance if extraction failed
                if execution.step.name.contains("extract")
                    || execution.step.name.contains("Extract")
                {
                    if let ExecutionResult::Success {
                        payload: Some(data),
                    } = &execution.result
                    {
                        if let Some(array) = data.as_array() {
                            if array.is_empty() {
                                user_content.push_str("   [WARNING]  EXTRACTION FAILED: 0 items extracted. The selectors are likely incorrect.\n");
                                user_content.push_str("   [TIP] NEXT STEP: Analyze the DOM structure above and choose different selectors.\n");
                                user_content.push_str("    Look for 'GOOD FOR EXTRACTION' or 'EXCELLENT EXTRACTION TARGET' recommendations.\n");
                                user_content.push_str("   [TARGET] Use selectors with 5-25 matching elements, not hundreds.\n");
                            } else if array.len() > 50 {
                                user_content.push_str(&format!(
                                    "   [WARNING]  SELECTOR TOO BROAD: {} items is way too many!\n",
                                    array.len()
                                ));
                                user_content.push_str("   [TIP] NEXT STEP: Use a more specific selector. DO NOT navigate away!\n");
                                user_content.push_str("   [TARGET] For Google search results, try: #search .g, .yuRUbf, or [data-sokoban-container]\n");
                                user_content.push_str("    IMPORTANT: You are ALREADY on the search results page. DO NOT navigate back to google.com!\n");
                            }
                        }
                    }
                }
            }
            user_content.push_str("\n");
        }

        user_content.push_str("What should be the next step to achieve the goal?");

        messages.push(json!({
            "role": "user",
            "content": user_content
        }));

        messages
    }

    /// Build planning prompt with structured DOM context (preferred method)
    pub fn build_dom_aware_planning_prompt(
        &self,
        goal: &str,
        dom_context: Option<&DomContext>,
        history: &[StepExecution],
    ) -> Vec<Value> {
        self.build_dom_aware_planning_prompt_with_recovery(goal, dom_context, history, None)
    }

    /// Build planning prompt with structured DOM context and recovery info
    pub fn build_dom_aware_planning_prompt_with_recovery(
        &self,
        goal: &str,
        dom_context: Option<&DomContext>,
        history: &[StepExecution],
        failure_tracker: Option<&crate::failure_recovery::FailureTracker>,
    ) -> Vec<Value> {
        let mut messages = vec![json!({
            "role": "system",
            "content": self.build_dom_aware_system_prompt()
        })];

        // Estimate capacity: goal + context + history + overhead (optimizes allocations)
        let estimated_capacity = goal.len()
            + dom_context
                .map(|ctx| ctx.url.len() + ctx.title.len() + ctx.interactive_elements.len() * 100)
                .unwrap_or(0)
            + history.len() * 150
            + 2000; // overhead for formatting

        let mut user_content = String::with_capacity(estimated_capacity);

        // Use write! macro to avoid intermediate allocations
        use std::fmt::Write;
        let _ = write!(user_content, "Goal: {}\n", goal);

        // Add DOM context if available
        if let Some(context) = dom_context {
            let _ = write!(
                user_content,
                "Current URL: {}\nPage Type: {}\nPage Title: {}\n\n",
                context.url, context.page_type, context.title
            );

            // Add semantic groups summary
            if !context.semantic_groups.is_empty() {
                user_content.push_str("Available interactions:\n");
                for (group, selectors) in &context.semantic_groups {
                    let _ = write!(user_content, "- {}: {} elements\n", group, selectors.len());
                }
                user_content.push('\n');
            }

            // Add key interactive elements with recommendations
            user_content.push_str("Key interactive elements:\n");
            for (i, element) in context.interactive_elements.iter().take(15).enumerate() {
                let primary_selector = element.selector_suggestions.first().unwrap_or(&element.tag);

                // Add element description (pre-allocate for efficiency)
                let description = if !element.text_content.is_empty() {
                    let truncated: String = element.text_content.chars().take(40).collect();
                    format!(" - \"{}\"", truncated)
                } else if let Some(label) = element.attributes.get("aria-label") {
                    let truncated: String = label.chars().take(40).collect();
                    format!(" - [{}]", truncated)
                } else if let Some(placeholder) = element.attributes.get("placeholder") {
                    let truncated: String = placeholder.chars().take(40).collect();
                    format!(" - placeholder: \"{}\"", truncated)
                } else {
                    String::new()
                };

                // Add quality indicator
                let quality = self.assess_selector_quality(element);

                let _ = write!(
                    user_content,
                    "{}. {} {} {}{}\n",
                    i + 1,
                    element.tag.to_uppercase(),
                    primary_selector,
                    quality,
                    description
                );

                // Show alternative selectors for important elements
                if element.selector_suggestions.len() > 1 {
                    // Use iterator chain to avoid temporary Vec allocation
                    let alternatives = element.selector_suggestions.iter().skip(1).take(2);

                    user_content.push_str("   Alternatives: ");
                    let mut first = true;
                    for alt in alternatives {
                        if !first {
                            user_content.push_str(", ");
                        }
                        user_content.push_str(alt);
                        first = false;
                    }
                    if !first {
                        user_content.push('\n');
                    }
                }
            }

            if context.interactive_elements.len() > 15 {
                let _ = write!(
                    user_content,
                    "... and {} more elements\n",
                    context.interactive_elements.len() - 15
                );
            }
            user_content.push('\n');
        } else {
            user_content.push_str(
                "Current State: No active tab - MUST start with navigate_to_url WITH A VALID URL\n",
            );
            user_content.push_str("IMPORTANT: The navigate_to_url action requires a 'url' parameter with a complete URL like 'https://google.com'\n\n");
        }

        // Add execution history with enhanced feedback
        if !history.is_empty() {
            user_content.push_str("Execution history:\n");
            for (i, execution) in history.iter().enumerate() {
                // Use a closure to handle the string allocation more efficiently
                let write_result_summary = |content: &mut String, execution: &StepExecution| {
                    match &execution.result {
                        ExecutionResult::Success {
                            payload: Some(data),
                        } => {
                            if let Some(array) = data.as_array() {
                                if array.is_empty() {
                                    content.push_str("[WARNING]  SUCCESS BUT 0 ITEMS EXTRACTED - Wrong selectors!");
                                } else {
                                    let _ = write!(
                                        content,
                                        "[OK] Success - extracted {} items",
                                        array.len()
                                    );
                                }
                            } else {
                                content.push_str("[OK] Success");
                            }
                        }
                        ExecutionResult::Success { payload: None } => {
                            content.push_str("[OK] Success");
                        }
                        ExecutionResult::Error { message, .. } => {
                            let _ = write!(content, "[ERROR] FAILED: {}", message);
                        }
                    }
                };

                let _ = write!(user_content, "{}. {} ", i + 1, execution.step.name);
                write_result_summary(&mut user_content, execution);
                user_content.push('\n');

                // Enhanced guidance for failed extractions
                if execution.step.name.contains("extract")
                    || execution.step.name.contains("Extract")
                {
                    if let ExecutionResult::Success {
                        payload: Some(data),
                    } = &execution.result
                    {
                        if let Some(array) = data.as_array() {
                            if array.is_empty() {
                                user_content.push_str("   [SEARCH] SELECTOR ANALYSIS NEEDED: The extraction selector found 0 items.\n");
                                user_content.push_str("   [TIP] SOLUTION: Use the element list above to find the correct container and item selectors.\n");
                                user_content.push_str("   [TARGET] LOOK FOR: Elements marked with quality indicators above.\n");
                            }
                        }
                    }
                }

                //  CRITICAL FIX: Include inspection data for failed extractions
                if execution
                    .step
                    .name
                    .contains("Page Inspection for Failed Extraction")
                {
                    if let ExecutionResult::Success {
                        payload: Some(inspection_data),
                    } = &execution.result
                    {
                        user_content.push_str("    PAGE INSPECTION RESULTS:\n");

                        // Display inspection data in a structured way for LLM
                        if let Some(element_counts) = inspection_data.get("element_counts") {
                            user_content.push_str("   [LIST] Available elements on the page:\n");
                            if let Some(counts_obj) = element_counts.as_object() {
                                for (element_type, count) in counts_obj {
                                    let _ = write!(
                                        user_content,
                                        "      - {}: {} elements\n",
                                        element_type, count
                                    );
                                }
                            }
                        }

                        if let Some(discovered) = inspection_data.get("discovered") {
                            if let Some(discovered_array) = discovered.as_array() {
                                if !discovered_array.is_empty() {
                                    user_content.push_str("   [TARGET] DISCOVERED ELEMENTS (potential extraction targets):\n");
                                    for discovered_item in discovered_array.iter().take(10) {
                                        if let Some(selector) = discovered_item.get("selector") {
                                            if let Some(count) = discovered_item.get("count") {
                                                if let Some(sample) =
                                                    discovered_item.get("sampleText")
                                                {
                                                    let _ = write!(
                                                        user_content,
                                                        "      - {}: {} items, sample: \"{}\"\n",
                                                        selector.as_str().unwrap_or("unknown"),
                                                        count,
                                                        sample
                                                            .as_str()
                                                            .unwrap_or("")
                                                            .chars()
                                                            .take(40)
                                                            .collect::<String>()
                                                    );
                                                } else {
                                                    let _ = write!(
                                                        user_content,
                                                        "      - {}: {} items\n",
                                                        selector.as_str().unwrap_or("unknown"),
                                                        count
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(page_title) = inspection_data.get("pageTitle") {
                            let _ = write!(
                                user_content,
                                "    Page Title: \"{}\"\n",
                                page_title.as_str().unwrap_or("Unknown")
                            );
                        }

                        if let Some(found_count) = inspection_data.get("foundCount") {
                            let _ = write!(
                                user_content,
                                "   [WARNING]  Original selector found {} items\n",
                                found_count
                            );
                        }

                        if let Some(original_selector) = inspection_data.get("originalSelector") {
                            let _ = write!(
                                user_content,
                                "   [SEARCH] Failed selector: \"{}\"\n",
                                original_selector.as_str().unwrap_or("unknown")
                            );
                        }

                        user_content.push_str("   [TIP] USE THIS INFORMATION: Choose selectors from the discovered elements above.\n");
                        user_content.push_str("   [TARGET] RECOMMENDATION: Pick elements with 5-25 items for extraction (not hundreds).\n");

                        // Include any debug information
                        if let Some(debug_fields) = inspection_data.as_object() {
                            let debug_items: Vec<_> = debug_fields
                                .iter()
                                .filter(|(key, _)| key.ends_with("_debug"))
                                .take(3) // Limit to first 3 debug items
                                .collect();

                            if !debug_items.is_empty() {
                                user_content.push_str(
                                    "    DEBUG INFO: Some fields couldn't be extracted:\n",
                                );
                                for (key, value) in debug_items {
                                    if let Some(debug_obj) = value.as_object() {
                                        if let Some(available_selectors) =
                                            debug_obj.get("availableSelectors")
                                        {
                                            if let Some(selectors_array) =
                                                available_selectors.as_array()
                                            {
                                                user_content.push_str(&format!(
                                                    "      - {}: Available elements: {}\n",
                                                    key.replace("_debug", ""),
                                                    selectors_array
                                                        .iter()
                                                        .take(5)
                                                        .map(|s| s.as_str().unwrap_or("unknown"))
                                                        .collect::<Vec<_>>()
                                                        .join(", ")
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        user_content.push('\n');
                    }
                }
            }
            user_content.push('\n');
        }

        // Add failure recovery context if available
        if let Some(tracker) = failure_tracker {
            if tracker.consecutive_failures > 0 {
                let _ = write!(user_content, "\n RECOVERY CONTEXT:\n");
                let _ = write!(user_content, "{}\n", tracker.get_error_summary());

                // Add specific recovery guidance based on error category
                if let Some(category) = &tracker.last_error_category {
                    match category {
                        crate::failure_recovery::ErrorCategory::SelectorNotFound => {
                            user_content.push_str("[TIP] Selector not found - try using text-based selectors or broader matches\n");
                        }
                        crate::failure_recovery::ErrorCategory::CaptchaOrPopup => {
                            user_content.push_str("[TIP] Popup/CAPTCHA detected - consider using Handle family actions\n");
                        }
                        crate::failure_recovery::ErrorCategory::AuthWall => {
                            user_content.push_str("[TIP] Authentication wall detected - may need to handle login flow\n");
                        }
                        crate::failure_recovery::ErrorCategory::FrameSwap => {
                            user_content.push_str("[TIP] DOM structure changed (SPA reload) - refresh your understanding of the page\n");
                        }
                        _ => {}
                    }
                }
                user_content.push('\n');
            }
        }

        user_content.push_str("Based on the page structure above, what should be the next step to achieve the goal? Choose selectors from the elements listed above.");

        messages.push(json!({
            "role": "user",
            "content": user_content
        }));

        messages
    }

    fn assess_selector_quality(&self, element: &crate::DomElement) -> &'static str {
        // Assess selector quality based on element attributes
        if element.id.is_some() {
            " EXCELLENT" // ID selectors are most reliable
        } else if element.attributes.contains_key("data-testid") {
            " EXCELLENT" // Test IDs are designed for automation
        } else if element.attributes.contains_key("aria-label") {
            " GOOD" // Aria labels are stable
        } else if element.attributes.contains_key("name") {
            " GOOD" // Name attributes are usually stable
        } else if !element.classes.is_empty() {
            " FAIR" // Classes can be dynamic
        } else {
            " POOR" // Tag-only selectors are unreliable
        }
    }

    fn build_dom_aware_system_prompt(&self) -> String {
        include_str!("prompts/dom_aware_system.md").to_string()
    }

    /// Build compact system prompt for optimized LLM consumption
    fn build_compact_system_prompt(&self) -> String {
        format!(
            r#"You are an expert browser automation planner using compact DOM snapshots.

{}

## CRITICAL RULES
1. **ALWAYS use EncodedIds from the snapshot** (e.g., btn_1, inp_2) - never make them up
2. **One atomic action at a time** - don't combine multiple interactions  
3. **Human-like behavior** - navigate naturally, type realistically, add delays
4. **Security first** - never bypass warnings or execute untrusted content

## ELEMENT SELECTION PRIORITY
1. Use EncodedIds from the provided snapshot (e.g., btn_1, inp_2, lnk_3)
2. Verify element exists in the snapshot before using
3. Choose elements closest to viewport (lower viewportDistance)
4. Prefer elements with clear actions listed

## ACTION TOOLS
• navigate_to_url(url) - Go to URL (MUST include https://)
• fill_input_field(encoded_id, value) - Fill text fields using EncodedId
• click_element(encoded_id) - Click any element using EncodedId
• press_special_key(encoded_id, key) - Press Enter, Tab, etc. on element
• extract_structured_data(container_encoded_id, fields) - Extract data
• wait_for_element(encoded_id) - Wait for element to appear

## COMPACT SNAPSHOT FORMAT
Elements are grouped by type with:
- EncodedId: Unique identifier (btn_1, inp_2, etc.)
- Selector: CSS selector for fallback
- Text: Element text content
- Actions: Available interactions (click, fill, select, etc.)
- Attributes: Key attributes like id, name, type

## SEARCH PATTERNS (Critical for Google)
❌ NEVER: navigate_to_url("google.com/search?q=...")
✅ ALWAYS: 
1. navigate_to_url("https://google.com")
2. fill_input_field(encoded_id: "inp_1", value: "search term")  # Use search input's EncodedId
3. press_special_key(encoded_id: "inp_1", key: "Enter")

## ERROR RECOVERY
When actions fail:
1. Check if EncodedId exists in the current snapshot
2. Look for alternatives with similar actions
3. Wait for dynamic content if needed
4. Report clear failure with attempted EncodedId

## RESPONSE FORMAT
Always respond with ONE action using EncodedIds:
```json
{{
  "action": "action_name",
  "parameters": {{
    "encoded_id": "btn_1",  // Use exact EncodedId from snapshot
    // other required parameters
  }},
  "reasoning": "why this action achieves the goal using this specific element"
}}
```

REMEMBER: 
- Use EncodedIds from the snapshot (btn_1, inp_2, etc.) NOT CSS selectors
- Verify the element exists in the provided snapshot
- One action at a time - be patient like a human"#,
            COMMON_SECURITY_RULES
        )
    }

    fn build_system_prompt(&self) -> String {
        format!(
            r#"You are an expert browser automation planner that analyzes DOM and plans actions.

{}

## CRITICAL RULES
1. **ONLY use selectors that exist in the provided DOM** - never make them up
2. **One atomic action at a time** - don't combine multiple interactions  
3. **Human-like behavior** - navigate naturally, type realistically, add delays
4. **Security first** - never bypass warnings or execute untrusted content

## SELECTOR PRIORITY (ALWAYS prefer in this order)
1. IDs: `#searchbox`
2. Data attributes: `[data-testid="search"]`, `[jsname="W0wltc"]`
3. Aria labels: `[aria-label="Search"]`
4. Name attributes: `[name="q"]`
5. Semantic HTML: `button[type="submit"]`, `input[type="search"]`
6. AVOID: Generic classes, position selectors, deep nesting

## ACTION TOOLS
• navigate_to_url(url) - Go to URL (MUST include https://)
• fill_input_field(selector, value) - Fill text fields
• click_element(selector) - Click any element
• press_special_key(selector, key) - Press Enter, Tab, etc.
• extract_structured_data(item_selector, fields) - Extract multiple items
• wait_for_element(selector) - Wait for element to appear

## SEARCH PATTERNS (Critical for Google)
[ERROR] NEVER: navigate_to_url("google.com/search?q=...")
[OK] ALWAYS: 
1. navigate_to_url("https://google.com")
2. fill_input_field(selector: "textarea[name='q']", value: "search term")
3. press_special_key(selector: "textarea[name='q']", key: "Enter")

Note: Google now uses TEXTAREA not INPUT for search!

## FORM INTERACTION
- Clear fields before typing: clear_first: true
- Type naturally: simulate_typing: true
- Tab between fields like a human
- Click submit buttons (don't just press Enter)
- Wait after actions: wait_after: 1000

## EXTRACTION STRATEGY
1. Find container pattern (e.g., .g for Google results)
2. Map fields to child selectors
3. Start with 5-10 items to test
4. Use stable attributes for selectors

## ERROR RECOVERY
When selectors fail:
1. Check if DOM has changed
2. Look for alternatives IN THE PROVIDED DOM
3. Try broader selectors if too specific
4. Wait for dynamic content
5. Report clear failure with attempted selector

## RESPONSE FORMAT
Always respond with ONE action:
```json
{{
  "action": "action_name",
  "parameters": {{
    "selector": "exact selector from DOM",
    // other required parameters
  }},
  "reasoning": "why this action achieves the goal"
}}
```

REMEMBER: 
- Analyze the PROVIDED DOM before choosing selectors
- If a selector isn't in the DOM, DO NOT use it
- One action at a time - be patient like a human"#,
            COMMON_SECURITY_RULES
        )
    }

    /// Build a self-healing prompt for fixing broken steps
    pub fn build_healing_prompt(
        &self,
        failed_step: &crate::StepExecution,
        current_dom: &str,
        error_message: &str,
    ) -> Vec<Value> {
        let content = format!(
            "A browser automation step failed and needs to be fixed.\n\nFAILED STEP:\n- id: {}\n- name: {}\n- kind: {}\n\nERROR:\n{}\n\nCURRENT PAGE SNAPSHOT (compact):\n{}\n\nRESPONSE RULES:\n- Respond with ONLY a single JSON object.\n- The JSON MUST be a valid StepKind object with a top-level \"type\" field.\n- Do NOT include \"id\" or \"name\" fields (the runner will set them).\n- Do NOT invent selectors. If you must choose a selector, choose one that appears in the snapshot.\n- Prefer safe steps: click_element, fill_input_field, press_special_key, wait_for_element, wait_for_timeout, scroll_window_to, scroll_element_into_view, extract_structured_data, detect_popups, dismiss_popups, wait_for_no_popups.\n- Avoid high-risk steps unless explicitly required: execute_javascript, set_cookie/clear_cookies, upload_file, download_images, handle_captcha/configure_captcha_solver.\n\nEXAMPLES:\n{{\"type\":\"click_element\",\"selector\":\"#submit\",\"timeout_ms\":8000}}\n{{\"type\":\"fill_input_field\",\"selector\":\"input[name=\\\"q\\\"]\",\"value\":\"rust\",\"timeout_ms\":8000}}\n{{\"type\":\"wait_for_element\",\"selector\":\"#results\",\"timeout_ms\":12000}}\n",
            failed_step.step.id,
            failed_step.step.name,
            serde_json::to_string(&failed_step.step.kind).unwrap_or_else(|_| "<unavailable>".to_string()),
            error_message,
            current_dom
        );

        vec![
            json!({
                "role": "system",
                "content": "You are a browser automation repair specialist. You return a single corrected StepKind JSON object (no prose)."
            }),
            json!({
                "role": "user",
                "content": content
            }),
        ]
    }

    /// Build whitelist-based planner prompt (Tier 1)
    pub fn build_planner_prompt(
        &self,
        goal: &str,
        current_url: &str,
        whitelist: &DomWhitelist,
        failure_cache: Option<&FailureCache>,
        history: &[StepExecution],
    ) -> Vec<Value> {
        let mut messages = vec![json!({
            "role": "system",
            "content": include_str!("prompts/planner.md")
        })];

        // Build user content with whitelist elements
        let mut user_content = String::new();
        user_content.push_str(&format!("Goal: {}\n", goal));
        user_content.push_str(&format!("Current URL: {}\n\n", current_url));

        // Add indexed elements from whitelist
        user_content.push_str(&whitelist.generate_summary());
        user_content.push('\n');

        // Add failure cache information if available
        if let Some(cache) = failure_cache {
            let failure_summary = cache.generate_failure_summary(current_url);
            if !failure_summary.trim().is_empty() {
                user_content.push_str(&failure_summary);
                user_content.push('\n');
            }
        }

        // Add execution history
        if !history.is_empty() {
            user_content.push_str("EXECUTION HISTORY:\n");
            for (i, execution) in history.iter().enumerate() {
                let result_summary = match &execution.result {
                    ExecutionResult::Success {
                        payload: Some(data),
                    } => {
                        if let Some(array) = data.as_array() {
                            format!("[OK] Success - extracted {} items", array.len())
                        } else {
                            "[OK] Success".to_string()
                        }
                    }
                    ExecutionResult::Success { payload: None } => "[OK] Success".to_string(),
                    ExecutionResult::Error { message, .. } => {
                        format!("[ERROR] Failed: {}", message)
                    }
                };

                user_content.push_str(&format!(
                    "{}. {} - {}\n",
                    i + 1,
                    execution.step.name,
                    result_summary
                ));
            }
            user_content.push('\n');
        }

        user_content.push_str("Based on the indexed elements above, what action should be taken next to progress toward the goal?");

        messages.push(json!({
            "role": "user",
            "content": user_content
        }));

        messages
    }

    /// Build planner prompt using compact snapshot (optimized for LLM)
    pub fn build_compact_snapshot_prompt(
        &self,
        goal: &str,
        snapshot: &CompactSnapshot,
        failure_cache: Option<&FailureCache>,
        history: &[StepExecution],
    ) -> Vec<Value> {
        let mut messages = vec![json!({
            "role": "system",
            "content": self.build_compact_system_prompt()
        })];

        let mut user_content = String::new();
        user_content.push_str(&format!("Goal: {}\n\n", goal));

        // Add page context
        user_content.push_str(&format!(
            "Page: {} ({})\n",
            self.truncate_text(&snapshot.title, 50),
            self.format_compact_url(&snapshot.url)
        ));

        user_content.push_str(&format!(
            "Viewport: {}x{}\n",
            snapshot.viewport.width, snapshot.viewport.height
        ));

        if !snapshot.frames.is_empty() {
            let accessible_count = snapshot.frames.iter().filter(|f| f.accessible).count();
            user_content.push_str(&format!(
                "Frames: {} ({} accessible)\n",
                snapshot.frames.len(),
                accessible_count
            ));
        }

        user_content.push('\n');

        // Add interactive elements grouped by type
        if !snapshot.elements.is_empty() {
            user_content.push_str("Interactive Elements:\n");

            let grouped = self.group_elements_by_type(&snapshot.elements);

            for (element_type, elements) in grouped {
                if !elements.is_empty() {
                    user_content.push_str(&format!(
                        "\n{} ({}):\n",
                        element_type.to_uppercase(),
                        elements.len()
                    ));

                    for element in elements {
                        let line = self.format_compact_element(&element);
                        user_content.push_str(&format!("  {}\n", line));
                    }
                }
            }
        } else {
            user_content.push_str("No interactive elements found.\n");
        }

        user_content.push('\n');

        // Add memory summary if available
        if let Some(memory) = &snapshot.memory_summary {
            user_content.push_str("Recent Actions:\n");
            user_content.push_str(memory);
            user_content.push('\n');
        }

        // Add failure cache information
        if let Some(cache) = failure_cache {
            let failure_summary = cache.generate_failure_summary(&snapshot.url);
            if !failure_summary.trim().is_empty() {
                user_content.push_str("Known Issues:\n");
                user_content.push_str(&failure_summary);
                user_content.push('\n');
            }
        }

        // Add execution history (compact format)
        if !history.is_empty() {
            user_content.push_str("Execution History:\n");
            for (i, execution) in history.iter().enumerate() {
                let result_summary = match &execution.result {
                    ExecutionResult::Success {
                        payload: Some(data),
                    } => {
                        if let Some(array) = data.as_array() {
                            format!("✓ {} items", array.len())
                        } else {
                            "✓ Success".to_string()
                        }
                    }
                    ExecutionResult::Success { payload: None } => "✓ Success".to_string(),
                    ExecutionResult::Error { message, .. } => {
                        let short_msg = if message.len() > 30 {
                            format!("{}...", &message[..27])
                        } else {
                            message.clone()
                        };
                        format!("✗ {}", short_msg)
                    }
                };

                user_content.push_str(&format!(
                    "{}. {} - {}\n",
                    i + 1,
                    self.truncate_text(&execution.step.name, 25),
                    result_summary
                ));
            }
            user_content.push('\n');
        }

        // Footer with metadata
        user_content.push_str(&format!(
            "Snapshot: {}KB, {} compression, {} elements\n\n",
            snapshot.size_kb,
            snapshot.compression_level,
            snapshot.elements.len()
        ));

        user_content.push_str(
            "Using the encoded IDs above, what action should be taken next to achieve the goal?",
        );

        messages.push(json!({
            "role": "user",
            "content": user_content
        }));

        messages
    }

    /// Build planner prompt using DOM snapshot (legacy format)
    pub fn build_snapshot_planner_prompt(
        &self,
        goal: &str,
        current_url: &str,
        snapshot: &DomSnapshot,
        failure_cache: Option<&FailureCache>,
        history: &[StepExecution],
    ) -> Vec<Value> {
        let system_content = format!(
            "{}\n\n{}",
            COMMON_SECURITY_RULES,
            include_str!("prompts/planner.md")
        );
        let mut messages = vec![json!({
            "role": "system",
            "content": system_content
        })];

        let mut user_content = String::new();
        user_content.push_str(&format!(
            "{}\n\n",
            wrap_user_request(&format!("Goal: {}", goal))
        ));

        if current_url.is_empty() || current_url == "about:blank" {
            user_content.push_str("Current URL: about:blank (no active page)\n");
        } else {
            user_content.push_str(&format!("Current URL: {}\n", current_url));
        }

        let element_summary = self.format_dom_snapshot_for_planner(snapshot, 120);
        let limited_snapshot = limit_string(&element_summary, self.max_untrusted_chars);
        let was_truncated = limited_snapshot.len() < element_summary.len();
        user_content.push_str(&format!(
            "\nDOM_SNAPSHOT ({} elements{}, use ref @eN):\n{}\n\n",
            snapshot.elements.len(),
            if was_truncated { ", truncated" } else { "" },
            wrap_untrusted_content(&limited_snapshot)
        ));

        // Add failure cache information if available
        if let Some(cache) = failure_cache {
            let failure_summary = cache.generate_failure_summary(current_url);
            if !failure_summary.trim().is_empty() {
                user_content.push_str(&failure_summary);
                user_content.push('\n');
            }
        }

        // Add execution history
        if !history.is_empty() {
            user_content.push_str("EXECUTION HISTORY:\n");
            for (i, execution) in history.iter().enumerate() {
                let result_summary = match &execution.result {
                    ExecutionResult::Success {
                        payload: Some(data),
                    } => {
                        if let Some(array) = data.as_array() {
                            format!("[OK] Success - extracted {} items", array.len())
                        } else {
                            "[OK] Success".to_string()
                        }
                    }
                    ExecutionResult::Success { payload: None } => "[OK] Success".to_string(),
                    ExecutionResult::Error { message, .. } => {
                        format!("[ERROR] Failed: {}", message)
                    }
                };

                user_content.push_str(&format!(
                    "{}. {} - {}\n",
                    i + 1,
                    execution.step.name,
                    result_summary
                ));
            }
            user_content.push('\n');
        }

        user_content.push_str(
            "Based on the DOM snapshot above, choose ONE next atomic action to progress toward the goal.",
        );

        messages.push(json!({
            "role": "user",
            "content": user_content
        }));

        messages
    }

    fn format_dom_snapshot_for_planner(
        &self,
        snapshot: &DomSnapshot,
        max_elements: usize,
    ) -> String {
        let mut lines: Vec<String> = Vec::new();

        lines.push(format!("URL: {}", snapshot.metadata.url));
        lines.push(format!(
            "Title: {}",
            limit_string(&snapshot.metadata.title, 80)
        ));
        lines.push(format!(
            "Viewport: {}x{}",
            snapshot.metadata.viewport.width, snapshot.metadata.viewport.height
        ));
        lines.push(String::new());
        lines.push("Element targeting: idx is 0-based. ref is @e{idx+1}.".to_string());
        lines.push("Preferred targeting in executed steps: selector=\"@eN\" (fallback: selector=\"<css>\").".to_string());
        lines.push(
            "If you see UNKNOWN_REF at runtime: take a fresh snapshot and retry with the new refs."
                .to_string(),
        );

        // Quick tag counts (helps the LLM decide what to do next)
        let mut button_count = 0usize;
        let mut input_count = 0usize;
        let mut link_count = 0usize;
        let mut select_count = 0usize;
        let mut other_count = 0usize;

        for el in &snapshot.elements {
            match el.tag.as_str() {
                "button" => button_count += 1,
                "input" | "textarea" => input_count += 1,
                "a" => link_count += 1,
                "select" => select_count += 1,
                _ => other_count += 1,
            }
        }

        lines.push(format!(
            "Counts: inputs={} buttons={} links={} selects={} other={}",
            input_count, button_count, link_count, select_count, other_count
        ));

        // Group by viewport region for spatial understanding.
        let mut top: Vec<(usize, &crate::broker_client::ElementStub)> = Vec::new();
        let mut middle: Vec<(usize, &crate::broker_client::ElementStub)> = Vec::new();
        let mut bottom: Vec<(usize, &crate::broker_client::ElementStub)> = Vec::new();
        let mut unknown: Vec<(usize, &crate::broker_client::ElementStub)> = Vec::new();

        for (idx, el) in snapshot.elements.iter().enumerate() {
            let bucket = el
                .spatial_info
                .as_ref()
                .map(|s| s.viewport_position.as_str())
                .unwrap_or("unknown");

            match bucket {
                "top" => top.push((idx, el)),
                "middle" => middle.push((idx, el)),
                "bottom" => bottom.push((idx, el)),
                _ => unknown.push((idx, el)),
            }
        }

        let mut shown = 0usize;
        let groups: [(&str, Vec<(usize, &crate::broker_client::ElementStub)>); 4] = [
            ("TOP", top),
            ("MIDDLE", middle),
            ("BOTTOM", bottom),
            ("UNKNOWN", unknown),
        ];

        for (label, group) in groups {
            if shown >= max_elements {
                break;
            }
            if group.is_empty() {
                continue;
            }

            lines.push(String::new());
            lines.push(format!("== {} ==", label));

            for (idx, el) in group {
                if shown >= max_elements {
                    break;
                }
                lines.push(self.format_snapshot_element_line(idx, el));
                shown += 1;
            }
        }

        if snapshot.elements.len() > shown {
            lines.push(String::new());
            lines.push(format!(
                "(Only showing {} of {} elements; request a fresh snapshot after actions.)",
                shown,
                snapshot.elements.len()
            ));
        }

        lines.join("\n")
    }

    fn format_snapshot_element_line(
        &self,
        idx: usize,
        el: &crate::broker_client::ElementStub,
    ) -> String {
        let ref_str = format!("@e{}", idx + 1);

        let mut parts: Vec<String> = Vec::new();
        parts.push(format!("[{}] ref={}", idx, ref_str));

        if let Some(eid) = el.id.as_ref().filter(|s| !s.trim().is_empty()) {
            parts.push(format!("eid={}", eid));
        }

        parts.push(format!("tag={}", el.tag));

        if let Some(text) = el.text.as_ref().map(|t| t.trim()).filter(|t| !t.is_empty()) {
            parts.push(format!("text=\"{}\"", limit_string(text, 50)));
        }

        let mut attr_parts: Vec<String> = Vec::new();
        let priority_keys = [
            "data-testid",
            "data-cy",
            "data-test",
            "aria-label",
            "aria-labelledby",
            "name",
            "id",
            "role",
            "type",
            "placeholder",
            "value",
            "href",
            "src",
            "alt",
            "title",
        ];

        for key in priority_keys {
            if let Some(v) = el.attributes.get(key) {
                let vv = v.trim();
                if vv.is_empty() {
                    continue;
                }
                attr_parts.push(format!("{}=\"{}\"", key, limit_string(vv, 40)));
                if attr_parts.len() >= 4 {
                    break;
                }
            }
        }

        if !attr_parts.is_empty() {
            parts.push(format!("attrs({})", attr_parts.join(" ")));
        }

        if !el.selector.trim().is_empty() {
            parts.push(format!(
                "selector=\"{}\"",
                limit_string(el.selector.trim(), 90)
            ));
        }

        if let Some(spatial) = &el.spatial_info {
            parts.push(format!(
                "pos={},{} size={}x{}",
                spatial.x, spatial.y, spatial.width, spatial.height
            ));
        }

        parts.join(" ")
    }

    /// Build navigator prompt for index-to-selector conversion (Tier 2)
    pub fn build_navigator_prompt(
        &self,
        planned_action: &Value,
        whitelist: &DomWhitelist,
        failure_cache: Option<&FailureCache>,
        current_url: &str,
    ) -> Vec<Value> {
        let mut messages = vec![json!({
            "role": "system",
            "content": include_str!("prompts/navigator.md")
        })];

        let mut user_content = String::new();
        user_content.push_str("PLANNED ACTION TO VALIDATE:\n");
        user_content.push_str(
            &serde_json::to_string_pretty(planned_action)
                .unwrap_or_else(|_| "Failed to serialize action".to_string()),
        );
        user_content.push_str("\n\n");

        // Add element details if action references an index
        if let Some(index) = planned_action
            .get("parameters")
            .and_then(|p| p.get("index"))
            .and_then(|i| i.as_u64())
        {
            if let Some(element) = whitelist.get_element(index as u32) {
                user_content.push_str(&format!("TARGET ELEMENT [{}]:\n", index));
                user_content.push_str(&format!("  Tag: {}\n", element.tag));
                user_content.push_str(&format!("  Primary Selector: {}\n", element.selector));
                user_content.push_str(&format!("  Text: \"{}\"\n", element.text));
                user_content.push_str(&format!("  Confidence: {:.2}\n", element.confidence));
                user_content.push_str(&format!("  Visible: {}\n", element.visible));

                if !element.alt_selectors.is_empty() {
                    user_content.push_str("  Alternative Selectors:\n");
                    for alt in &element.alt_selectors {
                        user_content.push_str(&format!("    - {}\n", alt));
                    }
                }

                if !element.attributes.is_empty() {
                    user_content.push_str("  Key Attributes:\n");
                    for (key, value) in &element.attributes {
                        let truncated_value = if value.len() > 30 {
                            format!("{}...", &value[..27])
                        } else {
                            value.clone()
                        };
                        user_content.push_str(&format!("    {}=\"{}\"\n", key, truncated_value));
                    }
                }
                user_content.push('\n');
            }
        }

        // Add failure cache warnings
        if let Some(cache) = failure_cache {
            let blacklisted = cache.get_blacklisted_selectors();
            if !blacklisted.is_empty() {
                user_content.push_str("BLACKLISTED SELECTORS (avoid these):\n");
                for selector in blacklisted.iter().take(10) {
                    user_content.push_str(&format!("  - {}\n", selector));
                }
                user_content.push('\n');
            }
        }

        user_content.push_str("Please validate this action and convert element indexes to executable selectors. Respond with validation status and optimized selector.");

        messages.push(json!({
            "role": "user",
            "content": user_content
        }));

        messages
    }

    /// Build navigator prompt using DOM snapshot (new format)
    pub fn build_snapshot_navigator_prompt(
        &self,
        planned_action: &Value,
        snapshot: &DomSnapshot,
        failure_cache: Option<&FailureCache>,
        current_url: &str,
    ) -> Vec<Value> {
        let system_content = format!(
            "{}\n\n{}",
            COMMON_SECURITY_RULES,
            include_str!("prompts/navigator.md")
        );
        let mut messages = vec![json!({
            "role": "system",
            "content": system_content
        })];

        let mut user_content = String::new();
        user_content.push_str("PLANNED ACTION TO VALIDATE:\n");
        user_content.push_str(
            &serde_json::to_string_pretty(planned_action)
                .unwrap_or_else(|_| "Failed to serialize action".to_string()),
        );
        user_content.push_str("\n\n");

        user_content.push_str(&format!("Current URL: {}\n\n", current_url));

        // Add element details when the planned action references specific targets.
        let mut target_lines: Vec<String> = Vec::new();

        let parse_ref = |raw: &str| -> Option<usize> {
            let mut s = raw.trim();
            if let Some(rest) = s.strip_prefix("ref=") {
                s = rest;
            }
            if let Some(rest) = s.strip_prefix('@') {
                s = rest;
            }
            let s = s.trim();
            let n = s.strip_prefix('e')?.parse::<usize>().ok()?;
            if n < 1 {
                return None;
            }
            Some(n - 1)
        };

        let element_line_by_index = |idx: usize| -> Option<String> {
            snapshot
                .elements
                .get(idx)
                .map(|el| self.format_snapshot_element_line(idx, el))
        };

        // Primary: parameters.index
        if let Some(index) = planned_action
            .get("parameters")
            .and_then(|p| p.get("index"))
            .and_then(|i| i.as_u64())
        {
            if let Some(line) = element_line_by_index(index as usize) {
                target_lines.push(line);
            }
        }

        // Secondary: parameters.selector (may be @eN or a CSS selector)
        if let Some(sel) = planned_action
            .get("parameters")
            .and_then(|p| p.get("selector"))
            .and_then(|s| s.as_str())
        {
            if let Some(idx) = parse_ref(sel) {
                if let Some(line) = element_line_by_index(idx) {
                    target_lines.push(line);
                }
            } else if let Some((idx, el)) = snapshot
                .elements
                .iter()
                .enumerate()
                .find(|(_, el)| el.selector == sel)
            {
                target_lines.push(self.format_snapshot_element_line(idx, el));
            }
        }

        // drag_and_drop: source_selector + target_selector
        if let Some(source_sel) = planned_action
            .get("parameters")
            .and_then(|p| p.get("source_selector"))
            .and_then(|s| s.as_str())
        {
            if let Some(idx) = parse_ref(source_sel) {
                if let Some(line) = element_line_by_index(idx) {
                    target_lines.push(line);
                }
            } else if let Some((idx, el)) = snapshot
                .elements
                .iter()
                .enumerate()
                .find(|(_, el)| el.selector == source_sel)
            {
                target_lines.push(self.format_snapshot_element_line(idx, el));
            }
        }

        if let Some(target_sel) = planned_action
            .get("parameters")
            .and_then(|p| p.get("target_selector"))
            .and_then(|s| s.as_str())
        {
            if let Some(idx) = parse_ref(target_sel) {
                if let Some(line) = element_line_by_index(idx) {
                    target_lines.push(line);
                }
            } else if let Some((idx, el)) = snapshot
                .elements
                .iter()
                .enumerate()
                .find(|(_, el)| el.selector == target_sel)
            {
                target_lines.push(self.format_snapshot_element_line(idx, el));
            }
        }

        if !target_lines.is_empty() {
            target_lines.sort();
            target_lines.dedup();
            let target_blob = limit_string(&target_lines.join("\n"), 8_000);
            user_content.push_str("TARGET ELEMENT(S) CONTEXT:\n");
            user_content.push_str(&format!("{}\n\n", wrap_untrusted_content(&target_blob)));
        }

        // Add failure cache information if available
        if let Some(cache) = failure_cache {
            let failure_summary = cache.generate_failure_summary(current_url);
            if !failure_summary.trim().is_empty() {
                user_content.push_str("KNOWN FAILURES TO AVOID:\n");
                user_content.push_str(&failure_summary);
                user_content.push_str("\n");
            }
        }

        user_content.push_str(
            "Validate this action. If it references `parameters.index`, translate it to the most reliable executable target (prefer `selector: \"@e{index+1}\"`). If it already has a selector, keep it if valid and improve stability if needed. Respond with ONE JSON object per the Navigator spec.",
        );

        messages.push(json!({
            "role": "user",
            "content": user_content
        }));

        messages
    }

    /// Build validator prompt for action outcome assessment (Tier 3)
    pub fn build_validator_prompt(
        &self,
        executed_action: &crate::StepExecution,
        before_state: &str,
        after_state: &str,
        goal: &str,
        history: &[StepExecution],
    ) -> Vec<Value> {
        let mut messages = vec![json!({
            "role": "system",
            "content": include_str!("prompts/validator.md")
        })];

        let mut user_content = String::new();
        user_content.push_str(&format!("GOAL: {}\n\n", goal));

        user_content.push_str("EXECUTED ACTION:\n");
        user_content.push_str(&format!("  Action: {}\n", executed_action.step.name));
        user_content.push_str(&format!("  Step ID: {}\n", executed_action.step.id));
        user_content.push_str(&format!("  Timestamp: {}\n", executed_action.timestamp));

        // Add action result
        match &executed_action.result {
            ExecutionResult::Success { payload } => {
                user_content.push_str("  Result: [OK] SUCCESS\n");
                if let Some(data) = payload {
                    user_content.push_str("  Returned Data:\n");
                    let data_str = serde_json::to_string_pretty(data)
                        .unwrap_or_else(|_| "Failed to serialize data".to_string());
                    let truncated = if data_str.len() > 500 {
                        format!(
                            "{}...\n(truncated - {} total chars)",
                            &data_str[..497],
                            data_str.len()
                        )
                    } else {
                        data_str
                    };
                    user_content.push_str(&format!("    {}\n", truncated));
                }
            }
            ExecutionResult::Error {
                message,
                retry_suggested,
            } => {
                user_content.push_str("  Result: [ERROR] FAILED\n");
                user_content.push_str(&format!("  Error: {}\n", message));
                user_content.push_str(&format!("  Retry Suggested: {}\n", retry_suggested));
            }
        }
        user_content.push('\n');

        // Add state comparison
        user_content.push_str("PAGE STATE CHANGES:\n");
        user_content.push_str("Before State:\n");
        let before_preview = if before_state.len() > 200 {
            format!("{}...", &before_state[..197])
        } else {
            before_state.to_string()
        };
        user_content.push_str(&format!("  {}\n", before_preview));

        user_content.push_str("After State:\n");
        let after_preview = if after_state.len() > 200 {
            format!("{}...", &after_state[..197])
        } else {
            after_state.to_string()
        };
        user_content.push_str(&format!("  {}\n\n", after_preview));

        // Add progress context
        user_content.push_str(&format!("PROGRESS CONTEXT:\n"));
        user_content.push_str(&format!("  Total Actions: {}\n", history.len()));
        let success_count = history
            .iter()
            .filter(|e| matches!(e.result, ExecutionResult::Success { .. }))
            .count();
        user_content.push_str(&format!("  Successful Actions: {}\n", success_count));
        user_content.push_str(&format!(
            "  Success Rate: {:.1}%\n\n",
            (success_count as f32 / history.len().max(1) as f32) * 100.0
        ));

        user_content.push_str("Please analyze the action outcome and determine:\n");
        user_content.push_str("1. Did the action succeed technically and functionally?\n");
        user_content.push_str("2. Are we closer to the goal?\n");
        user_content.push_str("3. What should happen next?\n");
        user_content.push_str("4. Any learning updates for future actions?");

        messages.push(json!({
            "role": "user",
            "content": user_content
        }));

        messages
    }

    /// Build summary prompt for completed workflows
    pub fn build_summary_prompt(
        &self,
        goal: &str,
        final_result: Option<&Value>,
        history: &[StepExecution],
        success: bool,
    ) -> Vec<Value> {
        let mut messages = vec![json!({
            "role": "system",
            "content": "You are a browser automation workflow summarizer. Create concise summaries of completed automation sessions."
        })];

        let mut user_content = String::new();
        user_content.push_str(&format!("WORKFLOW SUMMARY REQUEST\n\n"));
        user_content.push_str(&format!("Goal: {}\n", goal));
        user_content.push_str(&format!(
            "Status: {}\n",
            if success {
                "[OK] SUCCESS"
            } else {
                "[ERROR] FAILED"
            }
        ));
        user_content.push_str(&format!("Total Actions: {}\n\n", history.len()));

        if let Some(result) = final_result {
            user_content.push_str("Final Result:\n");
            let result_str = serde_json::to_string_pretty(result)
                .unwrap_or_else(|_| "Failed to serialize result".to_string());
            user_content.push_str(&format!("{}\n\n", result_str));
        }

        user_content.push_str("Action Sequence:\n");
        for (i, execution) in history.iter().enumerate() {
            let status = match &execution.result {
                ExecutionResult::Success { .. } => "[OK]",
                ExecutionResult::Error { .. } => "[ERROR]",
            };
            user_content.push_str(&format!(
                "{}. {} {} {}\n",
                i + 1,
                status,
                execution.step.name,
                execution.step.id
            ));
        }

        user_content.push_str("\nPlease provide a concise summary including:\n");
        user_content.push_str("- What was accomplished\n");
        user_content.push_str("- Key steps taken\n");
        user_content.push_str("- Any notable patterns or issues\n");
        user_content.push_str("- Recommendations for similar tasks");

        messages.push(json!({
            "role": "user",
            "content": user_content
        }));

        messages
    }

    /// Group elements by type for organized display
    fn group_elements_by_type<'a>(
        &self,
        elements: &'a [CompactElement],
    ) -> std::collections::HashMap<&'a str, Vec<&'a CompactElement>> {
        let mut groups = std::collections::HashMap::new();

        for element in elements {
            let group_name = match element.tag.as_str() {
                "input" | "textarea" => "inputs",
                "button" => "buttons",
                "a" => "links",
                "select" => "selects",
                _ => {
                    if element.role.as_deref() == Some("button") {
                        "buttons"
                    } else if element.role.as_deref() == Some("link") {
                        "links"
                    } else if element.role.as_deref() == Some("combobox") {
                        "selects"
                    } else {
                        "other"
                    }
                }
            };

            groups
                .entry(group_name)
                .or_insert_with(Vec::new)
                .push(element);
        }

        groups
    }

    /// Format compact element for LLM prompt
    fn format_compact_element(&self, element: &CompactElement) -> String {
        let mut parts = Vec::new();

        // Encoded ID (most important)
        parts.push(element.encoded_id.clone());

        // Selector
        parts.push(format!("[{}]", self.truncate_text(&element.selector, 30)));

        // Text content
        if let Some(text) = &element.text {
            if !text.is_empty() {
                parts.push(format!("\"{}\"", self.truncate_text(text, 25)));
            }
        } else if let Some(name) = &element.name {
            if !name.is_empty() {
                parts.push(format!("\"{}\"", self.truncate_text(name, 25)));
            }
        }

        // Actions
        if let Some(actions) = &element.actions {
            if !actions.is_empty() {
                parts.push(format!("({})", actions.join(", ")));
            }
        }

        // Key attributes
        if let Some(attrs) = &element.attrs {
            let key_attrs = self.format_key_attributes(attrs);
            if !key_attrs.is_empty() {
                parts.push(format!("{{{}}}", key_attrs.join(", ")));
            }
        }

        // Type if different from tag
        if let Some(element_type) = &element.element_type {
            if element_type != &element.tag {
                parts.push(format!("type:{}", element_type));
            }
        }

        parts.join(" ")
    }

    /// Format key attributes for display
    fn format_key_attributes(
        &self,
        attrs: &std::collections::HashMap<String, String>,
    ) -> Vec<String> {
        let mut display = Vec::new();
        let priority = ["data-testid", "id", "name", "type", "placeholder"];

        for key in &priority {
            if let Some(value) = attrs.get(*key) {
                display.push(format!("{}={}", key, self.truncate_text(value, 20)));
                break; // Only show one key attribute
            }
        }

        display
    }

    /// Format URL for compact display
    fn format_compact_url(&self, url: &str) -> String {
        if let Ok(parsed) = url::Url::parse(url) {
            let domain = parsed
                .host_str()
                .unwrap_or("unknown")
                .trim_start_matches("www.");
            let path = parsed.path();

            if path == "/" || path.is_empty() {
                domain.to_string()
            } else {
                let truncated_path = if path.len() > 30 {
                    format!("{}...", &path[..27])
                } else {
                    path.to_string()
                };
                format!("{}{}", domain, truncated_path)
            }
        } else {
            self.truncate_text(url, 40)
        }
    }

    /// Truncate text to specified length
    fn truncate_text(&self, text: &str, max_len: usize) -> String {
        if text.len() <= max_len {
            text.to_string()
        } else {
            format!("{}...", &text[..max_len.saturating_sub(3)])
        }
    }
}

/// Limit string to max characters
fn limit_string(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        let mut end = max_chars.saturating_sub(3);
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}
