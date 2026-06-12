use colored::*;
use serde_json::Value;

pub fn format_browser_target_error(data: &Value) -> Option<String> {
    let code = data.get("error_code").and_then(Value::as_str)?;
    if !matches!(
        code,
        "NO_BROWSER_BRIDGE_CONNECTED"
            | "AMBIGUOUS_BROWSER_TARGET"
            | "BRIDGE_NOT_FOUND"
            | "BROWSER_INSTANCE_NOT_CONNECTED"
            | "SESSION_TARGET_CONFLICT"
            | "SESSION_NOT_FOUND"
            | "INVALID_TAB_REF"
    ) {
        return None;
    }

    let mut output = String::new();
    output.push_str(&format!("[ERROR] {}\n", code.red().bold()));
    if let Some(error) = data.get("error").and_then(Value::as_str) {
        output.push_str(error);
        output.push('\n');
    }
    if let Some(format_example) = data.get("format_example").and_then(Value::as_str) {
        output.push_str(&format!("format: {}\n", format_example));
    }

    if let Some(candidates) = data.get("candidates").and_then(Value::as_array) {
        if !candidates.is_empty() {
            output.push_str("\nConnected browser targets:\n");
            for candidate in candidates {
                let bridge_id = candidate
                    .get("bridge_id")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>");
                let browser = candidate
                    .get("browser")
                    .or_else(|| candidate.get("extension_target"))
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>");
                let instance = candidate
                    .get("browser_instance_id")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>");
                let hint = candidate
                    .get("extension_target_hint")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>");
                let extension_id = candidate
                    .get("extension_id")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>");
                let last_health = candidate
                    .get("last_health_at_ms")
                    .and_then(Value::as_u64)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<unknown>".to_string());
                output.push_str(&format!(
                    "- bridge={} browser={} instance={} hint={} extension={} last_health_at_ms={}\n",
                    bridge_id, browser, instance, hint, extension_id, last_health
                ));
            }
        }
    }

    if let Some(steps) = data.get("next_steps").and_then(Value::as_array) {
        if !steps.is_empty() {
            output.push_str("\nNext steps:\n");
            for step in steps.iter().filter_map(Value::as_str) {
                output.push_str(&format!("- {}\n", step));
            }
        }
    }

    Some(output)
}

pub fn format_browser_tab_context_result(data: &Value) -> Option<String> {
    let tab_ref = data
        .get("tab_ref")
        .or_else(|| data.get("current_tab_ref"))
        .or_else(|| data.pointer("/result/tab_ref"))
        .or_else(|| data.pointer("/result/current_tab_ref"))
        .and_then(Value::as_str)?;
    let tab_id = data
        .get("tab_id")
        .or_else(|| data.get("current_tab_id"))
        .or_else(|| data.pointer("/result/tab_id"))
        .or_else(|| data.pointer("/result/current_tab_id"))
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<unknown>".to_string());
    let browser_instance_id = data
        .get("browser_instance_id")
        .or_else(|| data.pointer("/result/browser_instance_id"))
        .or_else(|| data.pointer("/resolved_browser_target/browser_instance_id"))
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let bridge_id = data
        .get("bridge_id")
        .or_else(|| data.pointer("/result/bridge_id"))
        .or_else(|| data.pointer("/resolved_browser_target/bridge_id"))
        .or_else(|| data.pointer("/resolved_browser_target/supervisor_bridge_id"))
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let browser = data
        .get("browser")
        .or_else(|| data.get("extension_target"))
        .or_else(|| data.pointer("/result/browser"))
        .or_else(|| data.pointer("/result/extension_target"))
        .or_else(|| data.pointer("/resolved_browser_target/browser"))
        .or_else(|| data.pointer("/resolved_browser_target/extension_target"))
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let current_url = data
        .get("current_url")
        .or_else(|| data.pointer("/result/current_url"))
        .and_then(Value::as_str);
    let success = data
        .get("success")
        .or_else(|| data.get("ok"))
        .and_then(Value::as_bool);

    let mut output = String::new();
    output.push_str(&format!("{}\n", "Browser tab".green().bold()));
    if let Some(success) = success {
        output.push_str(&format!(
            "status: {}\n",
            if success { "success" } else { "failed" }
        ));
    }
    output.push_str(&format!("browser: {}\n", browser));
    output.push_str(&format!("bridge_id: {}\n", bridge_id));
    output.push_str(&format!("browser_instance_id: {}\n", browser_instance_id));
    output.push_str(&format!("tab_id: {}\n", tab_id));
    output.push_str(&format!("tab_ref: {}\n", tab_ref));
    if let Some(current_url) = current_url {
        output.push_str(&format!("current_url: {}\n", current_url));
    }
    Some(output)
}

pub fn format_browser_targets_result(data: &Value) -> Option<String> {
    let targets = data.get("targets").and_then(Value::as_array)?;
    let status = data
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let mut output = String::new();
    output.push_str(&format!("{}\n", "Browser targets".green().bold()));
    output.push_str(&format!("status: {}\n", status));
    output.push_str(&format!("count: {}\n", targets.len()));
    if data.get("compat_source").and_then(Value::as_str).is_some() {
        output.push_str(
            "note: target list came from runtime.status compatibility fallback; restart the supervisor after upgrading.\n",
        );
    }
    if let Some(default_target) = data.get("default_target").filter(|value| !value.is_null()) {
        output.push_str(&format!(
            "default: {}\n",
            format_browser_target(default_target)
        ));
    } else {
        output.push_str("default: none\n");
    }
    if targets.is_empty() {
        output.push_str("No browser bridges are connected.\n");
        output.push_str("Set a default with: rzn-browser browser set chromium\n");
        return Some(output);
    }

    for target in targets {
        let bridge_id = target
            .get("bridge_id")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let browser_instance_id = target
            .get("browser_instance_id")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let browser = target
            .get("browser")
            .or_else(|| target.get("extension_target"))
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let extension_id = target
            .get("extension_id")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let caller_origin = target
            .get("caller_origin")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let last_ping = target
            .get("last_ping_status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let active_sessions = target
            .get("active_session_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        output.push_str(&format!(
            "- browser={} bridge={} instance={} extension={} origin={} ping={} sessions={}\n",
            browser,
            bridge_id,
            browser_instance_id,
            extension_id,
            caller_origin,
            last_ping,
            active_sessions
        ));
        if bridge_id != "<unknown>" {
            output.push_str(&format!("  --bridge {}\n", bridge_id));
        }
        if browser_instance_id != "<unknown>" {
            output.push_str(&format!("  --browser-instance {}\n", browser_instance_id));
        }
        if browser != "<unknown>" {
            output.push_str(&format!("  rzn-browser browser set {}\n", browser));
        } else {
            output.push_str(
                "  identity unavailable; reload the extension or restart the upgraded supervisor\n",
            );
        }
    }
    output.push_str("\nSet default examples:\n");
    output.push_str("  rzn-browser browser set chromium\n");
    output.push_str("  rzn-browser browser set edge\n");
    output.push_str("  rzn-browser browser set --browser-instance <browser_instance_id>\n");

    Some(output)
}

fn format_browser_target(target: &Value) -> String {
    if let Some(preferred) = target.get("preferred") {
        return format!(
            "{} (fallback: single connected)",
            format_browser_target(preferred)
        );
    }
    if let Some(browser) = target.get("browser").and_then(Value::as_str) {
        return format!("browser={browser}");
    }
    if let Some(browser_instance_id) = target.get("browser_instance_id").and_then(Value::as_str) {
        return format!("browser_instance_id={browser_instance_id}");
    }
    if let Some(bridge_id) = target
        .get("bridge_id")
        .or_else(|| target.get("supervisor_bridge_id"))
        .and_then(Value::as_str)
    {
        return format!("bridge_id={bridge_id}");
    }
    target.to_string()
}

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
        if let Some(first) = arr.first() {
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
        if results_slice.is_none()
            && arr.iter().all(|item| {
                item.get("title").is_some()
                    || item.get("url").is_some()
                    || item.get("snippet").is_some()
            })
        {
            results_slice = Some(arr);
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
        if current_line.len() + word.len() + 1 > width && !current_line.is_empty() {
            lines.push(current_line.clone());
            current_line.clear();
            current_line.push_str("   "); // Indent continuation
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn browser_target_error_formatter_suggests_concrete_flags() {
        let rendered = format_browser_target_error(&json!({
            "error_code": "AMBIGUOUS_BROWSER_TARGET",
            "error": "2 browser bridges are connected.",
            "candidates": [
                {
                    "bridge_id": "chrome-bridge",
                    "browser": "chrome",
                    "browser_instance_id": "chrome-instance",
                    "extension_target_hint": "chromium-mv3",
                    "extension_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "last_health_at_ms": 123
                }
            ],
            "next_steps": [
                "Pass --browser <chrome|chromium|edge> to select a browser kind.",
                "Pass --browser-instance <browser_instance_id> to select a browser profile.",
                "Pass --bridge <bridge_id> to select an exact connected bridge."
            ]
        }))
        .expect("browser target error formats");

        assert!(rendered.contains("AMBIGUOUS_BROWSER_TARGET"));
        assert!(rendered.contains("chrome-bridge"));
        assert!(rendered.contains("chrome-instance"));
        assert!(rendered.contains("--browser "));
        assert!(rendered.contains("--browser-instance "));
        assert!(rendered.contains("--bridge "));
    }

    #[test]
    fn browser_tab_context_formatter_shows_distinguishing_context() {
        let rendered = format_browser_tab_context_result(&json!({
            "success": true,
            "current_url": "https://example.test/",
            "result": {
                "browser": "edge",
                "bridge_id": "edge-bridge",
                "browser_instance_id": "edge-instance",
                "tab_id": 7,
                "tab_ref": "rzn://browser/edge-instance/tab/7"
            }
        }))
        .expect("tab context formats");

        assert!(rendered.contains("edge"));
        assert!(rendered.contains("edge-bridge"));
        assert!(rendered.contains("edge-instance"));
        assert!(rendered.contains("tab_id: 7"));
        assert!(rendered.contains("rzn://browser/edge-instance/tab/7"));
        assert!(rendered.contains("https://example.test/"));
    }

    #[test]
    fn browser_targets_formatter_is_compact_and_omits_sensitive_paths() {
        let rendered = format_browser_targets_result(&json!({
            "ok": true,
            "status": "connected",
            "target_count": 1,
            "targets": [
                {
                    "browser": "chrome",
                    "bridge_id": "chrome-bridge",
                    "browser_instance_id": "chrome-instance",
                    "extension_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "caller_origin": "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
                    "last_ping_status": "ok",
                    "active_session_count": 1,
                    "token_path": "/tmp/should-not-render/token"
                }
            ]
        }))
        .expect("browser targets formats");

        assert!(rendered.contains("chrome-bridge"));
        assert!(rendered.contains("chrome-instance"));
        assert!(rendered.contains("--bridge chrome-bridge"));
        assert!(rendered.contains("--browser-instance chrome-instance"));
        assert!(!rendered.contains("token_path"));
        assert!(!rendered.contains("/tmp/should-not-render"));
    }
}
