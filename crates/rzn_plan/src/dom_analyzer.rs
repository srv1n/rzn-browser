use crate::PlanResult;
use dom_query::Document;
use log::debug;

/// Analyzes and reduces DOM content for LLM processing with dom_query
pub struct DomAnalyzer {
    max_size: usize,
}

impl DomAnalyzer {
    pub fn new(max_size: usize) -> Self {
        Self { max_size }
    }

    /// Reduce HTML to a concise outline for LLM consumption
    pub fn reduce_html(&self, html: &str) -> PlanResult<String> {
        let document = Document::from(html);
        let mut outline = String::new();

        // Use dom_query's powerful CSS selector support
        self.extract_interactive_elements(&document, &mut outline)?;
        self.extract_content_structure(&document, &mut outline)?;
        self.extract_data_patterns(&document, &mut outline)?;
        self.analyze_extraction_opportunities(&document, &mut outline)?;

        // Truncate if too long
        if outline.len() > self.max_size {
            outline.truncate(self.max_size);
            outline.push_str("\n... (truncated)");
        }

        debug!(
            "Reduced HTML from {} to {} characters",
            html.len(),
            outline.len()
        );
        Ok(outline)
    }

    /// Extract interactive elements using dom_query's superior CSS selector support
    fn extract_interactive_elements(
        &self,
        document: &Document,
        outline: &mut String,
    ) -> PlanResult<()> {
        outline.push_str("=== INTERACTIVE ELEMENTS ===\n");

        // Enhanced patterns that dom_query can handle
        let patterns = [
            ("button", "Buttons"),
            ("input", "Input Fields"),
            ("select", "Select Dropdowns"),
            ("textarea", "Text Areas"),
            ("[role='button']", "Button Roles"),
            ("[data-testid]", "Test Targets"),
            ("[data-test-id]", "Alternative Test IDs"),
            ("[data-cy]", "Cypress Test Targets"),
            ("a[href]", "Links"),
            ("[onclick]", "Click Handlers"),
            ("[contenteditable='true']", "Editable Content"),
        ];

        for (selector, category) in &patterns {
            let selection = document.select(selector);
            let count = selection.length();
            if count > 0 {
                outline.push_str(&format!("{}: {} elements\n", category, count));

                // Show sample attributes from first match if available
                if selection.length() > 0 {
                    let text = selection
                        .text()
                        .to_string()
                        .trim()
                        .chars()
                        .take(50)
                        .collect::<String>();
                    if !text.is_empty() {
                        outline.push_str(&format!("  Sample text: '{}'\n", text));
                    }
                }
            }
        }
        outline.push('\n');
        Ok(())
    }

    /// Extract content structure using semantic HTML analysis
    fn extract_content_structure(
        &self,
        document: &Document,
        outline: &mut String,
    ) -> PlanResult<()> {
        outline.push_str("=== CONTENT STRUCTURE ===\n");

        let structure_patterns = [
            ("main, [role='main']", "Main Content Areas"),
            ("article, [role='article']", "Articles"),
            ("section", "Sections"),
            ("nav, [role='navigation']", "Navigation"),
            ("header", "Headers"),
            ("footer", "Footers"),
            ("h1", "H1 Headings"),
            ("h2", "H2 Headings"),
            ("h3", "H3 Headings"),
            ("h4, h5, h6", "Lower Level Headings"),
            ("form", "Forms"),
            ("table", "Tables"),
        ];

        for (selector, category) in &structure_patterns {
            let selection = document.select(selector);
            let count = selection.length();
            if count > 0 {
                outline.push_str(&format!("{}: {} elements\n", category, count));

                // For headings, show sample text
                if (selector.contains("h1") || selector.contains("h2") || selector.contains("h3"))
                    && selection.length() > 0
                {
                    let heading_text = selection
                        .text()
                        .to_string()
                        .trim()
                        .chars()
                        .take(60)
                        .collect::<String>();
                    if !heading_text.is_empty() {
                        outline.push_str(&format!("  Sample: '{}'\n", heading_text));
                    }
                }
            }
        }
        outline.push('\n');
        Ok(())
    }

    /// Extract data patterns using enhanced attribute selectors
    fn extract_data_patterns(&self, document: &Document, outline: &mut String) -> PlanResult<()> {
        outline.push_str("=== DATA PATTERNS ===\n");

        let data_patterns = [
            "[data-testid]",
            "[data-test-id]",
            "[data-cy]",
            "[data-qa]",
            "[data-automation]",
            "[data-ved]",
            "[data-item-id]",
            "[data-product-id]",
            "[data-track]",
            "[data-analytics]",
            "[data-component]",
            "[data-element]",
            "[aria-label]",
            "[role]",
            "[itemscope]",
            "[itemtype]",
        ];

        for pattern in &data_patterns {
            let selection = document.select(pattern);
            let count = selection.length();
            if count > 0 {
                outline.push_str(&format!("{}: {} elements\n", pattern, count));
            }
        }
        outline.push('\n');
        Ok(())
    }

    /// Analyze extraction opportunities using dom_query's advanced selectors
    fn analyze_extraction_opportunities(
        &self,
        document: &Document,
        outline: &mut String,
    ) -> PlanResult<()> {
        outline.push_str("=== EXTRACTION OPPORTUNITIES ===\n");

        // dom_query supports :has() and other advanced pseudo-selectors!
        let opportunity_patterns = [
            ("div:has(h1, h2, h3)", "Content blocks with headings"),
            ("div:has(a[href])", "Content blocks with links"),
            ("article:has(h1, h2, h3)", "Articles with headings"),
            ("li:has(a[href])", "List items with links"),
            ("[data-testid]:has(a)", "Test elements containing links"),
            (
                "[data-testid]:has(h1, h2, h3)",
                "Test elements with headings",
            ),
            ("div:has(img):has(a)", "Media content blocks"),
            (".card, .item, .result, .post", "Common content containers"),
            (".list-item, .search-result, .product", "Structured content"),
        ];

        let mut best_opportunities = Vec::new();

        for (selector, description) in &opportunity_patterns {
            let selection = document.select(selector);
            let count = selection.length();

            if count > 0 {
                let quality_score = self.assess_extraction_quality(count);

                outline.push_str(&format!(
                    "[LIST] {}: {} elements (quality: {})\n",
                    description, count, quality_score
                ));

                if quality_score >= 7 {
                    best_opportunities.push((selector, description, count, quality_score));
                }
            }
        }

        // Provide specific recommendations
        outline.push_str("\n[TARGET] EXTRACTION RECOMMENDATIONS:\n");
        if best_opportunities.is_empty() {
            outline.push_str(
                "[ERROR] No high-quality extraction targets found. Try broader selectors.\n",
            );

            // Suggest fallback patterns
            outline.push_str("[TIP] FALLBACK SUGGESTIONS:\n");
            let fallbacks = ["div", "article", "li", "[data-testid]", ".item", ".result"];
            for fallback in &fallbacks {
                let count = document.select(fallback).length();
                if count > 0 {
                    outline.push_str(&format!("   - '{}': {} elements\n", fallback, count));
                }
            }
        } else {
            best_opportunities.sort_by(|a, b| b.3.cmp(&a.3)); // Sort by quality score

            for (selector, description, count, score) in best_opportunities.iter().take(3) {
                outline.push_str(&format!(
                    "[OK] RECOMMENDED: '{}' - {} ({} elements, score: {})\n",
                    selector, description, count, score
                ));
            }
        }

        // Enhanced dynamic content analysis
        self.analyze_dynamic_content(document, outline)?;

        outline.push('\n');
        Ok(())
    }

    /// Assess extraction quality based on element count
    fn assess_extraction_quality(&self, count: usize) -> u8 {
        let mut score = 5u8; // Base score

        // Optimal range scoring
        if (5..=25).contains(&count) {
            score += 3; // Perfect range
        } else if (3..5).contains(&count) || (25..50).contains(&count) {
            score += 1; // Acceptable range
        } else if count < 3 {
            score = score.saturating_sub(2); // Too few
        } else if count > 100 {
            score = score.saturating_sub(3); // Way too many
        } else if count > 50 {
            score = score.saturating_sub(1); // Too many
        }

        score.min(10) // Cap at 10
    }

    /// Analyze dynamic content and pagination patterns
    fn analyze_dynamic_content(&self, document: &Document, outline: &mut String) -> PlanResult<()> {
        outline.push_str("\n DYNAMIC CONTENT ANALYSIS:\n");

        // Infinite scroll and load more indicators
        let scroll_patterns = [
            ("[data-testid*='load']", "Load more buttons"),
            ("[data-testid*='more']", "More content buttons"),
            ("[aria-label*='load' i]", "Load indicators"),
            ("[aria-label*='more' i]", "More indicators"),
            (".infinite-scroll", "Infinite scroll containers"),
            (".load-more", "Load more elements"),
            ("button:has-text('Load')", "Load buttons by text"),
            ("button:has-text('More')", "More buttons by text"),
        ];

        for (selector, description) in &scroll_patterns {
            let count = document.select(selector).length();
            if count > 0 {
                outline.push_str(&format!(" {}: {} found\n", description, count));
            }
        }

        // Pagination patterns
        let pagination_patterns = [
            (".pagination", "Pagination containers"),
            ("[aria-label*='page' i]", "Page navigation"),
            ("[aria-label*='next' i]", "Next page links"),
            ("[data-testid*='next']", "Next page test elements"),
            ("[data-testid*='page']", "Page test elements"),
            ("a[href*='page=']", "Page parameter links"),
            ("a[href*='p=']", "Page parameter (short)"),
            ("button:has-text('Next')", "Next buttons by text"),
        ];

        for (selector, description) in &pagination_patterns {
            let count = document.select(selector).length();
            if count > 0 {
                outline.push_str(&format!(" {}: {} found\n", description, count));
            }
        }

        // Search and filter patterns
        let interaction_patterns = [
            ("input[type='search']", "Search inputs"),
            ("input[placeholder*='search' i]", "Search placeholders"),
            ("[data-testid*='search']", "Search test elements"),
            ("[data-testid*='filter']", "Filter test elements"),
            ("select:has(option)", "Filter dropdowns"),
            (".filter, .filters", "Filter containers"),
        ];

        for (selector, description) in &interaction_patterns {
            let count = document.select(selector).length();
            if count > 0 {
                outline.push_str(&format!("[SEARCH] {}: {} found\n", description, count));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dom_query_enhanced_analysis() {
        let analyzer = DomAnalyzer::new(50_000);

        let html = r#"
        <html>
        <head><title>Test Site</title></head>
        <body>
            <main class="container">
                <h1>Main Page Title</h1>
                
                <!-- Good extraction targets -->
                <div class="result-item" data-testid="result-1">
                    <h3>First Article Title</h3>
                    <a href="/article/1">Read More</a>
                    <p>Description of the first article.</p>
                </div>
                <div class="result-item" data-testid="result-2">
                    <h3>Second Article Title</h3>
                    <a href="/article/2">Read More</a>
                    <p>Description of the second article.</p>
                </div>
                <div class="result-item" data-testid="result-3">
                    <h3>Third Article Title</h3>
                    <a href="/article/3">Read More</a>
                    <p>Description of the third article.</p>
                </div>
                
                <!-- Dynamic content indicators -->
                <button data-testid="load-more">Load More Results</button>
                <nav class="pagination">
                    <a href="?page=2" aria-label="Next page">Next</a>
                </nav>
            </main>
        </body>
        </html>
        "#;

        let result = analyzer.reduce_html(html).unwrap();

        println!("DOM Query Enhanced Analysis:\n{}", result);

        // Verify the enhanced analysis structure
        assert!(result.contains("=== INTERACTIVE ELEMENTS ==="));
        assert!(result.contains("=== CONTENT STRUCTURE ==="));
        assert!(result.contains("=== DATA PATTERNS ==="));
        assert!(result.contains("=== EXTRACTION OPPORTUNITIES ==="));

        // Check for specific elements
        assert!(result.contains("[data-testid]: 4 elements")); // 3 results + 1 button
        assert!(result.contains("[TARGET] EXTRACTION RECOMMENDATIONS:"));
        assert!(result.contains(" DYNAMIC CONTENT ANALYSIS:"));

        // Should detect the good extraction patterns
        assert!(result.contains("Content blocks with headings"));
        assert!(result.contains("Load more buttons"));
    }
}
