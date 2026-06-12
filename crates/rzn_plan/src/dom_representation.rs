/// Simplified DOM representation for LLM consumption
/// Based on learnings from public reference projects
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Minimal element representation optimized for LLM understanding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimplifiedElement {
    /// Element tag (e.g., "input", "button", "a")
    pub tag: String,

    /// Human-readable text or label
    pub text: String,

    /// Key attributes only
    pub attrs: HashMap<String, String>,

    /// Simple location description
    pub location: String,

    /// Whether element is visible and interactive
    pub interactive: bool,
}

/// Attributes we actually care about for browser automation
pub const RELEVANT_ATTRIBUTES: &[&str] = &[
    "type",
    "name",
    "value",
    "placeholder",
    "href",
    "title",
    "alt",
    "aria-label",
    "role",
    "checked",
    "selected",
    "disabled",
];

impl SimplifiedElement {
    /// Create a concise string representation for LLM
    pub fn to_llm_string(&self, index: usize) -> String {
        let mut parts = vec![format!("[{}]", index)];

        // Add tag
        parts.push(self.tag.clone());

        // Add text if present
        if !self.text.is_empty() {
            parts.push(format!("'{}'", self.text));
        }

        // Add key attributes
        if let Some(placeholder) = self.attrs.get("placeholder") {
            parts.push(format!("placeholder='{}'", placeholder));
        }
        if let Some(href) = self.attrs.get("href") {
            parts.push(format!("→{}", href));
        }

        // Add location hint
        parts.push(format!("@{}", self.location));

        parts.join(" ")
    }
}

/// Convert whitelist to simplified format for LLM
pub fn simplify_for_llm(
    elements: &HashMap<usize, crate::dom_whitelist::WhitelistElement>,
) -> HashMap<usize, String> {
    let mut simplified = HashMap::new();

    for (index, element) in elements {
        let simple = SimplifiedElement {
            tag: element.tag.clone(),
            text: element.text.clone(),
            attrs: element
                .attributes
                .iter()
                .filter(|(k, _)| RELEVANT_ATTRIBUTES.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            location: describe_location_from_selector(&element.selector),
            interactive: true,
        };

        simplified.insert(*index, simple.to_llm_string(*index));
    }

    simplified
}

/// Simple location description based on selector or element type
fn describe_location_from_selector(selector: &str) -> String {
    // Try to infer location from common patterns
    if selector.contains("header") || selector.contains("nav") || selector.contains("top") {
        "top".to_string()
    } else if selector.contains("footer") || selector.contains("bottom") {
        "bottom".to_string()
    } else if selector.contains("sidebar") || selector.contains("left") {
        "left".to_string()
    } else if selector.contains("right") {
        "right".to_string()
    } else if selector.contains("main")
        || selector.contains("content")
        || selector.contains("center")
    {
        "center".to_string()
    } else {
        "page".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simplified_representation() {
        let mut attrs = HashMap::new();
        attrs.insert("placeholder".to_string(), "Search".to_string());
        attrs.insert("type".to_string(), "text".to_string());

        let element = SimplifiedElement {
            tag: "input".to_string(),
            text: String::new(),
            attrs,
            location: "top-center".to_string(),
            interactive: true,
        };

        let result = element.to_llm_string(1);
        assert!(result.contains("[1]"));
        assert!(result.contains("input"));
        assert!(result.contains("placeholder='Search'"));
        assert!(result.contains("@top-center"));
    }
}
