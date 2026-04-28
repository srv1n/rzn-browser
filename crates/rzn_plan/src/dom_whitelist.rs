use scraper::{Element, ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Interactive element with assigned index for whitelist-based planning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhitelistElement {
    /// Unique index assigned to this element
    pub index: u32,
    /// Element tag name (div, button, input, etc.)
    pub tag: String,
    /// Unique selector for this specific element
    pub selector: String,
    /// Alternative selectors that could target this element
    pub alt_selectors: Vec<String>,
    /// Element text content (truncated)
    pub text: String,
    /// Element attributes (filtered to useful ones)
    pub attributes: HashMap<String, String>,
    /// Element type classification
    pub element_type: ElementType,
    /// Confidence score for automation suitability (0.0 - 1.0)
    pub confidence: f32,
    /// Whether this element is currently visible
    pub visible: bool,
    /// Parent element index (if any)
    pub parent_index: Option<u32>,
    /// Child element indexes
    pub child_indexes: Vec<u32>,
}

/// Classification of element types for better LLM understanding
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ElementType {
    /// Clickable button or button-like element
    Button,
    /// Text input field
    TextInput,
    /// Select dropdown
    Select,
    /// Checkbox or radio button
    Checkbox,
    /// Link or anchor element
    Link,
    /// Form element
    Form,
    /// Navigation element
    Navigation,
    /// Content container
    Container,
    /// List or grid item
    ListItem,
    /// Image element
    Image,
    /// Other interactive element
    Other,
}

/// DOM whitelist system for index-based element targeting
#[derive(Debug)]
pub struct DomWhitelist {
    /// Map of element indexes to elements
    elements: HashMap<u32, WhitelistElement>,
    /// Next available index
    next_index: u32,
    /// CSS selectors for finding interactive elements
    interactive_selectors: Vec<String>,
    /// Maximum number of elements to index
    max_elements: usize,
    /// Minimum confidence threshold for inclusion
    min_confidence: f32,
}

impl DomWhitelist {
    /// Create new DOM whitelist with default settings
    pub fn new() -> Self {
        Self {
            elements: HashMap::new(),
            next_index: 1, // Start from 1 (0 reserved for "no element")
            interactive_selectors: Self::default_interactive_selectors(),
            max_elements: 200,   // Limit to prevent prompt bloat
            min_confidence: 0.3, // Only include reasonably confident elements
        }
    }

    /// Create DOM whitelist with custom settings
    pub fn with_settings(max_elements: usize, min_confidence: f32) -> Self {
        Self {
            elements: HashMap::new(),
            next_index: 1,
            interactive_selectors: Self::default_interactive_selectors(),
            max_elements,
            min_confidence,
        }
    }

    /// Extract and index interactive elements from HTML
    pub fn extract_from_html(
        &mut self,
        html: &str,
        base_url: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let document = Html::parse_document(html);
        self.elements.clear();
        self.next_index = 1;

        // Find all potentially interactive elements
        let mut candidates = Vec::new();

        for selector_str in &self.interactive_selectors {
            if let Ok(selector) = Selector::parse(selector_str) {
                for element in document.select(&selector) {
                    candidates.push(element);
                }
            }
        }

        // Remove duplicates and sort by relevance
        let mut unique_elements = Vec::new();
        let mut seen_elements = HashSet::new();

        for element in candidates {
            let element_id = self.generate_element_id(element);
            if !seen_elements.contains(&element_id) {
                seen_elements.insert(element_id);
                unique_elements.push(element);
            }
        }

        // Score and filter elements
        let mut scored_elements = Vec::new();
        for element in unique_elements {
            if let Some(whitelist_element) = self.create_whitelist_element(element, base_url) {
                if whitelist_element.confidence >= self.min_confidence {
                    scored_elements.push(whitelist_element);
                }
            }
        }

        // Sort by confidence (highest first) and limit count
        scored_elements.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored_elements.truncate(self.max_elements);

        // Assign indexes and store elements
        for mut element in scored_elements {
            element.index = self.next_index;
            self.elements.insert(self.next_index, element);
            self.next_index += 1;
        }

        // Build parent-child relationships
        self.build_relationships();

        Ok(())
    }

    /// Get element by index
    pub fn get_element(&self, index: u32) -> Option<&WhitelistElement> {
        self.elements.get(&index)
    }

    /// Get all elements sorted by index
    pub fn get_all_elements(&self) -> Vec<&WhitelistElement> {
        let mut elements: Vec<_> = self.elements.values().collect();
        elements.sort_by_key(|e| e.index);
        elements
    }

    /// Get elements by type
    pub fn get_elements_by_type(&self, element_type: ElementType) -> Vec<&WhitelistElement> {
        self.elements
            .values()
            .filter(|e| e.element_type == element_type)
            .collect()
    }

    /// Convert index back to selector for execution
    pub fn index_to_selector(&self, index: u32) -> Option<String> {
        self.elements.get(&index).map(|e| e.selector.clone())
    }

    /// Get element count
    pub fn element_count(&self) -> usize {
        self.elements.len()
    }

    /// Generate compact summary for LLM prompts
    pub fn generate_summary(&self) -> String {
        // Use simplified representation if available
        if let Ok(simplified) = self.generate_simplified_summary() {
            return simplified;
        }

        // Fallback to original implementation
        self.generate_verbose_summary()
    }

    /// Generate simplified summary optimized for LLM consumption
    fn generate_simplified_summary(&self) -> Result<String, Box<dyn std::error::Error>> {
        use crate::dom_representation::simplify_for_llm;

        // Convert HashMap<u32, WhitelistElement> to HashMap<usize, WhitelistElement>
        let elements_usize: HashMap<usize, WhitelistElement> = self
            .elements
            .iter()
            .map(|(k, v)| (*k as usize, v.clone()))
            .collect();

        let simplified = simplify_for_llm(&elements_usize);
        let mut summary = String::new();
        summary.push_str("ELEMENTS (use index to interact):\n\n");

        // Group by rough location for better spatial understanding
        let mut by_location: HashMap<&str, Vec<(usize, &String)>> = HashMap::new();
        for (index, desc) in &simplified {
            if desc.contains("@top") {
                by_location.entry("Top").or_default().push((*index, desc));
            } else if desc.contains("@bottom") {
                by_location
                    .entry("Bottom")
                    .or_default()
                    .push((*index, desc));
            } else {
                by_location
                    .entry("Middle")
                    .or_default()
                    .push((*index, desc));
            }
        }

        // Output by location
        for (location, mut elements) in by_location {
            if !elements.is_empty() {
                summary.push_str(&format!("{} area:\n", location));
                elements.sort_by_key(|(idx, _)| *idx);
                for (_, desc) in elements.iter().take(10) {
                    // Limit per area
                    summary.push_str(&format!("  {}\n", desc));
                }
                summary.push('\n');
            }
        }

        Ok(summary)
    }

    /// Original verbose summary (fallback)
    fn generate_verbose_summary(&self) -> String {
        let mut summary = String::new();
        summary.push_str("INTERACTIVE ELEMENTS (index-based targeting):\n\n");

        let elements = self.get_all_elements();

        // Group by type for better organization
        let mut by_type: HashMap<ElementType, Vec<&WhitelistElement>> = HashMap::new();
        for element in &elements {
            by_type
                .entry(element.element_type.clone())
                .or_default()
                .push(element);
        }

        // Display elements by type
        for (element_type, type_elements) in by_type {
            if type_elements.is_empty() {
                continue;
            }

            summary.push_str(&format!(
                "\n{}:\n",
                Self::type_to_display_name(&element_type)
            ));

            for element in type_elements.iter().take(20) {
                // Limit per type
                let text_preview = if element.text.len() > 40 {
                    format!("{}...", &element.text[..37])
                } else {
                    element.text.clone()
                };

                let confidence_indicator = if element.confidence >= 0.8 {
                    "" // High confidence
                } else if element.confidence >= 0.6 {
                    "" // Medium confidence
                } else {
                    "" // Low confidence
                };

                summary.push_str(&format!(
                    "  [{}] {} {} \"{}\" {}\n",
                    element.index,
                    confidence_indicator,
                    element.tag.to_uppercase(),
                    text_preview,
                    if element.visible { "" } else { "(hidden)" }
                ));

                // Show key attributes
                let key_attrs: Vec<String> = element
                    .attributes
                    .iter()
                    .filter(|(k, _)| {
                        ["id", "name", "class", "data-testid", "aria-label"].contains(&k.as_str())
                    })
                    .map(|(k, v)| {
                        if v.len() > 20 {
                            format!("{}={}...", k, &v[..17])
                        } else {
                            format!("{}={}", k, v)
                        }
                    })
                    .collect();

                if !key_attrs.is_empty() {
                    summary.push_str(&format!("      {}\n", key_attrs.join(" ")));
                }
            }
        }

        summary.push_str(&format!(
            "\nTotal: {} elements indexed\n",
            self.element_count()
        ));
        summary.push_str("Use [index] to target elements in actions.\n");

        summary
    }

    /// Default interactive element selectors
    fn default_interactive_selectors() -> Vec<String> {
        vec![
            // Form elements
            "input".to_string(),
            "button".to_string(),
            "select".to_string(),
            "textarea".to_string(),
            "option".to_string(),
            // Interactive elements
            "a[href]".to_string(),
            "[onclick]".to_string(),
            "[role='button']".to_string(),
            "[role='link']".to_string(),
            "[role='tab']".to_string(),
            "[role='menuitem']".to_string(),
            // Common interactive patterns
            "[data-testid]".to_string(),
            "[data-cy]".to_string(),
            "[data-test]".to_string(),
            "[jsname]".to_string(),   // Google-specific
            "[data-ved]".to_string(), // Google-specific
            // Containers that are often clickable
            "div[class*='button']".to_string(),
            "div[class*='click']".to_string(),
            "span[class*='button']".to_string(),
            // Lists and items
            "li".to_string(),
            "[role='listitem']".to_string(),
            "[role='option']".to_string(),
            // Images that might be clickable
            "img[onclick]".to_string(),
            "img[data-src]".to_string(),
            // Navigation elements
            "nav a".to_string(),
            "[role='navigation'] a".to_string(),
            // Search-specific
            "[name='q']".to_string(),
            "[name='search']".to_string(),
            "input[type='search']".to_string(),
        ]
    }

    /// Generate unique ID for element deduplication
    fn generate_element_id(&self, element: ElementRef) -> String {
        // Use multiple attributes to create unique ID
        let tag = element.value().name();
        let id = element.value().attr("id").unwrap_or("");
        let class = element.value().attr("class").unwrap_or("");
        let name = element.value().attr("name").unwrap_or("");
        let text = element.text().collect::<Vec<_>>().join(" ");
        let text_truncated = if text.len() > 50 { &text[..50] } else { &text };

        format!("{}|{}|{}|{}|{}", tag, id, class, name, text_truncated)
    }

    /// Create whitelist element from HTML element
    fn create_whitelist_element(
        &self,
        element: ElementRef,
        base_url: &str,
    ) -> Option<WhitelistElement> {
        let tag = element.value().name().to_string();
        let text = element
            .text()
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();

        // Extract attributes
        let mut attributes = HashMap::new();
        for attr in element.value().attrs() {
            // Only keep useful attributes to reduce noise
            if Self::is_useful_attribute(attr.0) {
                attributes.insert(attr.0.to_string(), attr.1.to_string());
            }
        }

        // Generate selectors
        let selector = self.generate_primary_selector(element);
        let alt_selectors = self.generate_alternative_selectors(element);

        // Classify element type
        let element_type = self.classify_element(element);

        // Calculate confidence score
        let confidence = self.calculate_confidence(element, &attributes, &text);

        // Check visibility (basic heuristic)
        let visible = self.is_likely_visible(element, &attributes);

        Some(WhitelistElement {
            index: 0, // Will be assigned later
            tag,
            selector,
            alt_selectors,
            text: Self::truncate_text(&text, 100),
            attributes,
            element_type,
            confidence,
            visible,
            parent_index: None,        // Will be calculated later
            child_indexes: Vec::new(), // Will be calculated later
        })
    }

    /// Generate primary selector for element
    fn generate_primary_selector(&self, element: ElementRef) -> String {
        // Priority order for selector generation

        // 1. ID selector (highest priority)
        if let Some(id) = element.value().attr("id") {
            if !id.is_empty() && Self::is_valid_id(id) {
                return format!("#{}", id);
            }
        }

        // 2. Data attributes (test IDs, etc.)
        for attr in ["data-testid", "data-cy", "data-test", "jsname"] {
            if let Some(value) = element.value().attr(attr) {
                if !value.is_empty() {
                    return format!("[{}='{}']", attr, value);
                }
            }
        }

        // 3. Name attribute (for form elements)
        if let Some(name) = element.value().attr("name") {
            if !name.is_empty() {
                return format!("[name='{}']", name);
            }
        }

        // 4. Aria-label
        if let Some(aria_label) = element.value().attr("aria-label") {
            if !aria_label.is_empty() {
                return format!("[aria-label='{}']", aria_label);
            }
        }

        // 5. Class-based selector (be careful with dynamic classes)
        if let Some(class) = element.value().attr("class") {
            let classes: Vec<&str> = class.split_whitespace().collect();
            // Look for stable-looking classes
            for cls in &classes {
                if Self::is_stable_class(cls) {
                    return format!(".{}", cls);
                }
            }
        }

        // 6. Fallback to tag + text content
        let text = element.text().collect::<Vec<_>>().join(" ");
        if !text.is_empty() && text.len() < 50 {
            return format!(
                "{}:contains('{}')",
                element.value().name(),
                text.replace("'", "\\'")
            );
        }

        // 7. Last resort: tag selector (not reliable)
        element.value().name().to_string()
    }

    /// Generate alternative selectors
    fn generate_alternative_selectors(&self, element: ElementRef) -> Vec<String> {
        let mut selectors = Vec::new();

        // Add class-based selectors
        if let Some(class) = element.value().attr("class") {
            let classes: Vec<&str> = class.split_whitespace().collect();
            for cls in &classes {
                if !cls.is_empty() && cls.len() > 2 {
                    selectors.push(format!(".{}", cls));
                }
            }
        }

        // Add attribute-based selectors
        for attr in element.value().attrs() {
            if Self::is_useful_for_selection(attr.0) && !attr.1.is_empty() {
                selectors.push(format!("[{}='{}']", attr.0, attr.1));
            }
        }

        // Add parent-child selectors (for context)
        if let Some(parent) = element.parent_element() {
            if let Some(parent_id) = parent.value().attr("id") {
                if !parent_id.is_empty() {
                    selectors.push(format!("#{} {}", parent_id, element.value().name()));
                }
            }
        }

        selectors
    }

    /// Classify element by type
    fn classify_element(&self, element: ElementRef) -> ElementType {
        let tag = element.value().name();
        let type_attr = element.value().attr("type").unwrap_or("");
        let role = element.value().attr("role").unwrap_or("");

        match tag {
            "button" => ElementType::Button,
            "input" => match type_attr {
                "text" | "email" | "password" | "search" | "url" | "tel" => ElementType::TextInput,
                "checkbox" | "radio" => ElementType::Checkbox,
                "submit" => ElementType::Button,
                _ => ElementType::TextInput,
            },
            "textarea" => ElementType::TextInput,
            "select" => ElementType::Select,
            "a" => ElementType::Link,
            "form" => ElementType::Form,
            "nav" => ElementType::Navigation,
            "li" => ElementType::ListItem,
            "img" => ElementType::Image,
            _ => {
                // Check role attribute
                match role {
                    "button" => ElementType::Button,
                    "link" => ElementType::Link,
                    "textbox" => ElementType::TextInput,
                    "listitem" => ElementType::ListItem,
                    "navigation" => ElementType::Navigation,
                    _ => {
                        // Check for interactive indicators
                        if element.value().attr("onclick").is_some()
                            || element.value().attr("href").is_some()
                        {
                            ElementType::Button
                        } else {
                            ElementType::Container
                        }
                    }
                }
            }
        }
    }

    /// Calculate confidence score for automation
    fn calculate_confidence(
        &self,
        element: ElementRef,
        attributes: &HashMap<String, String>,
        text: &str,
    ) -> f32 {
        let mut score: f32 = 0.5; // Base score

        // ID increases confidence significantly
        if attributes.contains_key("id") {
            score += 0.3;
        }

        // Test attributes are excellent for automation
        if attributes.contains_key("data-testid")
            || attributes.contains_key("data-cy")
            || attributes.contains_key("data-test")
        {
            score += 0.4;
        }

        // Name attributes are good for forms
        if attributes.contains_key("name") {
            score += 0.2;
        }

        // Aria labels provide stability
        if attributes.contains_key("aria-label") {
            score += 0.15;
        }

        // Semantic HTML elements are more reliable
        let tag = element.value().name();
        match tag {
            "button" | "input" | "select" | "textarea" | "a" => score += 0.2,
            _ => {}
        }

        // Visible text content is good for targeting
        if !text.is_empty() && text.len() > 2 && text.len() < 100 {
            score += 0.1;
        }

        // Interactive attributes
        if element.value().attr("onclick").is_some()
            || element.value().attr("href").is_some()
            || element.value().attr("role").is_some()
        {
            score += 0.1;
        }

        // Penalty for generic/dynamic classes
        if let Some(class) = attributes.get("class") {
            let classes: Vec<&str> = class.split_whitespace().collect();
            for cls in &classes {
                if Self::is_dynamic_class(cls) {
                    score -= 0.05;
                }
            }
        }

        // Cap at 1.0
        score.min(1.0)
    }

    /// Check if element is likely visible
    fn is_likely_visible(&self, element: ElementRef, attributes: &HashMap<String, String>) -> bool {
        // Basic visibility heuristics

        // Check style attribute for display/visibility
        if let Some(style) = attributes.get("style") {
            if style.contains("display:none")
                || style.contains("display: none")
                || style.contains("visibility:hidden")
                || style.contains("visibility: hidden")
            {
                return false;
            }
        }

        // Check for hidden attribute
        if attributes.contains_key("hidden") {
            return false;
        }

        // Check for screen reader only classes (common patterns)
        if let Some(class) = attributes.get("class") {
            if class.contains("sr-only")
                || class.contains("screen-reader")
                || class.contains("visually-hidden")
            {
                return false;
            }
        }

        true // Assume visible by default
    }

    /// Build parent-child relationships
    fn build_relationships(&mut self) {
        // This is a simplified version - full implementation would require
        // maintaining the DOM tree structure during parsing
        // For now, we'll leave relationships empty
    }

    /// Check if attribute is useful for automation
    fn is_useful_attribute(attr_name: &str) -> bool {
        matches!(
            attr_name,
            "id" | "class"
                | "name"
                | "type"
                | "role"
                | "aria-label"
                | "aria-labelledby"
                | "data-testid"
                | "data-cy"
                | "data-test"
                | "jsname"
                | "data-ved"
                | "placeholder"
                | "value"
                | "href"
                | "src"
                | "alt"
                | "title"
        )
    }

    /// Check if attribute is useful for generating selectors
    fn is_useful_for_selection(attr_name: &str) -> bool {
        matches!(
            attr_name,
            "data-testid"
                | "data-cy"
                | "data-test"
                | "jsname"
                | "data-ved"
                | "role"
                | "type"
                | "name"
                | "placeholder"
        )
    }

    /// Check if ID is valid for CSS selector
    fn is_valid_id(id: &str) -> bool {
        !id.is_empty() && !id.starts_with(char::is_numeric) && !id.contains(' ') && id.len() < 50
        // Avoid extremely long IDs
    }

    /// Check if class looks stable (not dynamically generated)
    fn is_stable_class(class: &str) -> bool {
        // Look for patterns that suggest stable classes
        class.len() > 3 && // Not too short
        !class.chars().all(|c| c.is_numeric()) && // Not all numbers
        !class.contains("_") || class.contains("btn") || class.contains("link") || // Allow some patterns
        class.contains("nav") || class.contains("search") || class.contains("form")
    }

    /// Check if class looks dynamically generated
    fn is_dynamic_class(class: &str) -> bool {
        // Common patterns for dynamic/generated classes
        class.len() > 20 || // Very long classes are often generated
        class.chars().filter(|c| c.is_numeric()).count() > class.len() / 2 || // Mostly numbers
        class.contains("css-") || // CSS-in-JS
        class.starts_with("_") // Webpack/build tool generated
    }

    /// Convert element type to display name
    fn type_to_display_name(element_type: &ElementType) -> &'static str {
        match element_type {
            ElementType::Button => "BUTTONS",
            ElementType::TextInput => "TEXT INPUTS",
            ElementType::Select => "DROPDOWNS",
            ElementType::Checkbox => "CHECKBOXES",
            ElementType::Link => "LINKS",
            ElementType::Form => "FORMS",
            ElementType::Navigation => "NAVIGATION",
            ElementType::Container => "CONTAINERS",
            ElementType::ListItem => "LIST ITEMS",
            ElementType::Image => "IMAGES",
            ElementType::Other => "OTHER",
        }
    }

    /// Truncate text to specified length
    fn truncate_text(text: &str, max_len: usize) -> String {
        if text.len() <= max_len {
            text.to_string()
        } else {
            format!("{}...", &text[..max_len.saturating_sub(3)])
        }
    }
}

impl Default for DomWhitelist {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dom_whitelist_creation() {
        let whitelist = DomWhitelist::new();
        assert_eq!(whitelist.element_count(), 0);
        assert_eq!(whitelist.next_index, 1);
    }

    #[test]
    fn test_basic_html_extraction() {
        let mut whitelist = DomWhitelist::new();
        let html = r#"
            <html>
                <body>
                    <button id="test-btn">Click me</button>
                    <input type="text" name="search" placeholder="Search..."/>
                    <a href="/link">Link</a>
                </body>
            </html>
        "#;

        whitelist
            .extract_from_html(html, "https://example.com")
            .unwrap();
        assert!(whitelist.element_count() > 0);

        // Should find the button
        let elements = whitelist.get_elements_by_type(ElementType::Button);
        assert!(!elements.is_empty());
    }

    #[test]
    fn test_selector_generation() {
        let mut whitelist = DomWhitelist::new();
        let html = r#"
            <button id="unique-id">Test</button>
            <input data-testid="search-input" name="q"/>
            <div class="generic-class">Content</div>
        "#;

        whitelist
            .extract_from_html(html, "https://example.com")
            .unwrap();

        let elements = whitelist.get_all_elements();

        // Check that ID selector is preferred
        let button_element = elements.iter().find(|e| e.tag == "button").unwrap();
        assert_eq!(button_element.selector, "#unique-id");

        // Check that data-testid is used
        let input_element = elements.iter().find(|e| e.tag == "input").unwrap();
        assert_eq!(input_element.selector, "[data-testid='search-input']");
    }

    #[test]
    fn test_confidence_scoring() {
        let mut whitelist = DomWhitelist::new();
        let html = r#"
            <button id="good-btn" data-testid="test">High confidence</button>
            <div class="css-abc123-def456">Low confidence</div>
            <input name="email" type="email">Medium confidence</input>
        "#;

        whitelist
            .extract_from_html(html, "https://example.com")
            .unwrap();

        let elements = whitelist.get_all_elements();

        // Button with ID and testid should have high confidence
        let button = elements.iter().find(|e| e.tag == "button").unwrap();
        assert!(button.confidence > 0.8);

        // Input with name should have medium confidence
        let input = elements.iter().find(|e| e.tag == "input").unwrap();
        assert!(input.confidence >= 0.6);
    }

    #[test]
    fn test_index_to_selector() {
        let mut whitelist = DomWhitelist::new();
        let html = r#"<button id="test">Click</button>"#;

        whitelist
            .extract_from_html(html, "https://example.com")
            .unwrap();

        let elements = whitelist.get_all_elements();
        let element = elements.first().unwrap();
        let index = element.index;

        assert_eq!(
            whitelist.index_to_selector(index),
            Some("#test".to_string())
        );
        assert_eq!(whitelist.index_to_selector(999), None);
    }

    #[test]
    fn test_summary_generation() {
        let mut whitelist = DomWhitelist::new();
        let html = r#"
            <button id="btn1">Button 1</button>
            <input type="text" name="search" placeholder="Search"/>
            <a href="/link">Link text</a>
        "#;

        whitelist
            .extract_from_html(html, "https://example.com")
            .unwrap();

        let summary = whitelist.generate_summary();
        // Accept either simplified or verbose summary formats
        assert!(
            summary.contains("INTERACTIVE ELEMENTS")
                || summary.contains("ELEMENTS (use index to interact)")
        );
        // Either classic index rendering or grouped area sections are present
        assert!(
            summary.contains("[1]")
                || summary.contains("[2]")
                || summary.contains("[3]")
                || summary.contains("Top area:")
                || summary.contains("Middle area:")
                || summary.contains("Bottom area:")
        );
    }
}
