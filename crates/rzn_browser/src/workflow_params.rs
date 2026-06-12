use serde_json::{Map, Value};
use std::collections::HashMap;

pub(crate) fn apply_parameters(mut value: Value, params: &HashMap<String, String>) -> Value {
    substitute_value(&mut value, params, false);
    inject_script_params(&mut value, params);
    value
}

fn substitute_value(value: &mut Value, params: &HashMap<String, String>, is_script_field: bool) {
    match value {
        Value::String(s) => {
            *s = if is_script_field {
                substitute_script_string(s, params)
            } else {
                substitute_string(s, params)
            };
        }
        Value::Array(items) => {
            for item in items {
                substitute_value(item, params, false);
            }
        }
        Value::Object(map) => {
            let is_script_step = is_script_step(map);
            for (key, value) in map.iter_mut() {
                substitute_value(value, params, is_script_step && key == "script");
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn substitute_string(input: &str, params: &HashMap<String, String>) -> String {
    let mut out = input.to_string();
    let max_passes = params.len().saturating_add(1).clamp(1, 32);
    for _ in 0..max_passes {
        let before = out.clone();
        for (key, val) in params {
            out = out.replace(&format!("{{{}}}", key), val);
        }
        if out == before {
            break;
        }
    }
    out
}

fn substitute_script_string(input: &str, params: &HashMap<String, String>) -> String {
    let mut out = input.to_string();
    let max_passes = params.len().saturating_add(1).clamp(1, 32);
    for _ in 0..max_passes {
        let before = out.clone();
        for (key, val) in params {
            let placeholder = format!("{{{}}}", key);
            let json_literal = serde_json::to_string(val).unwrap_or_else(|_| "\"\"".to_string());
            out = out
                .replace(&format!("'{}'", placeholder), &json_literal)
                .replace(&format!("\"{}\"", placeholder), &json_literal)
                .replace(&format!("`{}`", placeholder), &json_literal)
                .replace(&placeholder, &escape_js_string_fragment(val));
        }
        if out == before {
            break;
        }
    }
    out
}

fn escape_js_string_fragment(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '"' => out.push_str("\\\""),
            '`' => out.push_str("\\`"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(ch),
        }
    }
    out.replace("${", "\\${")
}

pub(crate) fn inject_script_params(value: &mut Value, params: &HashMap<String, String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                inject_script_params(item, params);
            }
        }
        Value::Object(map) => {
            if is_script_step(map) {
                let params_value = params
                    .iter()
                    .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                    .collect();
                map.insert("params".to_string(), Value::Object(params_value));
            }
            for value in map.values_mut() {
                inject_script_params(value, params);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn is_script_step(map: &Map<String, Value>) -> bool {
    map.get("type")
        .and_then(|value| value.as_str())
        .map(|step_type| {
            matches!(
                step_type,
                "execute_javascript" | "eval_main_world" | "eval_isolated_world"
            )
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::apply_parameters;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn substitute_injects_safe_params_for_script_steps() {
        let workflow = json!({
            "steps": [{
                "type": "execute_javascript",
                "script": "return window.__rzn_params.message_body;",
                "args": []
            }]
        });
        let params = HashMap::from([("message_body".to_string(), "O'Reilly".to_string())]);

        let applied = apply_parameters(workflow, &params);
        let step = &applied["steps"][0];

        assert_eq!(step["params"]["message_body"], "O'Reilly");
        assert_eq!(step["script"], "return window.__rzn_params.message_body;");
    }

    #[test]
    fn substitute_expands_chained_param_defaults() {
        let workflow = json!({
            "steps": [{
                "type": "navigate_to_url",
                "url": "{app_url}"
            }]
        });
        let params = HashMap::from([
            (
                "app_url".to_string(),
                "https://apps.apple.com/{country}/app/id{app_id}".to_string(),
            ),
            ("country".to_string(), "us".to_string()),
            ("app_id".to_string(), "123456789".to_string()),
        ]);

        let applied = apply_parameters(workflow, &params);

        assert_eq!(
            applied
                .pointer("/steps/0/url")
                .and_then(|value| value.as_str()),
            Some("https://apps.apple.com/us/app/id123456789")
        );
    }

    #[test]
    fn substitute_script_placeholder_inside_single_quotes_as_js_literal() {
        let workflow = json!({
            "steps": [{
                "type": "execute_javascript",
                "script": "const value = cleanArg('{message}'); return value;"
            }]
        });
        let params = HashMap::from([("message".to_string(), "O'Reilly \\\n</script>".to_string())]);

        let applied = apply_parameters(workflow, &params);

        assert_eq!(
            applied["steps"][0]["script"],
            "const value = cleanArg(\"O'Reilly \\\\\\n</script>\"); return value;"
        );
    }

    #[test]
    fn substitute_script_placeholder_inside_template_fragment_escapes_js_string_chars() {
        let workflow = json!({
            "steps": [{
                "type": "execute_javascript",
                "script": "const url = `https://example.com?q={query}`; return url;"
            }]
        });
        let params = HashMap::from([("query".to_string(), "x`);alert(1);//${bad}".to_string())]);

        let applied = apply_parameters(workflow, &params);
        let script = applied["steps"][0]["script"].as_str().unwrap();

        assert!(script.contains("x\\`);alert(1);//\\${bad}"));
    }
}
