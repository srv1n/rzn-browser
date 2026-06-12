use anyhow::Result;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents a simplified DOM element for LLM context
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DomElement {
    pub tag: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attributes: HashMap<String, String>,
    pub text_content: String,
    pub selector_suggestions: Vec<String>,
    pub frame_id: Option<String>, // NEW: Frame ID for iframe elements
}

/// DOM context optimized for LLM consumption
#[derive(Debug, Serialize, Deserialize)]
pub struct DomContext {
    pub url: String,
    pub title: String,
    pub interactive_elements: Vec<DomElement>,
    pub semantic_groups: HashMap<String, Vec<String>>, // e.g., "forms" -> [selectors]
    pub page_type: String,                             // "search_page", "article", "form", etc.
    pub total_elements: usize,
    pub processed_elements: usize,
    pub iframes: Vec<IframeInfo>, // NEW: Information about iframes on the page
}

/// Information about iframe elements for frame_id routing
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IframeInfo {
    pub frame_id: String,
    pub src: Option<String>,
    pub name: Option<String>,
    pub id: Option<String>,
    pub selector: String,
    pub domain: Option<String>,
}

/// Configuration for DOM processing
#[derive(Debug, Clone)]
pub struct DomProcessorConfig {
    pub max_elements: usize,
    pub max_text_length: usize,
    pub include_hidden: bool,
    pub priority_selectors: Vec<String>,
}

impl Default for DomProcessorConfig {
    fn default() -> Self {
        Self {
            max_elements: 600,
            max_text_length: 100,
            include_hidden: false,
            priority_selectors: vec![
                "input".to_string(),
                "button".to_string(),
                "a[href]".to_string(),
                "[data-testid]".to_string(),
                "[aria-label]".to_string(),
                "form".to_string(),
                "[role]".to_string(),
                "select".to_string(),
                "textarea".to_string(),
                "iframe".to_string(), // NEW: Include iframe elements
            ],
        }
    }
}

pub struct DomProcessor {
    config: DomProcessorConfig,
}

impl DomProcessor {
    pub fn new(config: DomProcessorConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self::new(DomProcessorConfig::default())
    }

    /// Extract relevant DOM elements from HTML string
    pub fn extract_dom_context(&self, html: &str, url: &str) -> Result<DomContext> {
        // Limit HTML processing to first 150KB for performance (optimization #3)
        const MAX_HTML_SIZE: usize = 150_000;
        let truncated_html = if html.len() > MAX_HTML_SIZE {
            // Try to find a good break point (end of tag or closing body)
            if let Some(body_end) = html[..MAX_HTML_SIZE].rfind("</body>") {
                &html[..body_end + 7] // Include </body>
            } else if let Some(tag_end) = html[..MAX_HTML_SIZE].rfind('>') {
                &html[..tag_end + 1] // Include complete tag
            } else {
                &html[..MAX_HTML_SIZE]
            }
        } else {
            html
        };

        let document = Html::parse_document(truncated_html);

        // Extract title
        let title = document
            .select(&Selector::parse("title").unwrap())
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_else(|| "Untitled".to_string());

        // Extract interactive elements with priority-based selection
        let mut interactive_elements = Vec::new();
        let total_elements = self.count_total_elements(&document);

        // Process priority selectors first
        for priority_selector in &self.config.priority_selectors {
            if interactive_elements.len() >= self.config.max_elements {
                break;
            }

            if let Ok(selector) = Selector::parse(priority_selector) {
                for element in document.select(&selector) {
                    if interactive_elements.len() >= self.config.max_elements {
                        break;
                    }

                    if let Some(dom_element) = self.process_element(&element) {
                        // Avoid duplicates
                        if !interactive_elements.iter().any(|e: &DomElement| {
                            e.selector_suggestions
                                .iter()
                                .any(|s| dom_element.selector_suggestions.contains(s))
                        }) {
                            interactive_elements.push(dom_element);
                        }
                    }
                }
            }
        }

        // Extract iframe information
        let iframes = self.extract_iframe_info(&document);

        // Identify semantic groups
        let semantic_groups = self.identify_semantic_groups(&interactive_elements);

        // Determine page type
        let page_type = self.determine_page_type(&interactive_elements, &title, url);

        let processed_elements = interactive_elements.len();

        Ok(DomContext {
            url: url.to_string(),
            title,
            interactive_elements,
            semantic_groups,
            page_type,
            total_elements,
            processed_elements,
            iframes,
        })
    }

    fn count_total_elements(&self, document: &Html) -> usize {
        document.select(&Selector::parse("*").unwrap()).count()
    }

    fn process_element(&self, element: &scraper::ElementRef) -> Option<DomElement> {
        let tag = element.value().name().to_string();

        // Skip if hidden (unless config allows)
        if !self.config.include_hidden && self.is_element_hidden(element) {
            return None;
        }

        let id = element.value().id().map(|s| s.to_string());
        let classes: Vec<String> = element.value().classes().map(|s| s.to_string()).collect();

        // Extract relevant attributes
        let mut attributes = HashMap::new();
        for attr in element.value().attrs() {
            match attr.0 {
                "type" | "name" | "placeholder" | "aria-label" | "data-testid" | "role"
                | "href" => {
                    attributes.insert(attr.0.to_string(), attr.1.to_string());
                }
                _ => {}
            }
        }

        // Extract text content (truncated)
        let text_content = element
            .text()
            .collect::<String>()
            .trim()
            .chars()
            .take(self.config.max_text_length)
            .collect();

        // Generate selector suggestions
        let selector_suggestions = self.generate_selector_suggestions(element);

        // Determine frame_id for iframe elements or elements inside iframes
        let frame_id = self.determine_frame_id(element);

        Some(DomElement {
            tag,
            id,
            classes,
            attributes,
            text_content,
            selector_suggestions,
            frame_id,
        })
    }

    fn is_element_hidden(&self, element: &scraper::ElementRef) -> bool {
        // Check for common hidden patterns
        if let Some(style) = element.value().attr("style") {
            if style.contains("display:none") || style.contains("visibility:hidden") {
                return true;
            }
        }

        if let Some(hidden) = element.value().attr("hidden") {
            return hidden.is_empty() || hidden == "true";
        }

        false
    }

    fn generate_selector_suggestions(&self, element: &scraper::ElementRef) -> Vec<String> {
        let mut suggestions = Vec::new();
        let tag = element.value().name();

        // ID-based selector (highest priority)
        if let Some(id) = element.value().id() {
            suggestions.push(format!("#{}", id));
            suggestions.push(format!("{}#{}", tag, id));
        }

        // Attribute-based selectors
        for attr in element.value().attrs() {
            match attr.0 {
                "data-testid" => {
                    suggestions.push(format!("[data-testid='{}']", attr.1));
                }
                "aria-label" => {
                    suggestions.push(format!("[aria-label='{}']", attr.1));
                }
                "name" => {
                    suggestions.push(format!("{}[name='{}']", tag, attr.1));
                }
                "type" => {
                    suggestions.push(format!("{}[type='{}']", tag, attr.1));
                }
                "role" => {
                    suggestions.push(format!("[role='{}']", attr.1));
                }
                _ => {}
            }
        }

        // Class-based selectors (lower priority due to dynamic classes)
        let classes: Vec<&str> = element.value().classes().collect();
        if !classes.is_empty() {
            // Single class selectors
            for class in &classes {
                // Skip obviously dynamic classes
                if !self.is_dynamic_class(class) {
                    suggestions.push(format!(".{}", class));
                    suggestions.push(format!("{}.{}", tag, class));
                }
            }

            // Multi-class selector (if not too many classes)
            if classes.len() <= 3 {
                let class_chain = classes.join(".");
                suggestions.push(format!(".{}", class_chain));
            }
        }

        // Tag-only selector (fallback)
        suggestions.push(tag.to_string());

        // Remove duplicates and limit suggestions
        suggestions.sort();
        suggestions.dedup();
        suggestions.into_iter().take(5).collect()
    }

    fn is_dynamic_class(&self, class_name: &str) -> bool {
        // Use lazy_static to compile regexes once for performance (optimization #2)
        use once_cell::sync::Lazy;
        static DYNAMIC_PATTERNS: Lazy<Vec<regex::Regex>> = Lazy::new(|| {
            vec![
                regex::Regex::new(r"^[a-z0-9_-]{8,}$").unwrap(), // Long random strings
                regex::Regex::new(r"^css-[a-z0-9]+$").unwrap(),  // CSS-in-JS
                regex::Regex::new(r"^[A-Z][a-zA-Z]*-[a-z0-9]+$").unwrap(), // React styled-components
            ]
        });

        DYNAMIC_PATTERNS
            .iter()
            .any(|pattern| pattern.is_match(class_name))
    }

    fn identify_semantic_groups(&self, elements: &[DomElement]) -> HashMap<String, Vec<String>> {
        let mut groups = HashMap::new();

        let mut forms = Vec::new();
        let mut navigation = Vec::new();
        let mut search = Vec::new();
        let mut buttons = Vec::new();
        let mut inputs = Vec::new();

        for element in elements {
            let primary_selector = element.selector_suggestions.first().unwrap_or(&element.tag);

            match element.tag.as_str() {
                "form" => forms.push(primary_selector.clone()),
                "nav" | "header" | "footer" => navigation.push(primary_selector.clone()),
                "input" => {
                    inputs.push(primary_selector.clone());
                    if element
                        .attributes
                        .get("type")
                        .is_some_and(|t| t == "search")
                        || element
                            .attributes
                            .get("name")
                            .is_some_and(|n| n.contains("search"))
                        || element
                            .attributes
                            .get("placeholder")
                            .is_some_and(|p| p.to_lowercase().contains("search"))
                    {
                        search.push(primary_selector.clone());
                    }
                }
                "button" => buttons.push(primary_selector.clone()),
                _ => {}
            }

            // Check for search-related attributes regardless of tag
            if element
                .attributes
                .iter()
                .any(|(k, v)| k.contains("search") || v.to_lowercase().contains("search"))
                || element.text_content.to_lowercase().contains("search")
            {
                search.push(primary_selector.clone());
            }
        }

        if !forms.is_empty() {
            groups.insert("forms".to_string(), forms);
        }
        if !navigation.is_empty() {
            groups.insert("navigation".to_string(), navigation);
        }
        if !search.is_empty() {
            groups.insert("search".to_string(), search);
        }
        if !buttons.is_empty() {
            groups.insert("buttons".to_string(), buttons);
        }
        if !inputs.is_empty() {
            groups.insert("inputs".to_string(), inputs);
        }

        groups
    }

    fn determine_page_type(&self, elements: &[DomElement], title: &str, url: &str) -> String {
        let title_lower = title.to_lowercase();
        let url_lower = url.to_lowercase();

        // Search page detection
        if title_lower.contains("search")
            || url_lower.contains("search")
            || elements.iter().any(|e| {
                e.attributes
                    .iter()
                    .any(|(k, v)| k.contains("search") || v.to_lowercase().contains("search"))
            })
        {
            return "search_page".to_string();
        }

        // Weather page detection
        if title_lower.contains("weather")
            || url_lower.contains("weather")
            || elements
                .iter()
                .any(|e| e.text_content.to_lowercase().contains("weather"))
        {
            return "weather_page".to_string();
        }

        // Form page detection
        if elements.iter().filter(|e| e.tag == "form").count() > 0 {
            return "form_page".to_string();
        }

        // Article/content page
        if elements.iter().any(|e| e.tag == "article")
            || title_lower.contains("blog")
            || title_lower.contains("article")
        {
            return "article_page".to_string();
        }

        "general_page".to_string()
    }

    /// Create a compact summary for LLM consumption
    pub fn create_llm_summary(&self, context: &DomContext) -> String {
        let mut summary = format!(
            "Page: {} ({})\nType: {}\nElements: {}/{}\n\n",
            context.title,
            context.url,
            context.page_type,
            context.processed_elements,
            context.total_elements
        );

        // Add semantic groups summary
        if !context.semantic_groups.is_empty() {
            summary.push_str("Available interactions:\n");
            for (group, selectors) in &context.semantic_groups {
                summary.push_str(&format!("- {}: {} elements\n", group, selectors.len()));
            }
            summary.push('\n');
        }

        // Add key interactive elements
        summary.push_str("Key elements:\n");
        for (i, element) in context.interactive_elements.iter().take(20).enumerate() {
            let primary_selector = element.selector_suggestions.first().unwrap_or(&element.tag);
            let description = if !element.text_content.is_empty() {
                format!(
                    " ({})",
                    element.text_content.chars().take(30).collect::<String>()
                )
            } else if let Some(label) = element.attributes.get("aria-label") {
                format!(" ({})", label.chars().take(30).collect::<String>())
            } else if let Some(placeholder) = element.attributes.get("placeholder") {
                format!(" ({})", placeholder.chars().take(30).collect::<String>())
            } else {
                String::new()
            };

            summary.push_str(&format!(
                "{}. {} {}{}\n",
                i + 1,
                element.tag.to_uppercase(),
                primary_selector,
                description
            ));
        }

        summary
    }

    /// Extract iframe information from the document
    fn extract_iframe_info(&self, document: &Html) -> Vec<IframeInfo> {
        let mut iframes = Vec::new();

        if let Ok(iframe_selector) = Selector::parse("iframe") {
            for (index, iframe) in document.select(&iframe_selector).enumerate() {
                let frame_id = format!("frame_{}", index); // Generate frame ID
                let src = iframe.value().attr("src").map(|s| s.to_string());
                let name = iframe.value().attr("name").map(|s| s.to_string());
                let id = iframe.value().id().map(|s| s.to_string());

                // Generate selector for this iframe
                let selector = if let Some(ref iframe_id) = id {
                    format!("iframe#{}", iframe_id)
                } else if let Some(ref iframe_name) = name {
                    format!("iframe[name='{}']", iframe_name)
                } else {
                    format!("iframe:nth-of-type({})", index + 1)
                };

                // Extract domain from src if available
                let domain = src.as_ref().and_then(|s| {
                    if let Ok(url) = url::Url::parse(s) {
                        Some(url.host_str().unwrap_or("").to_string())
                    } else {
                        None
                    }
                });

                iframes.push(IframeInfo {
                    frame_id,
                    src,
                    name,
                    id,
                    selector,
                    domain,
                });
            }
        }

        iframes
    }

    /// Determine frame_id for an element (if it's an iframe or inside one)
    fn determine_frame_id(&self, element: &scraper::ElementRef) -> Option<String> {
        // For now, we can only detect iframe elements themselves
        // Browser-side frame detection for nested elements would need the real DOM API
        if element.value().name() == "iframe" {
            // Generate a frame ID based on element attributes
            if let Some(id) = element.value().id() {
                Some(format!("iframe_{}", id))
            } else if let Some(name) = element.value().attr("name") {
                Some(format!("iframe_{}", name))
            } else {
                // Would need to determine index in browser context
                Some("iframe_0".to_string()) // Default fallback
            }
        } else {
            // For elements inside iframes, we'd need the actual browser DOM API
            // This is a limitation of the server-side HTML parsing approach
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_dom_processing() {
        let html = r#"
            <html>
                <head><title>Test Page</title></head>
                <body>
                    <form>
                        <input type="text" name="search" placeholder="Search..." />
                        <button type="submit">Search</button>
                    </form>
                    <div class="content">Some content</div>
                </body>
            </html>
        "#;

        let processor = DomProcessor::with_defaults();
        let context = processor
            .extract_dom_context(html, "https://example.com")
            .unwrap();

        assert_eq!(context.title, "Test Page");
        assert_eq!(context.page_type, "search_page");
        assert!(context.semantic_groups.contains_key("search"));
        assert!(!context.interactive_elements.is_empty());
    }

    #[test]
    fn test_selector_generation() {
        let html = r#"
            <input id="search-input" type="text" data-testid="search-field" class="search-box" />
        "#;

        let processor = DomProcessor::with_defaults();
        let context = processor
            .extract_dom_context(html, "https://example.com")
            .unwrap();

        let element = &context.interactive_elements[0];
        assert!(element
            .selector_suggestions
            .contains(&"#search-input".to_string()));
        assert!(element
            .selector_suggestions
            .contains(&"[data-testid='search-field']".to_string()));
    }
}
