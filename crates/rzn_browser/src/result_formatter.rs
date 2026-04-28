use colored::*;
use serde_json::Value;

pub fn format_google_search_results(data: &Value) -> String {
    let mut output = String::new();

    // Check if this is a simple array of results (new format)
    if let Some(arr) = data.as_array() {
        if arr.iter().all(|item| {
            item.get("title").is_some()
                || item.get("url").is_some()
                || item.get("snippet").is_some()
        }) {
            // Simple Google search results format
            output.push_str(&format!(
                "\n[SEARCH] {} ({})\n",
                "Search Results".blue().bold(),
                arr.len()
            ));
            output.push_str(&format!("{}\n", "─".repeat(60).dimmed()));

            for (idx, result) in arr.iter().enumerate() {
                // Title
                if let Some(title) = result.get("title").and_then(|v| v.as_str()) {
                    output.push_str(&format!(
                        "{}. {}\n",
                        (idx + 1).to_string().bright_white().bold(),
                        title.green().bold()
                    ));
                }

                // URL
                if let Some(url) = result.get("url").and_then(|v| v.as_str()) {
                    output.push_str(&format!("    {}\n", url.bright_cyan()));
                }

                // Snippet
                if let Some(snippet) = result.get("snippet").and_then(|v| v.as_str()) {
                    if !snippet.is_empty() {
                        output.push_str("   ");
                        output.push_str(&wrap_text(snippet, 57));
                        output.push('\n');
                    }
                }

                output.push('\n');
            }

            return output;
        }
    }

    // Check if this is Google search results based on structure
    if !is_google_search_results(data) {
        // Fallback to pretty JSON for non-Google results
        return serde_json::to_string_pretty(data)
            .unwrap_or_else(|_| "Failed to format data".to_string());
    }

    // Extract query info if available
    if let Some(query_info) = extract_query_info(data) {
        output.push_str(&format!(
            "\n[SEARCH] {} for: {}\n",
            "Search Results".blue().bold(),
            query_info.yellow()
        ));
        output.push_str(&format!(
            "⏰ {}\n",
            chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
                .dimmed()
        ));
        output.push_str(&format!("{}\n", "─".repeat(60).dimmed()));
    }

    // Extract and display AI Overview if present
    if let Some(ai_overview) = extract_ai_overview(data) {
        output.push_str(&format!(
            "\n[BOT] {} {}\n",
            "AI Overview:".green().bold(),
            "(Google AI)".dimmed()
        ));
        output.push_str(&format!("{}\n", "─".repeat(60).dimmed()));
        output.push_str(&wrap_text(&ai_overview, 60));
        output.push_str(&format!("\n{}\n", "─".repeat(60).dimmed()));
    }

    // Extract and display search results
    if let Some(results) = extract_search_results(data) {
        output.push_str(&format!(
            "\n[LIST] {} ({})\n\n",
            "Search Results".cyan().bold(),
            results.len()
        ));

        for (idx, result) in results.iter().enumerate() {
            // Position and title
            output.push_str(&format!(
                "{}. {}\n",
                (idx + 1).to_string().bright_white().bold(),
                result
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .green()
                    .bold()
            ));

            // URL and domain
            if let Some(url) = result.get("url").and_then(|v| v.as_str()) {
                let domain = result.get("domain").and_then(|v| v.as_str()).unwrap_or("");
                output.push_str(&format!(
                    "    {} {}\n",
                    domain.bright_cyan(),
                    format!("({})", url).dimmed()
                ));
            }

            // Description
            if let Some(desc) = result.get("description").and_then(|v| v.as_str()) {
                if !desc.is_empty() {
                    output.push_str("   ");
                    output.push_str(&wrap_text(desc, 57));
                    output.push('\n');
                }
            }

            output.push('\n');
        }
    }

    // Extract and display People Also Ask
    if let Some(paa) = extract_people_also_ask(data) {
        if !paa.is_empty() {
            output.push_str(&format!("{}\n", "─".repeat(60).dimmed()));
            output.push_str(&format!("❓ {}\n", "People Also Ask".magenta().bold()));
            for question in paa {
                output.push_str(&format!("   • {}\n", question));
            }
        }
    }

    // Extract and display related searches
    if let Some(related) = extract_related_searches(data) {
        if !related.is_empty() {
            output.push_str(&format!("\n{}\n", "─".repeat(60).dimmed()));
            output.push_str(&format!("🔎 {}\n", "Related Searches".blue().bold()));
            for search in related {
                output.push_str(&format!("   • {}\n", search.italic()));
            }
        }
    }

    output
}

/// Format a simple search results array into Markdown.
/// Supports arrays of objects with { title, url, snippet }.
pub fn format_markdown_results(data: &Value) -> Option<String> {
    // Accept either:
    // - Top-level array of result objects {title,url,snippet}
    // - Top-level array where [0] is that result array and subsequent entries are metadata (e.g., dom_snapshot)
    let mut results_slice: Option<&Vec<Value>> = None;
    if let Some(arr) = data.as_array() {
        // Case A: nested array first element
        if let Some(first) = arr.get(0) {
            if let Some(inner) = first.as_array() {
                // Ensure it looks like results
                if inner
                    .iter()
                    .any(|item| item.get("title").is_some() || item.get("url").is_some())
                {
                    results_slice = Some(inner);
                }
            }
        }
        // Case B: flat array of objects
        if results_slice.is_none() {
            if arr.iter().all(|item| {
                item.get("title").is_some()
                    || item.get("url").is_some()
                    || item.get("snippet").is_some()
            }) {
                results_slice = Some(arr);
            }
        }
    }
    let results = results_slice?;

    let mut out = String::new();
    out.push_str("## Search Results\n\n");
    for (i, item) in results.iter().enumerate() {
        let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let snippet = item.get("snippet").and_then(|v| v.as_str()).unwrap_or("");
        if !url.is_empty() && !title.is_empty() {
            out.push_str(&format!("{}. [{}]({})\n", i + 1, title, url));
        } else if !title.is_empty() {
            out.push_str(&format!("{}. {}\n", i + 1, title));
        }
        if !snippet.is_empty() {
            out.push_str(&format!("   - {}\n", snippet));
        }
    }
    Some(out)
}

fn is_google_search_results(data: &Value) -> bool {
    // Check for multiple extraction results (our new format)
    if let Some(arr) = data.as_array() {
        // Look for our specific extraction steps
        return arr.iter().any(|v| {
            if let Some(obj) = v.as_object() {
                return obj.contains_key("position")
                    || obj.contains_key("questions")
                    || obj.contains_key("hasAiOverview");
            }
            false
        });
    }
    false
}

fn extract_query_info(data: &Value) -> Option<String> {
    if let Some(arr) = data.as_array() {
        for item in arr {
            if let Some(query) = item.get("query").and_then(|v| v.as_str()) {
                return Some(query.to_string());
            }
        }
    }
    None
}

fn extract_ai_overview(data: &Value) -> Option<String> {
    if let Some(arr) = data.as_array() {
        for item in arr {
            if let Some(has_ai) = item.get("hasAiOverview").and_then(|v| v.as_bool()) {
                if has_ai {
                    return item
                        .get("content")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
            }
        }
    }
    None
}

fn extract_search_results(data: &Value) -> Option<Vec<&Value>> {
    if let Some(arr) = data.as_array() {
        for item in arr {
            if let Some(results_arr) = item.as_array() {
                // Check if this is the search results array
                if results_arr.iter().any(|r| r.get("position").is_some()) {
                    return Some(results_arr.iter().collect());
                }
            }
        }
    }
    None
}

fn extract_people_also_ask(data: &Value) -> Option<Vec<String>> {
    if let Some(arr) = data.as_array() {
        for item in arr {
            if let Some(questions) = item.get("questions").and_then(|v| v.as_array()) {
                return Some(
                    questions
                        .iter()
                        .filter_map(|q| q.as_str().map(|s| s.to_string()))
                        .collect(),
                );
            }
        }
    }
    None
}

fn extract_related_searches(data: &Value) -> Option<Vec<String>> {
    if let Some(arr) = data.as_array() {
        for item in arr {
            if let Some(searches) = item.get("searches").and_then(|v| v.as_array()) {
                return Some(
                    searches
                        .iter()
                        .filter_map(|s| s.as_str().map(|s| s.to_string()))
                        .collect(),
                );
            }
        }
    }
    None
}

fn wrap_text(text: &str, width: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut lines = vec![];
    let mut current_line = String::new();

    for word in words {
        if current_line.len() + word.len() + 1 > width {
            if !current_line.is_empty() {
                lines.push(current_line.clone());
                current_line.clear();
                current_line.push_str("   "); // Indent continuation
            }
        }
        if !current_line.is_empty() && !current_line.ends_with(' ') {
            current_line.push(' ');
        }
        current_line.push_str(word);
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    lines.join("\n")
}
