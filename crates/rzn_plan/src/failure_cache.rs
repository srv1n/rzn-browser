use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Failure information for a specific selector
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectorFailure {
    /// The selector that failed
    pub selector: String,
    /// Number of times this selector has failed
    pub failure_count: u32,
    /// Last time this selector failed
    pub last_failure: DateTime<Utc>,
    /// First time this selector failed
    pub first_failure: DateTime<Utc>,
    /// Reason for failure
    pub failure_reason: String,
    /// URL where the failure occurred
    pub url: String,
    /// Action type that failed
    pub action_type: String,
    /// Whether this selector is permanently blacklisted
    pub blacklisted: bool,
    /// Alternative selectors that were tried
    pub attempted_alternatives: Vec<String>,
}

/// Context information about when a failure occurred
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureContext {
    /// Current URL
    pub url: String,
    /// Page title (if available)
    pub page_title: Option<String>,
    /// Action being attempted
    pub action_type: String,
    /// Element index that failed (if using whitelist system)
    pub element_index: Option<u32>,
    /// DOM snippet around the failed element (for analysis)
    pub dom_context: Option<String>,
    /// User goal being pursued
    pub goal: Option<String>,
}

/// Statistics about failure patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureStats {
    /// Total number of failures recorded
    pub total_failures: u32,
    /// Number of unique selectors that have failed
    pub unique_failed_selectors: u32,
    /// Most common failure reason
    pub most_common_failure: Option<String>,
    /// URLs with the most failures
    pub problematic_urls: Vec<(String, u32)>,
    /// Actions with the most failures
    pub problematic_actions: Vec<(String, u32)>,
}

/// Cache system for tracking and preventing repeated selector failures
#[derive(Debug, Clone)]
pub struct FailureCache {
    /// Map of selector -> failure information
    failures: HashMap<String, SelectorFailure>,
    /// Global failure statistics
    stats: FailureStats,
    /// Maximum age for failure records (in hours)
    max_age_hours: i64,
    /// Failure count threshold for blacklisting
    blacklist_threshold: u32,
    /// Whether to enable automatic blacklisting
    auto_blacklist: bool,
}

impl FailureCache {
    /// Create new failure cache with default settings
    pub fn new() -> Self {
        Self {
            failures: HashMap::new(),
            stats: FailureStats {
                total_failures: 0,
                unique_failed_selectors: 0,
                most_common_failure: None,
                problematic_urls: Vec::new(),
                problematic_actions: Vec::new(),
            },
            max_age_hours: 24,      // Keep failures for 24 hours
            blacklist_threshold: 3, // Blacklist after 3 failures
            auto_blacklist: true,
        }
    }

    /// Create failure cache with custom settings
    pub fn with_settings(
        max_age_hours: i64,
        blacklist_threshold: u32,
        auto_blacklist: bool,
    ) -> Self {
        Self {
            failures: HashMap::new(),
            stats: FailureStats {
                total_failures: 0,
                unique_failed_selectors: 0,
                most_common_failure: None,
                problematic_urls: Vec::new(),
                problematic_actions: Vec::new(),
            },
            max_age_hours,
            blacklist_threshold,
            auto_blacklist,
        }
    }

    /// Record a selector failure
    pub fn record_failure(
        &mut self,
        selector: &str,
        failure_reason: &str,
        context: FailureContext,
    ) {
        let now = Utc::now();

        match self.failures.get_mut(selector) {
            Some(existing_failure) => {
                // Update existing failure record
                existing_failure.failure_count += 1;
                existing_failure.last_failure = now;
                existing_failure.failure_reason = failure_reason.to_string();
                existing_failure.url = context.url.clone();
                existing_failure.action_type = context.action_type.clone();

                // Check if we should blacklist this selector
                if self.auto_blacklist
                    && existing_failure.failure_count >= self.blacklist_threshold
                    && !existing_failure.blacklisted
                {
                    existing_failure.blacklisted = true;
                    log::warn!(
                        "Selector '{}' blacklisted after {} failures",
                        selector,
                        existing_failure.failure_count
                    );
                }
            }
            None => {
                // Create new failure record
                let failure = SelectorFailure {
                    selector: selector.to_string(),
                    failure_count: 1,
                    last_failure: now,
                    first_failure: now,
                    failure_reason: failure_reason.to_string(),
                    url: context.url.clone(),
                    action_type: context.action_type.clone(),
                    blacklisted: false,
                    attempted_alternatives: Vec::new(),
                };

                self.failures.insert(selector.to_string(), failure);
                self.stats.unique_failed_selectors += 1;
            }
        }

        self.stats.total_failures += 1;
        self.update_statistics(&context);

        log::debug!(
            "Recorded failure for selector '{}': {} (total: {} failures)",
            selector,
            failure_reason,
            self.get_failure_count(selector)
        );
    }

    /// Check if a selector is blacklisted
    pub fn is_blacklisted(&self, selector: &str) -> bool {
        self.failures
            .get(selector)
            .map(|f| f.blacklisted)
            .unwrap_or(false)
    }

    /// Check if a selector has failed recently
    pub fn has_failed_recently(&self, selector: &str, hours: i64) -> bool {
        if let Some(failure) = self.failures.get(selector) {
            let threshold = Utc::now() - Duration::hours(hours);
            failure.last_failure > threshold
        } else {
            false
        }
    }

    /// Get failure count for a selector
    pub fn get_failure_count(&self, selector: &str) -> u32 {
        self.failures
            .get(selector)
            .map(|f| f.failure_count)
            .unwrap_or(0)
    }

    /// Get failure information for a selector
    pub fn get_failure_info(&self, selector: &str) -> Option<&SelectorFailure> {
        self.failures.get(selector)
    }

    /// Get all blacklisted selectors
    pub fn get_blacklisted_selectors(&self) -> Vec<String> {
        self.failures
            .values()
            .filter(|f| f.blacklisted)
            .map(|f| f.selector.clone())
            .collect()
    }

    /// Get failure statistics
    pub fn get_stats(&self) -> &FailureStats {
        &self.stats
    }

    /// Filter a list of selectors to remove blacklisted ones
    pub fn filter_selectors(&self, selectors: Vec<String>) -> Vec<String> {
        selectors
            .into_iter()
            .filter(|s| !self.is_blacklisted(s))
            .collect()
    }

    /// Get alternative selectors for a failed selector
    pub fn get_alternatives(&self, failed_selector: &str) -> Vec<String> {
        // This is a basic implementation - could be enhanced with ML-based suggestions
        let mut alternatives = Vec::new();

        // If the failed selector was very specific, try more general versions
        if failed_selector.contains("nth-child") || failed_selector.contains(":eq(") {
            // Remove positional selectors
            let base = failed_selector
                .split("nth-child")
                .next()
                .or_else(|| failed_selector.split(":eq(").next())
                .unwrap_or(failed_selector);
            alternatives.push(base.trim_end_matches(' ').to_string());
        }

        // If it was a class selector, try ID or data attributes
        if failed_selector.starts_with('.') {
            // Look for similar elements with IDs or data attributes
            // This would require DOM context, so for now we just suggest common patterns
            alternatives.push(format!("[data-testid*='{}']", &failed_selector[1..]));
            alternatives.push(format!("[aria-label*='{}']", &failed_selector[1..]));
        }

        // If it was an ID selector, try class-based alternatives
        if let Some(id) = failed_selector.strip_prefix('#') {
            alternatives.push(format!(".{}", id));
            alternatives.push(format!("[data-testid='{}']", id));
        }

        // Remove any alternatives that are also blacklisted
        self.filter_selectors(alternatives)
    }

    /// Clean up old failure records
    pub fn cleanup_old_failures(&mut self) {
        let cutoff = Utc::now() - Duration::hours(self.max_age_hours);
        let initial_count = self.failures.len();

        self.failures
            .retain(|_, failure| failure.last_failure > cutoff);

        let removed_count = initial_count - self.failures.len();
        if removed_count > 0 {
            log::info!("Cleaned up {} old failure records", removed_count);
            self.stats.unique_failed_selectors = self.failures.len() as u32;
        }
    }

    /// Generate a summary report of failures for LLM context
    pub fn generate_failure_summary(&self, current_url: &str) -> String {
        let mut summary = String::new();

        // Overall statistics
        summary.push_str(&format!(
            "FAILURE CACHE SUMMARY (Total: {} failures, {} unique selectors)\n\n",
            self.stats.total_failures, self.stats.unique_failed_selectors
        ));

        // Blacklisted selectors
        let blacklisted = self.get_blacklisted_selectors();
        if !blacklisted.is_empty() {
            summary.push_str("🚫 BLACKLISTED SELECTORS (avoid these):\n");
            for selector in blacklisted.iter().take(10) {
                if let Some(failure) = self.failures.get(selector) {
                    summary.push_str(&format!(
                        "  - {} (failed {} times: {})\n",
                        selector, failure.failure_count, failure.failure_reason
                    ));
                }
            }
            summary.push('\n');
        }

        // Recent failures on current URL
        let recent_failures: Vec<_> = self
            .failures
            .values()
            .filter(|f| f.url == current_url && f.last_failure > Utc::now() - Duration::hours(1))
            .collect();

        if !recent_failures.is_empty() {
            summary.push_str("[WARNING] RECENT FAILURES ON THIS PAGE:\n");
            for failure in recent_failures.iter().take(5) {
                summary.push_str(&format!(
                    "  - {} ({} times): {}\n",
                    failure.selector, failure.failure_count, failure.failure_reason
                ));
            }
            summary.push('\n');
        }

        // Most problematic action types
        if !self.stats.problematic_actions.is_empty() {
            summary.push_str("[TARGET] ACTIONS WITH MOST FAILURES:\n");
            for (action, count) in self.stats.problematic_actions.iter().take(3) {
                summary.push_str(&format!("  - {}: {} failures\n", action, count));
            }
            summary.push('\n');
        }

        summary.push_str("[TIP] RECOMMENDATION: Use different selectors or action types to avoid repeated failures.\n");

        summary
    }

    /// Record that alternatives were attempted for a selector
    pub fn record_alternatives_attempted(
        &mut self,
        original_selector: &str,
        alternatives: Vec<String>,
    ) {
        if let Some(failure) = self.failures.get_mut(original_selector) {
            failure.attempted_alternatives.extend(alternatives);
        }
    }

    /// Mark a selector as manually blacklisted
    pub fn blacklist_selector(&mut self, selector: &str, reason: &str) {
        match self.failures.get_mut(selector) {
            Some(failure) => {
                failure.blacklisted = true;
                failure.failure_reason = format!("Manual blacklist: {}", reason);
            }
            None => {
                // Create a new failure record for manual blacklisting
                let failure = SelectorFailure {
                    selector: selector.to_string(),
                    failure_count: self.blacklist_threshold, // Set to threshold to indicate severity
                    last_failure: Utc::now(),
                    first_failure: Utc::now(),
                    failure_reason: format!("Manual blacklist: {}", reason),
                    url: "unknown".to_string(),
                    action_type: "unknown".to_string(),
                    blacklisted: true,
                    attempted_alternatives: Vec::new(),
                };
                self.failures.insert(selector.to_string(), failure);
                self.stats.unique_failed_selectors += 1;
            }
        }

        log::warn!("Manually blacklisted selector '{}': {}", selector, reason);
    }

    /// Remove a selector from the blacklist
    pub fn unblacklist_selector(&mut self, selector: &str) {
        if let Some(failure) = self.failures.get_mut(selector) {
            failure.blacklisted = false;
            log::info!("Removed selector '{}' from blacklist", selector);
        }
    }

    /// Clear all failure records (use with caution)
    pub fn clear_all(&mut self) {
        let count = self.failures.len();
        self.failures.clear();
        self.stats = FailureStats {
            total_failures: 0,
            unique_failed_selectors: 0,
            most_common_failure: None,
            problematic_urls: Vec::new(),
            problematic_actions: Vec::new(),
        };
        log::info!("Cleared {} failure records from cache", count);
    }

    /// Update failure statistics
    fn update_statistics(&mut self, _context: &FailureContext) {
        // Update most common failure reason
        let mut failure_reasons: HashMap<String, u32> = HashMap::new();
        for failure in self.failures.values() {
            *failure_reasons
                .entry(failure.failure_reason.clone())
                .or_insert(0) += failure.failure_count;
        }

        if let Some((reason, _)) = failure_reasons.iter().max_by_key(|(_, count)| *count) {
            self.stats.most_common_failure = Some(reason.clone());
        }

        // Update problematic URLs
        let mut url_failures: HashMap<String, u32> = HashMap::new();
        for failure in self.failures.values() {
            *url_failures.entry(failure.url.clone()).or_insert(0) += failure.failure_count;
        }

        let mut url_vec: Vec<_> = url_failures.into_iter().collect();
        url_vec.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        self.stats.problematic_urls = url_vec.into_iter().take(10).collect();

        // Update problematic actions
        let mut action_failures: HashMap<String, u32> = HashMap::new();
        for failure in self.failures.values() {
            *action_failures
                .entry(failure.action_type.clone())
                .or_insert(0) += failure.failure_count;
        }

        let mut action_vec: Vec<_> = action_failures.into_iter().collect();
        action_vec.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        self.stats.problematic_actions = action_vec.into_iter().take(10).collect();
    }
}

impl Default for FailureCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_context() -> FailureContext {
        FailureContext {
            url: "https://example.com".to_string(),
            page_title: Some("Test Page".to_string()),
            action_type: "click_element".to_string(),
            element_index: Some(1),
            dom_context: Some("<div>test</div>".to_string()),
            goal: Some("Test goal".to_string()),
        }
    }

    #[test]
    fn test_failure_cache_creation() {
        let cache = FailureCache::new();
        assert_eq!(cache.stats.total_failures, 0);
        assert_eq!(cache.stats.unique_failed_selectors, 0);
    }

    #[test]
    fn test_record_failure() {
        let mut cache = FailureCache::new();
        let context = create_test_context();

        cache.record_failure("#test-selector", "Element not found", context);

        assert_eq!(cache.stats.total_failures, 1);
        assert_eq!(cache.stats.unique_failed_selectors, 1);
        assert_eq!(cache.get_failure_count("#test-selector"), 1);
        assert!(!cache.is_blacklisted("#test-selector"));
    }

    #[test]
    fn test_blacklisting() {
        let mut cache = FailureCache::with_settings(24, 2, true); // Blacklist after 2 failures
        let context = create_test_context();

        // First failure
        cache.record_failure("#test-selector", "Element not found", context.clone());
        assert!(!cache.is_blacklisted("#test-selector"));

        // Second failure - should trigger blacklisting
        cache.record_failure("#test-selector", "Element not found", context);
        assert!(cache.is_blacklisted("#test-selector"));

        let blacklisted = cache.get_blacklisted_selectors();
        assert_eq!(blacklisted, vec!["#test-selector"]);
    }

    #[test]
    fn test_filter_selectors() {
        let mut cache = FailureCache::new();
        let _context = create_test_context();

        // Blacklist one selector
        cache.blacklist_selector("#bad-selector", "Test blacklist");

        let selectors = vec![
            "#good-selector".to_string(),
            "#bad-selector".to_string(),
            ".another-good".to_string(),
        ];

        let filtered = cache.filter_selectors(selectors);
        assert_eq!(filtered.len(), 2);
        assert!(!filtered.contains(&"#bad-selector".to_string()));
    }

    #[test]
    fn test_recent_failures() {
        let mut cache = FailureCache::new();
        let context = create_test_context();

        cache.record_failure("#test-selector", "Element not found", context);

        // Should be recent (within 1 hour)
        assert!(cache.has_failed_recently("#test-selector", 1));
        // Should not be recent (within 0 hours = now)
        assert!(!cache.has_failed_recently("#test-selector", 0));
    }

    #[test]
    fn test_alternatives_generation() {
        let cache = FailureCache::new();

        // Test class selector alternatives
        let alternatives = cache.get_alternatives(".failed-class");
        assert!(alternatives.iter().any(|s| s.contains("data-testid")));

        // Test ID selector alternatives
        let alternatives = cache.get_alternatives("#failed-id");
        assert!(alternatives.iter().any(|s| s.starts_with('.')));
    }

    #[test]
    fn test_manual_blacklisting() {
        let mut cache = FailureCache::new();

        cache.blacklist_selector("#manual-blacklist", "Testing manual blacklist");

        assert!(cache.is_blacklisted("#manual-blacklist"));
        assert_eq!(cache.stats.unique_failed_selectors, 1);

        // Test unblacklisting
        cache.unblacklist_selector("#manual-blacklist");
        assert!(!cache.is_blacklisted("#manual-blacklist"));
    }

    #[test]
    fn test_failure_summary() {
        let mut cache = FailureCache::new();
        let context = create_test_context();

        cache.record_failure("#test1", "Not found", context.clone());
        cache.record_failure("#test2", "Not visible", context);

        let summary = cache.generate_failure_summary("https://example.com");
        assert!(summary.contains("FAILURE CACHE SUMMARY"));
        assert!(summary.contains("2 failures"));
    }

    #[test]
    fn test_clear_all() {
        let mut cache = FailureCache::new();
        let context = create_test_context();

        cache.record_failure("#test", "Error", context);
        assert_eq!(cache.stats.total_failures, 1);

        cache.clear_all();
        assert_eq!(cache.stats.total_failures, 0);
        assert_eq!(cache.stats.unique_failed_selectors, 0);
    }
}
