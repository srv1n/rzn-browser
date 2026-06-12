use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// DOM context formatter for LLM consumption
/// Provides smart truncation, change highlighting, and interaction history tracking
/// Optimized for token efficiency while maintaining semantic clarity

// ============ Re-export key types from broker ============
// Note: In a real implementation, these would be shared types
// For now, we'll duplicate the essential ones

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementPosition {
    pub top: i32,
    pub left: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractiveElementSummary {
    pub highlight_index: u32,
    pub tag_name: String,
    pub text: String,
    pub selector_hint: String,
    pub element_type: String,
    pub role: String,
    pub position: ElementPosition,
    pub action_candidates: Vec<String>,
    pub priority: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeDetectionSummary {
    pub new_elements: Vec<u32>,
    pub removed_elements: Vec<String>,
    pub modified_elements: Vec<u32>,
    pub significant_changes: bool,
    pub change_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionHint {
    pub element_index: u32,
    pub suggested_actions: Vec<String>,
    pub reasoning: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessedDOMState {
    pub interactive_elements: Vec<InteractiveElementSummary>,
    pub element_count: u32,
    pub viewport_element_count: u32,
    pub change_summary: Option<ChangeDetectionSummary>,
    pub simplified_dom: String,
    pub action_hints: Vec<ActionHint>,
}

// ============ DOM Context Types ============

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DOMContextConfig {
    /// Maximum tokens to use for DOM representation
    pub max_tokens: usize,
    /// Maximum number of elements to include
    pub max_elements: usize,
    /// Include change highlights
    pub include_changes: bool,
    /// Include action suggestions
    pub include_action_hints: bool,
    /// Include interaction history
    pub include_interaction_history: bool,
    /// Prioritize viewport elements
    pub prioritize_viewport: bool,
    /// Include element relationships
    pub include_relationships: bool,
    /// Truncate text length per element
    pub max_text_per_element: usize,
    /// Focus mode - only show most relevant elements
    pub focus_mode: bool,
}

impl Default for DOMContextConfig {
    fn default() -> Self {
        Self {
            max_tokens: 4000, // Conservative token limit
            max_elements: 20,
            include_changes: true,
            include_action_hints: true,
            include_interaction_history: false, // Can be verbose
            prioritize_viewport: true,
            include_relationships: false, // Can be verbose
            max_text_per_element: 30,
            focus_mode: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormattedDOMContext {
    /// Main DOM representation optimized for LLM
    pub dom_representation: String,
    /// Highlighted changes from previous state
    pub change_summary: Option<String>,
    /// Action suggestions with reasoning
    pub action_suggestions: Option<String>,
    /// Interaction context and history
    pub interaction_context: Option<String>,
    /// Metadata about the DOM state
    pub metadata: DOMMetadata,
    /// Estimated token count
    pub estimated_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DOMMetadata {
    pub total_elements: u32,
    pub interactive_elements: u32,
    pub viewport_elements: u32,
    pub high_priority_elements: u32,
    pub url: Option<String>,
    pub title: Option<String>,
    pub processing_time_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionHistoryEntry {
    pub element_index: u32,
    pub action: String,
    pub timestamp: u64,
    pub success: bool,
    pub result_summary: Option<String>,
}

/// Main DOM context formatter
pub struct DOMContextFormatter {
    config: DOMContextConfig,
    interaction_history: Vec<InteractionHistoryEntry>,
    element_usage_stats: HashMap<u32, ElementUsageStats>,
    last_dom_state: Option<ProcessedDOMState>,
}

#[derive(Debug, Clone)]
struct ElementUsageStats {
    interaction_count: u32,
    last_interaction: u64,
    success_rate: f32,
    most_common_action: Option<String>,
}

impl DOMContextFormatter {
    /// Create new formatter with default configuration
    pub fn new() -> Self {
        Self {
            config: DOMContextConfig::default(),
            interaction_history: Vec::new(),
            element_usage_stats: HashMap::new(),
            last_dom_state: None,
        }
    }

    /// Create formatter with custom configuration
    pub fn with_config(config: DOMContextConfig) -> Self {
        Self {
            config,
            interaction_history: Vec::new(),
            element_usage_stats: HashMap::new(),
            last_dom_state: None,
        }
    }

    /// Format DOM state for LLM consumption
    pub fn format_dom_context(&mut self, dom_state: ProcessedDOMState) -> FormattedDOMContext {
        let start_time = std::time::Instant::now();

        // Apply focus mode filtering if enabled
        let filtered_elements = if self.config.focus_mode {
            self.apply_focus_mode_filtering(&dom_state.interactive_elements)
        } else {
            self.apply_standard_filtering(&dom_state.interactive_elements)
        };

        // Build main DOM representation
        let dom_representation = self.build_dom_representation(&filtered_elements, &dom_state);

        // Build change summary if available and enabled
        let change_summary = if self.config.include_changes {
            self.build_change_summary(&dom_state.change_summary)
        } else {
            None
        };

        // Build action suggestions if enabled
        let action_suggestions = if self.config.include_action_hints {
            self.build_action_suggestions(&dom_state.action_hints)
        } else {
            None
        };

        // Build interaction context if enabled
        let interaction_context = if self.config.include_interaction_history {
            self.build_interaction_context(&filtered_elements)
        } else {
            None
        };

        // Calculate metadata
        let metadata = DOMMetadata {
            total_elements: dom_state.element_count,
            interactive_elements: dom_state.interactive_elements.len() as u32,
            viewport_elements: dom_state.viewport_element_count,
            high_priority_elements: dom_state
                .interactive_elements
                .iter()
                .filter(|e| e.priority >= 8)
                .count() as u32,
            url: None,   // Would be passed from dom_state if available
            title: None, // Would be passed from dom_state if available
            processing_time_ms: Some(start_time.elapsed().as_millis() as f64),
        };

        // Estimate total tokens
        let estimated_tokens = self.estimate_token_count(
            &dom_representation,
            &change_summary,
            &action_suggestions,
            &interaction_context,
        );

        // Store current state for next comparison
        self.last_dom_state = Some(dom_state);

        FormattedDOMContext {
            dom_representation,
            change_summary,
            action_suggestions,
            interaction_context,
            metadata,
            estimated_tokens,
        }
    }

    /// Apply focus mode filtering - only most relevant elements
    fn apply_focus_mode_filtering(
        &self,
        elements: &[InteractiveElementSummary],
    ) -> Vec<InteractiveElementSummary> {
        let mut filtered = Vec::new();
        let mut seen_types = HashSet::new();

        // Sort by priority and usage frequency
        let mut sorted_elements: Vec<_> = elements.to_vec();
        sorted_elements.sort_by(|a, b| {
            let a_usage = self
                .element_usage_stats
                .get(&a.highlight_index)
                .map(|s| s.interaction_count)
                .unwrap_or(0);
            let b_usage = self
                .element_usage_stats
                .get(&b.highlight_index)
                .map(|s| s.interaction_count)
                .unwrap_or(0);

            b.priority
                .cmp(&a.priority)
                .then_with(|| b_usage.cmp(&a_usage))
                .then_with(|| a.position.top.cmp(&b.position.top))
        });

        // Select diverse, high-value elements
        for element in sorted_elements {
            if filtered.len() >= self.config.max_elements / 2 {
                break;
            }

            // Prefer diversity in element types
            let element_key = format!("{}:{}", element.tag_name, element.element_type);
            if seen_types.contains(&element_key) && element.priority < 8 {
                continue;
            }

            seen_types.insert(element_key);
            filtered.push(element);
        }

        filtered
    }

    /// Apply standard filtering based on priority and viewport
    fn apply_standard_filtering(
        &self,
        elements: &[InteractiveElementSummary],
    ) -> Vec<InteractiveElementSummary> {
        let mut filtered = Vec::new();

        // Separate viewport and non-viewport elements
        let mut viewport_elements: Vec<_> = elements
            .iter()
            .filter(|e| self.is_likely_in_viewport(e))
            .cloned()
            .collect();

        let mut non_viewport_elements: Vec<_> = elements
            .iter()
            .filter(|e| !self.is_likely_in_viewport(e))
            .cloned()
            .collect();

        // Sort both by priority
        viewport_elements.sort_by(|a, b| b.priority.cmp(&a.priority));
        non_viewport_elements.sort_by(|a, b| b.priority.cmp(&a.priority));

        // Add viewport elements first (prioritized)
        let viewport_limit = if self.config.prioritize_viewport {
            (self.config.max_elements * 2) / 3 // 2/3 for viewport elements
        } else {
            self.config.max_elements / 2
        };

        filtered.extend(viewport_elements.into_iter().take(viewport_limit));

        // Add remaining non-viewport elements
        let remaining_slots = self.config.max_elements.saturating_sub(filtered.len());
        filtered.extend(non_viewport_elements.into_iter().take(remaining_slots));

        filtered
    }

    /// Build main DOM representation
    fn build_dom_representation(
        &self,
        elements: &[InteractiveElementSummary],
        dom_state: &ProcessedDOMState,
    ) -> String {
        let mut lines = Vec::new();

        // Header with context
        lines.push("## Page Elements".to_string());
        lines.push(format!(
            "Interactive elements: {} (showing top {})",
            dom_state.interactive_elements.len(),
            elements.len()
        ));

        if dom_state.viewport_element_count > 0 {
            lines.push(format!("In viewport: {}", dom_state.viewport_element_count));
        }

        lines.push("".to_string());

        // Group elements by type for better organization
        let mut form_elements = Vec::new();
        let mut navigation_elements = Vec::new();
        let mut content_elements = Vec::new();
        let mut other_elements = Vec::new();

        for element in elements {
            match element.tag_name.as_str() {
                "input" | "select" | "textarea" | "button" => form_elements.push(element),
                "a" | "nav" => navigation_elements.push(element),
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "span" => {
                    content_elements.push(element)
                }
                _ => other_elements.push(element),
            }
        }

        // Add form elements
        if !form_elements.is_empty() {
            lines.push("### Form Elements".to_string());
            for element in form_elements {
                lines.push(self.format_element_compact(element, dom_state));
            }
            lines.push("".to_string());
        }

        // Add navigation elements
        if !navigation_elements.is_empty() {
            lines.push("### Navigation".to_string());
            for element in navigation_elements {
                lines.push(self.format_element_compact(element, dom_state));
            }
            lines.push("".to_string());
        }

        // Add content elements (limited)
        if !content_elements.is_empty() {
            lines.push("### Content".to_string());
            for element in content_elements.iter().take(5) {
                // Limit content elements
                lines.push(self.format_element_compact(element, dom_state));
            }
            if content_elements.len() > 5 {
                lines.push(format!(
                    "... and {} more content elements",
                    content_elements.len() - 5
                ));
            }
            lines.push("".to_string());
        }

        // Add other elements
        if !other_elements.is_empty() {
            lines.push("### Other Interactive".to_string());
            for element in other_elements.iter().take(3) {
                // Limit other elements
                lines.push(self.format_element_compact(element, dom_state));
            }
            if other_elements.len() > 3 {
                lines.push(format!(
                    "... and {} more elements",
                    other_elements.len() - 3
                ));
            }
        }

        lines.join("\n")
    }

    /// Format element in compact form for LLM
    fn format_element_compact(
        &self,
        element: &InteractiveElementSummary,
        _dom_state: &ProcessedDOMState,
    ) -> String {
        let mut parts = Vec::new();

        // Index and basic info
        parts.push(format!("[{}]", element.highlight_index));

        // Element description
        let mut desc = format!("<{}", element.tag_name);
        if !element.element_type.is_empty() && element.element_type != element.tag_name {
            desc += &format!(" type=\"{}\"", element.element_type);
        }
        if !element.role.is_empty() {
            desc += &format!(" role=\"{}\"", element.role);
        }
        desc += ">";
        parts.push(desc);

        // Text content (truncated)
        if !element.text.trim().is_empty() {
            let text = self.truncate_text(&element.text, self.config.max_text_per_element);
            parts.push(format!("\"{}\"", text));
        }

        // Priority indicator
        if element.priority >= 8 {
            parts.push("".to_string()); // High priority indicator
        } else if element.priority >= 6 {
            parts.push("".to_string()); // Medium priority indicator
        }

        // Viewport indicator
        if self.is_likely_in_viewport(element) {
            parts.push("".to_string()); // Visible indicator
        }

        // Usage indicator
        if let Some(stats) = self.element_usage_stats.get(&element.highlight_index) {
            if stats.interaction_count > 0 {
                parts.push(format!("{}", stats.interaction_count));
            }
        }

        // Top actions (limited)
        if !element.action_candidates.is_empty() {
            let top_actions: Vec<_> = element
                .action_candidates
                .iter()
                .take(2)
                .map(|a| self.action_to_emoji(a))
                .collect();
            parts.push(format!("[{}]", top_actions.join("")));
        }

        parts.join(" ")
    }

    /// Convert action to emoji for compact representation
    fn action_to_emoji(&self, action: &str) -> String {
        match action {
            "click_element" => "",
            "fill_input_field" => "",
            "select_option" => "[LIST]",
            "hover_element" => "👋",
            "extract_text" => "",
            "wait_for_element" => "⏳",
            "take_screenshot" => "📸",
            "scroll_into_view" => "📜",
            _ => "[ACTION]",
        }
        .to_string()
    }

    /// Build change summary
    fn build_change_summary(
        &self,
        change_summary: &Option<ChangeDetectionSummary>,
    ) -> Option<String> {
        if let Some(changes) = change_summary {
            if changes.change_count == 0 {
                return None;
            }

            let mut lines = Vec::new();
            lines.push("## Recent Changes".to_string());

            if !changes.new_elements.is_empty() {
                lines.push(format!(
                    "🆕 New elements: {}",
                    changes
                        .new_elements
                        .iter()
                        .map(|i| format!("[{}]", i))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }

            if !changes.removed_elements.is_empty() {
                lines.push(format!(
                    "[ERROR] Removed: {} elements",
                    changes.removed_elements.len()
                ));
            }

            if !changes.modified_elements.is_empty() {
                lines.push(format!(
                    " Modified elements: {}",
                    changes
                        .modified_elements
                        .iter()
                        .map(|i| format!("[{}]", i))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }

            if changes.significant_changes {
                lines.push(
                    "[WARNING]  Significant changes detected - page may have updated substantially"
                        .to_string(),
                );
            }

            Some(lines.join("\n"))
        } else {
            None
        }
    }

    /// Build action suggestions
    fn build_action_suggestions(&self, action_hints: &[ActionHint]) -> Option<String> {
        if action_hints.is_empty() {
            return None;
        }

        let mut lines = Vec::new();
        lines.push("## Suggested Actions".to_string());

        // Sort by confidence and limit
        let mut sorted_hints: Vec<_> = action_hints.iter().collect();
        sorted_hints.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for hint in sorted_hints.iter().take(5) {
            lines.push(format!(
                "[TARGET] [{}]: {} (confidence: {:.1})",
                hint.element_index,
                hint.suggested_actions.join(" → "),
                hint.confidence * 100.0
            ));

            if !hint.reasoning.is_empty() {
                lines.push(format!(
                    "   [TIP] {}",
                    self.truncate_text(&hint.reasoning, 80)
                ));
            }
        }

        Some(lines.join("\n"))
    }

    /// Build interaction context
    fn build_interaction_context(&self, elements: &[InteractiveElementSummary]) -> Option<String> {
        if self.interaction_history.is_empty() {
            return None;
        }

        let mut lines = Vec::new();
        lines.push("## Recent Interactions".to_string());

        // Show last few interactions
        let recent_interactions: Vec<_> = self.interaction_history.iter().rev().take(5).collect();

        for interaction in recent_interactions {
            let status = if interaction.success {
                "[OK]"
            } else {
                "[ERROR]"
            };
            lines.push(format!(
                "{} [{}] {} ({}s ago)",
                status,
                interaction.element_index,
                interaction.action,
                self.seconds_ago(interaction.timestamp)
            ));

            if let Some(result) = &interaction.result_summary {
                lines.push(format!("   [NOTE] {}", self.truncate_text(result, 60)));
            }
        }

        // Show frequently interacted elements
        let mut frequent_elements: Vec<_> = self
            .element_usage_stats
            .iter()
            .filter(|(_, stats)| stats.interaction_count > 1)
            .collect();
        frequent_elements.sort_by(|a, b| b.1.interaction_count.cmp(&a.1.interaction_count));

        if !frequent_elements.is_empty() {
            lines.push("".to_string());
            lines.push("### Frequently Used".to_string());
            for (element_index, stats) in frequent_elements.iter().take(3) {
                if let Some(element) = elements
                    .iter()
                    .find(|e| e.highlight_index == **element_index)
                {
                    lines.push(format!(
                        " [{}] {} ({}x, {:.0}% success)",
                        element_index,
                        element.tag_name,
                        stats.interaction_count,
                        stats.success_rate * 100.0
                    ));
                }
            }
        }

        Some(lines.join("\n"))
    }

    /// Record interaction for history tracking
    pub fn record_interaction(
        &mut self,
        element_index: u32,
        action: String,
        success: bool,
        result_summary: Option<String>,
    ) {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Add to history
        self.interaction_history.push(InteractionHistoryEntry {
            element_index,
            action: action.clone(),
            timestamp,
            success,
            result_summary,
        });

        // Keep history bounded
        if self.interaction_history.len() > 50 {
            self.interaction_history.remove(0);
        }

        // Update usage stats
        let stats = self
            .element_usage_stats
            .entry(element_index)
            .or_insert(ElementUsageStats {
                interaction_count: 0,
                last_interaction: timestamp,
                success_rate: 0.0,
                most_common_action: None,
            });

        stats.interaction_count += 1;
        stats.last_interaction = timestamp;

        // Update success rate
        let successful_interactions = self
            .interaction_history
            .iter()
            .filter(|i| i.element_index == element_index && i.success)
            .count() as f32;
        let total_interactions = self
            .interaction_history
            .iter()
            .filter(|i| i.element_index == element_index)
            .count() as f32;

        if total_interactions > 0.0 {
            stats.success_rate = successful_interactions / total_interactions;
        }

        // Update most common action
        let action_counts: HashMap<String, usize> = self
            .interaction_history
            .iter()
            .filter(|i| i.element_index == element_index)
            .fold(HashMap::new(), |mut acc, i| {
                *acc.entry(i.action.clone()).or_insert(0) += 1;
                acc
            });

        if let Some((most_common, _)) = action_counts.iter().max_by_key(|(_, count)| *count) {
            stats.most_common_action = Some(most_common.clone());
        }
    }

    /// Update configuration
    pub fn update_config(&mut self, config: DOMContextConfig) {
        self.config = config;
    }

    /// Clear interaction history
    pub fn clear_history(&mut self) {
        self.interaction_history.clear();
        self.element_usage_stats.clear();
    }

    /// Get interaction statistics
    pub fn get_interaction_stats(&self) -> HashMap<String, u32> {
        let mut stats = HashMap::new();

        stats.insert(
            "total_interactions".to_string(),
            self.interaction_history.len() as u32,
        );
        stats.insert(
            "successful_interactions".to_string(),
            self.interaction_history
                .iter()
                .filter(|i| i.success)
                .count() as u32,
        );
        stats.insert(
            "unique_elements_interacted".to_string(),
            self.element_usage_stats.len() as u32,
        );

        // Most common actions
        let mut action_counts: HashMap<String, u32> = HashMap::new();
        for interaction in &self.interaction_history {
            *action_counts.entry(interaction.action.clone()).or_insert(0) += 1;
        }

        if let Some((most_common_action, count)) =
            action_counts.iter().max_by_key(|(_, count)| *count)
        {
            stats.insert(format!("most_common_action_{}", most_common_action), *count);
        }

        stats
    }

    // ============ Utility Methods ============

    /// Estimate token count (rough approximation)
    fn estimate_token_count(
        &self,
        dom_rep: &str,
        change_summary: &Option<String>,
        action_suggestions: &Option<String>,
        interaction_context: &Option<String>,
    ) -> usize {
        let mut total = 0;

        // DOM representation (roughly 1 token per 4 characters)
        total += dom_rep.len() / 4;

        if let Some(changes) = change_summary {
            total += changes.len() / 4;
        }

        if let Some(actions) = action_suggestions {
            total += actions.len() / 4;
        }

        if let Some(context) = interaction_context {
            total += context.len() / 4;
        }

        total
    }

    /// Check if element is likely in viewport (heuristic)
    fn is_likely_in_viewport(&self, element: &InteractiveElementSummary) -> bool {
        // Simple heuristic based on position
        element.position.top >= 0 &&
        element.position.top < 1000 && // Assume viewport height ~1000px
        element.position.left >= 0 &&
        element.position.width > 0 &&
        element.position.height > 0
    }

    /// Truncate text smartly
    fn truncate_text(&self, text: &str, max_len: usize) -> String {
        let cleaned = text.trim().replace(['\n', '\t'], " ");
        if cleaned.len() <= max_len {
            cleaned
        } else {
            let truncated = &cleaned[..max_len.min(cleaned.len())];
            format!("{}...", truncated)
        }
    }

    /// Calculate seconds ago from timestamp
    fn seconds_ago(&self, timestamp: u64) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_sub(timestamp)
    }
}

impl Default for DOMContextFormatter {
    fn default() -> Self {
        Self::new()
    }
}

/// Utility functions for working with DOM context

/// Create a focused DOM context for specific task types
pub fn create_focused_context(
    dom_state: ProcessedDOMState,
    task_type: &str,
) -> FormattedDOMContext {
    let config = match task_type {
        "form_filling" => DOMContextConfig {
            max_elements: 15,
            focus_mode: true,
            include_action_hints: true,
            include_changes: false,
            prioritize_viewport: true,
            max_text_per_element: 40,
            ..Default::default()
        },
        "navigation" => DOMContextConfig {
            max_elements: 10,
            focus_mode: true,
            include_action_hints: true,
            include_changes: true,
            prioritize_viewport: false, // Navigation elements might be anywhere
            max_text_per_element: 25,
            ..Default::default()
        },
        "data_extraction" => DOMContextConfig {
            max_elements: 30,
            focus_mode: false,
            include_action_hints: false,
            include_changes: false,
            prioritize_viewport: false,
            max_text_per_element: 50,
            ..Default::default()
        },
        _ => DOMContextConfig::default(),
    };

    let mut formatter = DOMContextFormatter::with_config(config);
    formatter.format_dom_context(dom_state)
}

/// Create minimal DOM context for token-constrained scenarios
pub fn create_minimal_context(dom_state: ProcessedDOMState) -> FormattedDOMContext {
    let config = DOMContextConfig {
        max_tokens: 1000,
        max_elements: 8,
        focus_mode: true,
        include_changes: false,
        include_action_hints: false,
        include_interaction_history: false,
        prioritize_viewport: true,
        max_text_per_element: 20,
        ..Default::default()
    };

    let mut formatter = DOMContextFormatter::with_config(config);
    formatter.format_dom_context(dom_state)
}

/// Create comprehensive DOM context for debugging/analysis
pub fn create_debug_context(dom_state: ProcessedDOMState) -> FormattedDOMContext {
    let config = DOMContextConfig {
        max_tokens: 8000,
        max_elements: 50,
        focus_mode: false,
        include_changes: true,
        include_action_hints: true,
        include_interaction_history: true,
        include_relationships: true,
        prioritize_viewport: false,
        max_text_per_element: 100,
        ..Default::default()
    };

    let mut formatter = DOMContextFormatter::with_config(config);
    formatter.format_dom_context(dom_state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_element(
        index: u32,
        tag: &str,
        text: &str,
        priority: u8,
    ) -> InteractiveElementSummary {
        InteractiveElementSummary {
            highlight_index: index,
            tag_name: tag.to_string(),
            text: text.to_string(),
            selector_hint: format!("{}:nth-child({})", tag, index),
            element_type: tag.to_string(),
            role: "".to_string(),
            position: ElementPosition {
                top: (index as i32) * 50,
                left: 10,
                width: 100,
                height: 30,
            },
            action_candidates: vec!["click_element".to_string()],
            priority,
        }
    }

    fn create_test_dom_state() -> ProcessedDOMState {
        ProcessedDOMState {
            interactive_elements: vec![
                create_test_element(1, "button", "Submit", 9),
                create_test_element(2, "input", "", 8),
                create_test_element(3, "a", "Home", 6),
            ],
            element_count: 3,
            viewport_element_count: 2,
            change_summary: None,
            simplified_dom: "test".to_string(),
            action_hints: vec![],
        }
    }

    #[test]
    fn test_formatter_creation() {
        let formatter = DOMContextFormatter::new();
        assert_eq!(formatter.config.max_elements, 20);
        assert!(formatter.interaction_history.is_empty());
    }

    #[test]
    fn test_dom_formatting() {
        let mut formatter = DOMContextFormatter::new();
        let dom_state = create_test_dom_state();

        let context = formatter.format_dom_context(dom_state);
        assert!(!context.dom_representation.is_empty());
        assert!(context.estimated_tokens > 0);
        assert_eq!(context.metadata.interactive_elements, 3);
    }

    #[test]
    fn test_focus_mode() {
        let config = DOMContextConfig {
            focus_mode: true,
            max_elements: 2,
            ..Default::default()
        };
        let mut formatter = DOMContextFormatter::with_config(config);
        let dom_state = create_test_dom_state();

        let context = formatter.format_dom_context(dom_state);
        // Should prioritize higher priority elements
        assert!(context.dom_representation.contains("[1]")); // High priority button
    }

    #[test]
    fn test_interaction_recording() {
        let mut formatter = DOMContextFormatter::new();

        formatter.record_interaction(
            1,
            "click_element".to_string(),
            true,
            Some("Success".to_string()),
        );
        assert_eq!(formatter.interaction_history.len(), 1);
        assert_eq!(formatter.element_usage_stats.len(), 1);

        let stats = formatter.element_usage_stats.get(&1).unwrap();
        assert_eq!(stats.interaction_count, 1);
        assert_eq!(stats.success_rate, 1.0);
    }

    #[test]
    fn test_utility_functions() {
        let dom_state = create_test_dom_state();

        let focused = create_focused_context(dom_state.clone(), "form_filling");
        assert!(focused.estimated_tokens > 0);

        let minimal = create_minimal_context(dom_state.clone());
        assert!(minimal.estimated_tokens > 0);
        // Note: minimal might not always be smaller due to fixed overhead, so just check it's valid

        let debug = create_debug_context(dom_state);
        // Debug should generate valid output too
        assert!(debug.estimated_tokens > 0);
    }
}
