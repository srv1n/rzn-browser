use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::Write;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: Value,
}

pub struct ToolOnlyLLMClient {
    client: reqwest::Client,
    api_key: String,
    correlation_id: String,
    log_file: String,
}

impl ToolOnlyLLMClient {
    pub fn new(api_key: String, correlation_id: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            correlation_id: correlation_id.clone(),
            log_file: format!("/tmp/llm_raw_{}.jsonl", correlation_id),
        }
    }

    pub async fn call_with_tools(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        tools: Vec<Tool>,
        allowed_tools: Vec<&str>,
    ) -> Result<Vec<ToolCall>, String> {
        // Filter tools to only allowed ones
        let tools: Vec<Tool> = tools
            .into_iter()
            .filter(|t| allowed_tools.contains(&t.name.as_str()))
            .collect();

        // New OpenAI models reject 'max_tokens'; prefer 'max_completion_tokens' or omit entirely
        let model = "gpt-5-mini-2025-08-07";
        let mut request_body = json!({
            "model": model,
            "temperature": 0.0,  // CRITICAL: Deterministic output
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt}
            ],
            "tools": tools.iter().map(|t| json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": {
                        "type": "object",
                        "required": t.parameters.required,
                        "properties": t.parameters.properties
                    }
                }
            })).collect::<Vec<_>>(),
            "tool_choice": "required"
        });

        // Only send token limit in the compatible field
        if model.starts_with("gpt-5-") || model.starts_with("o1-") || model.starts_with("o4-") {
            request_body["max_completion_tokens"] = json!(1000);
        } else {
            request_body["max_tokens"] = json!(1000);
        }

        // Log raw request
        self.log_raw("request", &request_body);

        let response = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        let response_text = response
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {}", e))?;

        let response_json: Value = serde_json::from_str(&response_text)
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        // Log raw response
        self.log_raw("response", &response_json);

        // Extract tool calls
        let tool_calls = response_json
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("tool_calls"))
            .and_then(|tc| tc.as_array())
            .ok_or("No tool calls in response")?;

        let mut results = Vec::new();
        for tool_call in tool_calls {
            let name = tool_call
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .ok_or("Missing tool name")?
                .to_string();

            let arguments_str = tool_call
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|a| a.as_str())
                .ok_or("Missing tool arguments")?;

            let arguments: Value = serde_json::from_str(arguments_str)
                .map_err(|e| format!("Failed to parse tool arguments: {}", e))?;

            results.push(ToolCall { name, arguments });
        }

        Ok(results)
    }

    fn log_raw(&self, event_type: &str, data: &Value) {
        let log_entry = json!({
            "timestamp": Utc::now().to_rfc3339(),
            "correlation_id": self.correlation_id,
            "event_type": event_type,
            "data": data
        });

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_file)
        {
            let _ = writeln!(file, "{}", log_entry.to_string());
        }

        log::debug!(
            "[{}] LLM {}: {}",
            self.correlation_id,
            event_type,
            serde_json::to_string_pretty(data).unwrap_or_default()
        );
    }

    pub fn get_standard_tools() -> Vec<Tool> {
        vec![
            Tool {
                name: "navigate".to_string(),
                description: "Navigate to a URL".to_string(),
                parameters: ToolParameters {
                    required: vec!["url".to_string()],
                    properties: json!({
                        "url": {"type": "string", "description": "The URL to navigate to"}
                    }),
                },
            },
            Tool {
                name: "type".to_string(),
                description: "Type text into an input field".to_string(),
                parameters: ToolParameters {
                    required: vec!["selector".to_string(), "text".to_string()],
                    properties: json!({
                        "selector": {"type": "string", "description": "CSS selector for the input"},
                        "text": {"type": "string", "description": "Text to type"}
                    }),
                },
            },
            Tool {
                name: "press_key".to_string(),
                description: "Press a keyboard key".to_string(),
                parameters: ToolParameters {
                    required: vec!["key".to_string()],
                    properties: json!({
                        "key": {
                            "type": "string",
                            "enum": ["Enter", "Tab", "Escape", "ArrowUp", "ArrowDown"],
                            "description": "The key to press"
                        }
                    }),
                },
            },
            Tool {
                name: "click".to_string(),
                description: "Click on an element".to_string(),
                parameters: ToolParameters {
                    required: vec!["selector".to_string()],
                    properties: json!({
                        "selector": {"type": "string", "description": "CSS selector for the element"}
                    }),
                },
            },
            Tool {
                name: "extract".to_string(),
                description: "Extract data from the page".to_string(),
                parameters: ToolParameters {
                    required: vec!["fields".to_string()],
                    properties: json!({
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
                },
            },
            Tool {
                name: "scroll".to_string(),
                description: "Scroll the page".to_string(),
                parameters: ToolParameters {
                    required: vec!["direction".to_string(), "amount".to_string()],
                    properties: json!({
                        "direction": {"type": "string", "enum": ["up", "down"]},
                        "amount": {"type": "integer", "description": "Pixels to scroll"}
                    }),
                },
            },
            Tool {
                name: "wait".to_string(),
                description: "Wait for a specified time".to_string(),
                parameters: ToolParameters {
                    required: vec!["milliseconds".to_string()],
                    properties: json!({
                        "milliseconds": {"type": "integer", "description": "Time to wait in ms"}
                    }),
                },
            },
            Tool {
                name: "complete".to_string(),
                description: "Mark the task as complete".to_string(),
                parameters: ToolParameters {
                    required: vec![],
                    properties: json!({}),
                },
            },
            Tool {
                name: "batch_actions".to_string(),
                description: "Execute multiple atomic steps in one trusted macro".to_string(),
                parameters: ToolParameters {
                    required: vec!["steps".to_string()],
                    properties: json!({
                        "steps": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 12,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "op": {
                                        "type": "string",
                                        "enum": ["click","insert_text","press_key","wait_selector","scroll_by"],
                                        "description": "The operation to perform"
                                    },
                                    "selector": {
                                        "type": "string",
                                        "description": "CSS or deep selector (>>>). Optional if encodedId provided."
                                    },
                                    "encodedId": {
                                        "type": "string",
                                        "description": "frameId:backendNodeId. Preferred when available."
                                    },
                                    "text": {
                                        "type": "string",
                                        "description": "Text to insert for insert_text operation"
                                    },
                                    "key": {
                                        "type": "string",
                                        "description": "Key to press for press_key operation"
                                    },
                                    "waitSelector": {
                                        "type": "string",
                                        "description": "Selector to wait for in wait_selector operation"
                                    },
                                    "dx": {
                                        "type": "number",
                                        "description": "Horizontal scroll amount for scroll_by"
                                    },
                                    "dy": {
                                        "type": "number",
                                        "description": "Vertical scroll amount for scroll_by"
                                    }
                                },
                                "required": ["op"]
                            }
                        }
                    }),
                },
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_tools() {
        let tools = ToolOnlyLLMClient::get_standard_tools();
        assert!(tools.iter().any(|t| t.name == "navigate"));
        assert!(tools.iter().any(|t| t.name == "press_key"));
        assert!(tools.iter().any(|t| t.name == "type"));
    }

    #[test]
    fn test_tool_filtering() {
        let tools = ToolOnlyLLMClient::get_standard_tools();
        let allowed = vec!["type", "press_key"];

        let filtered: Vec<Tool> = tools
            .into_iter()
            .filter(|t| allowed.contains(&t.name.as_str()))
            .collect();

        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|t| allowed.contains(&t.name.as_str())));
    }
}
