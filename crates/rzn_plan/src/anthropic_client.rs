use crate::{llm_provider::LLMProvider, PlanError, PlanResult};
use async_trait::async_trait;
use log::debug;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic client using Messages API with tool use
pub struct AnthropicClient {
    client: Client,
    api_key: String,
    model: String,
}

impl AnthropicClient {
    pub fn new(api_key: String, model: String, timeout_secs: u64) -> PlanResult<Self> {
        if api_key.is_empty() {
            return Err(PlanError::LLMError(
                "Anthropic API key not provided".to_string(),
            ));
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| PlanError::HttpError(e))?;
        Ok(Self {
            client,
            api_key,
            model,
        })
    }

    fn convert_messages(
        &self,
        openai_messages: Vec<Value>,
    ) -> PlanResult<(Option<String>, Vec<Value>)> {
        let mut system_parts: Vec<String> = Vec::new();
        let mut messages: Vec<Value> = Vec::new();

        for m in openai_messages {
            let role = m
                .get("role")
                .and_then(|r| r.as_str())
                .ok_or_else(|| PlanError::LLMError("Message missing role".to_string()))?;
            let content = m.get("content").and_then(|c| c.as_str()).unwrap_or("");
            match role {
                "system" => system_parts.push(content.to_string()),
                "user" => messages.push(json!({
                    "role": "user",
                    "content": [{"type": "text", "text": content}]
                })),
                "assistant" => messages.push(json!({
                    "role": "assistant",
                    "content": [{"type": "text", "text": content}]
                })),
                other => return Err(PlanError::LLMError(format!("Unsupported role: {}", other))),
            }
        }

        let system = if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n"))
        };
        Ok((system, messages))
    }

    fn convert_tools(&self, openai_tools: Vec<Value>) -> PlanResult<Vec<Value>> {
        let mut tools: Vec<Value> = Vec::new();
        for tool in openai_tools {
            if tool.get("type").and_then(|t| t.as_str()) == Some("function") {
                if let Some(function) = tool.get("function") {
                    let name = function
                        .get("name")
                        .and_then(|n| n.as_str())
                        .ok_or_else(|| PlanError::LLMError("Function missing name".to_string()))?;
                    let description = function
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("");
                    let parameters = function
                        .get("parameters")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    tools.push(json!({
                        "name": name,
                        "description": description,
                        "input_schema": parameters
                    }));
                }
            }
        }
        Ok(tools)
    }

    fn convert_response_to_openai(&self, anthropic_response: Value) -> PlanResult<Value> {
        // Expect { role: "assistant", content: [ {type: ..., ...}, ... ] }
        let content = anthropic_response
            .get("content")
            .and_then(|c| c.as_array())
            .ok_or_else(|| {
                PlanError::LLMError("Missing content in Anthropic response".to_string())
            })?;

        // Aggregate tool uses and text
        let mut tool_calls: Vec<Value> = Vec::new();
        let mut texts: Vec<String> = Vec::new();
        for part in content {
            if let Some(t) = part.get("type").and_then(|t| t.as_str()) {
                match t {
                    "tool_use" => {
                        let name = part.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        let args = part.get("input").cloned().unwrap_or_else(|| json!({}));
                        tool_calls.push(json!({
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string())
                            }
                        }));
                    }
                    "text" => {
                        if let Some(txt) = part.get("text").and_then(|x| x.as_str()) {
                            texts.push(txt.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }

        let content_text = if texts.is_empty() {
            None
        } else {
            Some(texts.join("\n"))
        };
        let mut message = json!({ "role": "assistant" });
        if let Some(t) = content_text {
            message["content"] = json!(t);
        } else {
            message["content"] = Value::Null;
        }
        if !tool_calls.is_empty() {
            message["tool_calls"] = json!(tool_calls);
        }

        Ok(json!({ "choices": [{ "message": message }] }))
    }
}

#[async_trait]
impl LLMProvider for AnthropicClient {
    fn provider_name(&self) -> &str {
        "Anthropic"
    }
    fn model_name(&self) -> &str {
        &self.model
    }

    async fn chat_completion(
        &self,
        messages: Vec<Value>,
        temperature: f32,
        tools: Option<Vec<Value>>,
        tool_choice: Option<Value>,
        max_tokens: Option<u32>,
    ) -> PlanResult<Value> {
        debug!("Anthropic messages request with model: {}", self.model);

        let (system, msgs) = self.convert_messages(messages)?;
        let mut body = json!({
            "model": self.model,
            "messages": msgs,
            "temperature": temperature,
            "max_tokens": max_tokens.unwrap_or(1000),
        });

        if let Some(sys) = system {
            body["system"] = json!(sys);
        }
        if let Some(t) = tools {
            body["tools"] = json!(self.convert_tools(t)?);
        }
        if let Some(tc) = tool_choice {
            // Map OpenAI-style tool_choice to Anthropic
            if tc.get("type").and_then(|x| x.as_str()) == Some("function") {
                if let Some(name) = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                {
                    body["tool_choice"] = json!({"type": "tool", "name": name});
                }
            } else if tc.get("type").and_then(|x| x.as_str()) == Some("auto") {
                body["tool_choice"] = json!("auto");
            }
        }

        let resp = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| PlanError::HttpError(e))?;

        if !resp.status().is_success() {
            let text = resp
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(PlanError::LLMError(format!(
                "Anthropic API error: {}",
                text
            )));
        }

        let json: Value = resp.json().await.map_err(|e| PlanError::HttpError(e))?;
        self.convert_response_to_openai(json)
    }

    async fn simple_chat(
        &self,
        messages: Vec<Value>,
        temperature: Option<f32>,
    ) -> PlanResult<String> {
        let temp = temperature.unwrap_or(0.7);
        let response = self
            .chat_completion(messages, temp, None, None, Some(1000))
            .await?;
        let content = response["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_convert_tools() {
        let client =
            AnthropicClient::new("key".into(), "claude-3-5-sonnet-latest".into(), 30).unwrap();
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "do_thing",
                "description": "Do a thing",
                "parameters": {"type": "object", "properties": {"x": {"type": "string"}}}
            }
        })];
        let converted = client.convert_tools(tools).unwrap();
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["name"], "do_thing");
        assert!(converted[0]["input_schema"].is_object());
    }
}
