use crate::dom_processor::{DomContext, DomElement};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::Url;

/// Estimate token count for DOM content (rough approximation)
pub fn estimate_tokens(dom_context: &DomContext) -> usize {
    // Rough estimate: 1 token per 4 characters
    let mut total_chars = 0;

    // Count URL and title
    total_chars += dom_context.url.len() + dom_context.title.len();

    // Count interactive elements
    for element in &dom_context.interactive_elements {
        total_chars += element.tag.len();
        total_chars += element.text_content.len();
        total_chars += element.selector_suggestions.join(" ").len();
        total_chars += element
            .attributes
            .iter()
            .map(|(k, v)| k.len() + v.len())
            .sum::<usize>();
    }

    // Count semantic groups
    for (group, selectors) in &dom_context.semantic_groups {
        total_chars += group.len() + selectors.join(" ").len();
    }

    // Rough token estimate
    total_chars / 4
}

/// Compact snapshot element for LLM consumption (matches extension format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactElement {
    /// Encoded ID for element selection (e.g., "btn_1", "inp_2")
    pub encoded_id: String,

    /// Element tag name
    pub tag: String,

    /// Primary selector for element
    pub selector: String,

    /// Text content (truncated)
    pub text: Option<String>,

    /// Element role from AX tree
    pub role: Option<String>,

    /// Element name from AX tree
    pub name: Option<String>,

    /// Distance from viewport center (for sorting)
    pub viewport_distance: f32,

    /// Key attributes
    pub attrs: Option<HashMap<String, String>>,

    /// Frame information for cross-origin awareness
    pub frame: Option<String>,

    /// Element type (button, input, link, etc)
    pub element_type: Option<String>,

    /// Interaction hints
    pub actions: Option<Vec<String>>,
}

/// Compact frame context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactFrame {
    pub id: String,
    pub url: String,
    pub origin: String,
    pub accessible: bool,
    pub element_count: usize,
}

/// Compact snapshot for LLM consumption (2-8KB)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactSnapshot {
    pub url: String,
    pub title: String,
    pub viewport: CompactViewport,
    pub elements: Vec<CompactElement>,
    pub frames: Vec<CompactFrame>,
    pub size_kb: f32,
    pub timestamp: u64,
    pub compression_level: String,
    pub memory_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactViewport {
    pub width: u32,
    pub height: u32,
    pub scroll_x: u32,
    pub scroll_y: u32,
}

/// Legacy compressed DOM representation
#[derive(Debug)]
pub struct CompressedDom {
    pub url: String,
    pub title: String,
    pub page_type: String,
    pub key_elements: Vec<CompressedElement>,
    pub action_hints: Vec<String>,
    pub estimated_tokens: usize,
}

#[derive(Debug)]
pub struct CompressedElement {
    pub tag: String,
    pub selector: String,
    pub text: String,
    pub action_type: String,
}

/// Aggressively compress DOM when approaching token limits
pub fn compress_dom_aggressive(dom_context: &DomContext, max_tokens: usize) -> CompressedDom {
    let current_tokens = estimate_tokens(dom_context);

    if current_tokens > 900 || current_tokens > max_tokens {
        // Aggressive compression mode
        compress_for_minimal_tokens(dom_context)
    } else {
        // Normal compression
        compress_normal(dom_context)
    }
}

/// Normal compression - preserve important details
fn compress_normal(dom_context: &DomContext) -> CompressedDom {
    let mut key_elements = Vec::new();
    let mut action_hints = Vec::new();

    // Process interactive elements with priority
    let priority_tags = ["input", "button", "a", "form", "select", "textarea"];

    // First pass: high priority interactive elements
    for element in &dom_context.interactive_elements {
        if priority_tags.contains(&element.tag.as_str()) {
            if let Some(compressed) = compress_element_normal(element) {
                key_elements.push(compressed);

                // Generate action hints
                if element.tag == "input" {
                    if let Some(input_type) = element.attributes.get("type") {
                        match input_type.as_str() {
                            "search" | "text" => {
                                action_hints.push("Can use fill_input_field".to_string())
                            }
                            "submit" => action_hints
                                .push("Can use click_element or submit_input".to_string()),
                            _ => {}
                        }
                    }
                } else if element.tag == "button" {
                    action_hints.push("Can use click_element".to_string());
                }
            }
        }
    }

    // Add semantic group hints
    if dom_context.semantic_groups.contains_key("search") {
        action_hints
            .push("Search functionality detected - look for search input and button".to_string());
    }
    if dom_context.semantic_groups.contains_key("forms") {
        action_hints.push("Forms detected - can fill and submit".to_string());
    }

    let estimated_tokens = estimate_compressed_tokens(&key_elements, &action_hints);

    CompressedDom {
        url: dom_context.url.clone(),
        title: truncate_text(&dom_context.title, 50),
        page_type: dom_context.page_type.clone(),
        key_elements,
        action_hints,
        estimated_tokens,
    }
}

/// Aggressive compression - minimal tokens only
fn compress_for_minimal_tokens(dom_context: &DomContext) -> CompressedDom {
    let mut key_elements = Vec::new();
    let mut action_hints = Vec::new();

    // Only keep top 10 most important interactive elements
    let mut scored_elements: Vec<(f32, &DomElement)> = dom_context
        .interactive_elements
        .iter()
        .map(|elem| (score_element_importance(elem), elem))
        .collect();

    scored_elements.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

    for (_, element) in scored_elements.iter().take(10) {
        if let Some(compressed) = compress_element_aggressive(element) {
            key_elements.push(compressed);
        }
    }

    // Minimal action hints
    if dom_context.page_type == "search_page" {
        action_hints.push("Search: find input, fill, submit".to_string());
    }

    let estimated_tokens = estimate_compressed_tokens(&key_elements, &action_hints);

    CompressedDom {
        url: truncate_text(&dom_context.url, 30),
        title: truncate_text(&dom_context.title, 20),
        page_type: dom_context.page_type.clone(),
        key_elements,
        action_hints,
        estimated_tokens,
    }
}

/// Score element importance for prioritization
fn score_element_importance(element: &DomElement) -> f32 {
    let mut score = 0.0;

    // Tag importance
    match element.tag.as_str() {
        "input" => score += 10.0,
        "button" => score += 8.0,
        "a" => score += 5.0,
        "form" => score += 7.0,
        "select" => score += 6.0,
        _ => score += 1.0,
    }

    // Attribute bonuses
    if element.attributes.contains_key("data-testid") {
        score += 5.0;
    }
    if element.attributes.contains_key("aria-label") {
        score += 3.0;
    }
    if element.id.is_some() {
        score += 4.0;
    }

    // Search-related bonus
    let search_keywords = ["search", "query", "find", "submit"];
    for keyword in &search_keywords {
        if element.text_content.to_lowercase().contains(keyword)
            || element
                .attributes
                .values()
                .any(|v| v.to_lowercase().contains(keyword))
        {
            score += 10.0;
            break;
        }
    }

    score
}

/// Compress element with normal detail level
fn compress_element_normal(element: &DomElement) -> Option<CompressedElement> {
    let selector = element.selector_suggestions.first()?.clone();
    let text = truncate_text(&element.text_content, 30);

    let action_type = match element.tag.as_str() {
        "input" => "fill",
        "button" => "click",
        "a" => "click",
        "select" => "select",
        _ => "interact",
    };

    Some(CompressedElement {
        tag: element.tag.clone(),
        selector,
        text,
        action_type: action_type.to_string(),
    })
}

/// Compress element aggressively - minimal info only
fn compress_element_aggressive(element: &DomElement) -> Option<CompressedElement> {
    let selector = element.selector_suggestions.first()?.clone();
    let text = truncate_text(&element.text_content, 8);

    let action_type = match element.tag.as_str() {
        "input" => "fill",
        "button" | "a" => "click",
        _ => "act",
    };

    Some(CompressedElement {
        tag: element.tag.chars().take(3).collect(), // Truncate tag names
        selector: shorten_selector(&selector),
        text,
        action_type: action_type.to_string(),
    })
}

/// Shorten selector for aggressive compression
fn shorten_selector(selector: &str) -> String {
    // Keep IDs and data-testids as-is
    if selector.starts_with('#') || selector.contains("data-testid") {
        selector.to_string()
    } else if selector.contains('.') {
        // For class selectors, keep only first class
        if let Some(pos) = selector.find('.') {
            let end = selector[pos + 1..]
                .find('.')
                .map(|p| pos + 1 + p)
                .unwrap_or(selector.len());
            selector[..end].to_string()
        } else {
            selector.to_string()
        }
    } else {
        selector.to_string()
    }
}

/// Truncate text to max length
fn truncate_text(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        format!("{}...", &text[..max_len.saturating_sub(3)])
    }
}

/// Estimate tokens for compressed DOM
fn estimate_compressed_tokens(elements: &[CompressedElement], hints: &[String]) -> usize {
    let mut chars = 0;

    for elem in elements {
        chars +=
            elem.tag.len() + elem.selector.len() + elem.text.len() + elem.action_type.len() + 10;
    }

    for hint in hints {
        chars += hint.len() + 5;
    }

    chars / 4 // Rough token estimate
}

/// Convert CompactSnapshot to LLM prompt format
pub fn format_compact_snapshot(snapshot: &CompactSnapshot) -> String {
    let mut output = String::new();

    // Header
    output.push_str(&format!(
        "Page: {} ({})\n",
        truncate_text(&snapshot.title, 50),
        format_compact_url(&snapshot.url)
    ));

    output.push_str(&format!(
        "Viewport: {}x{}\n",
        snapshot.viewport.width, snapshot.viewport.height
    ));

    if !snapshot.frames.is_empty() {
        let accessible_count = snapshot.frames.iter().filter(|f| f.accessible).count();
        output.push_str(&format!(
            "Frames: {} ({} accessible)\n",
            snapshot.frames.len(),
            accessible_count
        ));
    }

    output.push('\n');

    // Group elements by type
    let grouped = group_elements_by_type(&snapshot.elements);

    if !snapshot.elements.is_empty() {
        output.push_str("Interactive Elements:\n");

        for (element_type, elements) in grouped {
            if !elements.is_empty() {
                output.push_str(&format!(
                    "\n{} ({}):\n",
                    element_type.to_uppercase(),
                    elements.len()
                ));

                for element in elements {
                    let line = format_compact_element(&element);
                    output.push_str(&format!("  {}\n", line));
                }
            }
        }
    } else {
        output.push_str("No interactive elements found.\n");
    }

    output.push('\n');

    // Memory summary if available
    if let Some(memory) = &snapshot.memory_summary {
        output.push_str("Recent Actions:\n");
        output.push_str(memory);
        output.push('\n');
    }

    // Footer
    output.push_str(&format!(
        "Snapshot: {}KB, {} compression, {} elements\n",
        snapshot.size_kb,
        snapshot.compression_level,
        snapshot.elements.len()
    ));

    output
}

/// Group elements by type for organized display
fn group_elements_by_type(elements: &[CompactElement]) -> HashMap<&str, Vec<&CompactElement>> {
    let mut groups = HashMap::new();

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
fn format_compact_element(element: &CompactElement) -> String {
    let mut parts = Vec::new();

    // Encoded ID (most important)
    parts.push(element.encoded_id.clone());

    // Selector
    parts.push(format!("[{}]", truncate_text(&element.selector, 30)));

    // Text content
    if let Some(text) = &element.text {
        if !text.is_empty() {
            parts.push(format!("\"{}\"", truncate_text(text, 25)));
        }
    } else if let Some(name) = &element.name {
        if !name.is_empty() {
            parts.push(format!("\"{}\"", truncate_text(name, 25)));
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
        let key_attrs = format_key_attributes(attrs);
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
fn format_key_attributes(attrs: &HashMap<String, String>) -> Vec<String> {
    let mut display = Vec::new();
    let priority = ["data-testid", "id", "name", "type", "placeholder"];

    for key in &priority {
        if let Some(value) = attrs.get(*key) {
            display.push(format!("{}={}", key, truncate_text(value, 20)));
            break; // Only show one key attribute
        }
    }

    display
}

/// Format URL for compact display
fn format_compact_url(url: &str) -> String {
    if let Ok(parsed) = Url::parse(url) {
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
        truncate_text(url, 40)
    }
}

/// Format compressed DOM for LLM consumption (legacy)
pub fn format_compressed_dom(compressed: &CompressedDom) -> String {
    let mut output = format!(
        "URL: {}\nType: {}\n\n",
        compressed.url, compressed.page_type
    );

    if !compressed.action_hints.is_empty() {
        output.push_str("Actions:\n");
        for hint in &compressed.action_hints {
            output.push_str(&format!("- {}\n", hint));
        }
        output.push('\n');
    }

    output.push_str("Elements:\n");
    for (i, elem) in compressed.key_elements.iter().enumerate() {
        output.push_str(&format!(
            "{}. {} {} [{}] {}\n",
            i + 1,
            elem.tag.to_uppercase(),
            elem.selector,
            elem.action_type,
            if elem.text.is_empty() { "" } else { &elem.text }
        ));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom_processor::DomProcessor;

    #[test]
    fn test_token_estimation() {
        let html = r#"
            <html>
                <head><title>Test Page with Long Title That Should Be Truncated</title></head>
                <body>
                    <input type="search" placeholder="Search for products..." />
                    <button>Search Now</button>
                    <div>Some very long text content that should definitely be truncated in aggressive mode</div>
                </body>
            </html>
        "#;

        let processor = DomProcessor::with_defaults();
        let context = processor
            .extract_dom_context(html, "https://example.com/search")
            .unwrap();

        let tokens = estimate_tokens(&context);
        assert!(tokens > 0);
        assert!(tokens < 1000); // Should be reasonable for simple page
    }

    #[test]
    fn test_aggressive_compression() {
        let processor = DomProcessor::with_defaults();

        // Create a large DOM context
        let mut context = DomContext {
            url: "https://example.com/very/long/url/path/that/should/be/truncated".to_string(),
            title: "Very Long Page Title That Definitely Needs To Be Truncated In Aggressive Mode"
                .to_string(),
            page_type: "search_page".to_string(),
            interactive_elements: vec![],
            semantic_groups: HashMap::new(),
            total_elements: 1000,
            processed_elements: 100,
            iframes: vec![],
        };

        // Add many elements to trigger aggressive mode
        for i in 0..50 {
            context.interactive_elements.push(DomElement {
                tag: "div".to_string(),
                id: Some(format!("element-{}", i)),
                classes: vec![format!("class-{}", i)],
                attributes: HashMap::new(),
                text_content: format!("This is element {} with some long text content", i),
                selector_suggestions: vec![format!("#element-{}", i)],
                frame_id: None,
            });
        }

        // Add search elements (should be prioritized)
        context.interactive_elements.push(DomElement {
            tag: "input".to_string(),
            id: Some("search-input".to_string()),
            classes: vec![],
            attributes: {
                let mut attrs = HashMap::new();
                attrs.insert("type".to_string(), "search".to_string());
                attrs.insert("placeholder".to_string(), "Search...".to_string());
                attrs
            },
            text_content: String::new(),
            selector_suggestions: vec!["#search-input".to_string()],
            frame_id: None,
        });

        let compressed = compress_dom_aggressive(&context, 500);

        // Should have limited elements
        assert!(compressed.key_elements.len() <= 10);

        // Should have short text
        for elem in &compressed.key_elements {
            assert!(elem.text.len() <= 11); // 8 chars + "..."
        }

        // URL and title should be truncated
        assert!(compressed.url.len() <= 33);
        assert!(compressed.title.len() <= 23);

        // Should prioritize search input
        assert!(compressed
            .key_elements
            .iter()
            .any(|e| e.selector == "#search-input"));
    }

    #[test]
    fn test_normal_compression_preserves_important_info() {
        let processor = DomProcessor::with_defaults();
        let html = r#"
            <html>
                <body>
                    <input id="username" type="text" placeholder="Enter username" />
                    <input id="password" type="password" />
                    <button data-testid="submit-btn">Login</button>
                </body>
            </html>
        "#;

        let context = processor
            .extract_dom_context(html, "https://example.com/login")
            .unwrap();
        let compressed = compress_dom_aggressive(&context, 2000); // High limit for normal mode

        // Should preserve full selectors
        assert!(compressed
            .key_elements
            .iter()
            .any(|e| e.selector == "#username"));
        assert!(compressed
            .key_elements
            .iter()
            .any(|e| e.selector == "[data-testid='submit-btn']"));

        // Should have action hints
        assert!(!compressed.action_hints.is_empty());
    }
}
