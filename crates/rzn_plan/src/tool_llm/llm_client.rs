use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: ToolParameters,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParameters {
    pub required: Vec<String>,
    pub properties: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub arguments: Value,
}

impl Tool {
    fn openai_value(self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": {
                    "type": "object",
                    "required": self.parameters.required,
                    "properties": self.parameters.properties,
                }
            }
        })
    }
}

pub fn allowed_tool_values(allowed: &[&str]) -> Vec<Value> {
    standard_tools()
        .into_iter()
        .filter(|tool| allowed.contains(&tool.name.as_str()))
        .map(Tool::openai_value)
        .collect()
}

pub fn parse_tool_calls(response: &Value) -> Result<Vec<ToolCall>, String> {
    response
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("tool_calls"))
        .and_then(Value::as_array)
        .ok_or_else(|| "No tool calls in response".to_string())?
        .iter()
        .map(|tool_call| {
            let function = tool_call
                .get("function")
                .ok_or_else(|| "Missing tool function".to_string())?;
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing tool name".to_string())?
                .to_string();
            let arguments = function
                .get("arguments")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing tool arguments".to_string())?;
            let arguments = serde_json::from_str(arguments)
                .map_err(|error| format!("Failed to parse tool arguments: {error}"))?;
            Ok(ToolCall { name, arguments })
        })
        .collect()
}

pub fn standard_tools() -> Vec<Tool> {
    vec![
        tool(
            "navigate",
            "Navigate to a URL",
            &["url"],
            json!({"url": {"type": "string", "description": "The URL to navigate to"}}),
        ),
        tool(
            "type",
            "Type text into an input field",
            &["selector", "text"],
            json!({
                "selector": {"type": "string", "description": "CSS selector for the input"},
                "text": {"type": "string", "description": "Text to type"}
            }),
        ),
        tool(
            "press_key",
            "Press a keyboard key",
            &["key"],
            json!({
                "key": {
                    "type": "string",
                    "enum": ["Enter", "Tab", "Escape", "ArrowUp", "ArrowDown"],
                    "description": "The key to press"
                }
            }),
        ),
        tool(
            "click",
            "Click on an element",
            &["selector"],
            json!({"selector": {"type": "string", "description": "CSS selector for the element"}}),
        ),
        tool(
            "extract",
            "Extract data from the page",
            &["fields"],
            json!({
                "fields": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {"type": "string"},
                            "selector": {"type": "string"}
                        }
                    }
                }
            }),
        ),
        tool(
            "scroll",
            "Scroll the page",
            &["direction", "amount"],
            json!({
                "direction": {"type": "string", "enum": ["up", "down"]},
                "amount": {"type": "integer", "description": "Pixels to scroll"}
            }),
        ),
        tool(
            "wait",
            "Wait for a specified time",
            &["milliseconds"],
            json!({"milliseconds": {"type": "integer", "description": "Time to wait in ms"}}),
        ),
        tool("complete", "Mark the task as complete", &[], json!({})),
        tool(
            "batch_actions",
            "Execute multiple atomic steps in one trusted macro",
            &["steps"],
            json!({
                "steps": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 12,
                    "items": {
                        "type": "object",
                        "properties": {
                            "op": {
                                "type": "string",
                                "enum": ["click", "insert_text", "press_key", "wait_selector", "scroll_by"]
                            },
                            "selector": {"type": "string"},
                            "encodedId": {"type": "string"},
                            "text": {"type": "string"},
                            "key": {"type": "string"},
                            "waitSelector": {"type": "string"},
                            "dx": {"type": "number"},
                            "dy": {"type": "number"}
                        },
                        "required": ["op"]
                    }
                }
            }),
        ),
    ]
}

fn tool(name: &str, description: &str, required: &[&str], properties: Value) -> Tool {
    Tool {
        name: name.to_string(),
        description: description.to_string(),
        parameters: ToolParameters {
            required: required.iter().map(|value| (*value).to_string()).collect(),
            properties,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_and_parses_tool_calls() {
        let tools = allowed_tool_values(&["click", "complete"]);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["function"]["name"], "click");

        let response = json!({
            "choices": [{"message": {"tool_calls": [
                {"function": {"name": "click", "arguments": "{\"selector\":\"#go\"}"}},
                {"function": {"name": "complete", "arguments": "{}"}}
            ]}}]
        });
        let calls = parse_tool_calls(&response).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments["selector"], "#go");
        assert!(parse_tool_calls(&json!({"choices": []})).is_err());
    }
}
