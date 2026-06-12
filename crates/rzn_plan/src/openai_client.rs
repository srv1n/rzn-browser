use crate::{llm_provider::LLMProvider, PlanError, PlanResult};
use async_trait::async_trait;
use log::debug;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

/// OpenAI API client implementation
pub struct OpenAIClient {
    client: Client,
    api_key: String,
    model: String,
}

impl OpenAIClient {
    pub fn new(api_key: String, model: String, timeout_secs: u64) -> PlanResult<Self> {
        if api_key.is_empty() {
            return Err(PlanError::LLMError(
                "OpenAI API key not provided".to_string(),
            ));
        }

        // Basic API key format validation (skip for dummy keys)
        if !api_key.starts_with("sk-") && api_key != "dummy-key-for-non-llm-mode" {
            return Err(PlanError::LLMError(
                "Invalid OpenAI API key format. API keys should start with 'sk-'".to_string(),
            ));
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(PlanError::HttpError)?;

        Ok(Self {
            client,
            api_key,
            model,
        })
    }

    fn simple_chat_content_from_response(response: &Value) -> PlanResult<String> {
        let choice = response
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|choices| choices.first())
            .ok_or_else(|| {
                PlanError::LLMError("OpenAI simple_chat response missing choices".to_string())
            })?;

        let message = choice.get("message").ok_or_else(|| {
            PlanError::LLMError("OpenAI simple_chat response missing message".to_string())
        })?;

        if let Some(refusal) = message
            .get("refusal")
            .and_then(|r| r.as_str())
            .filter(|r| !r.trim().is_empty())
        {
            let finish_reason = choice
                .get("finish_reason")
                .and_then(|f| f.as_str())
                .unwrap_or("unknown");
            return Err(PlanError::LLMError(format!(
                "OpenAI simple_chat refused content: {} (finish_reason: {})",
                refusal, finish_reason
            )));
        }

        if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
            return Ok(content.to_string());
        }

        let finish_reason = choice
            .get("finish_reason")
            .and_then(|f| f.as_str())
            .unwrap_or("unknown");
        Err(PlanError::LLMError(format!(
            "OpenAI simple_chat missing content (finish_reason: {})",
            finish_reason
        )))
    }
}

#[async_trait]
impl LLMProvider for OpenAIClient {
    fn provider_name(&self) -> &str {
        "OpenAI"
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
        debug!("OpenAI chat completion request with model: {}", self.model);

        // Handle model-specific temperature requirements
        let adjusted_temperature = if self.model.starts_with("o1-")
            || self.model.starts_with("o4-")
            || self.model.starts_with("gpt-5-")
        {
            // o1, o4, and gpt-5 family only support the default temperature (1.0)
            1.0
        } else {
            temperature
        };

        let mut request_body = json!({
            "model": self.model,
            "messages": messages,
            "temperature": adjusted_temperature,
        });

        // Add tools if provided
        if let Some(tools) = tools {
            request_body["tools"] = json!(tools);
        }

        // Add tool choice if provided
        if let Some(tool_choice) = tool_choice {
            request_body["tool_choice"] = tool_choice;
        }

        // Add max tokens (env override: OPENAI_MAX_TOKENS)
        let env_max: Option<u32> = std::env::var("OPENAI_MAX_TOKENS")
            .ok()
            .and_then(|s| s.parse().ok());
        let max = max_tokens.or(env_max).unwrap_or(1000);
        // Some models (o1/o4/gpt-5 family) require 'max_completion_tokens' instead of 'max_tokens'
        let use_max_completion = self.model.starts_with("o1-")
            || self.model.starts_with("o4-")
            || self.model.starts_with("gpt-5-")
            || self.model.contains("reasoning");
        if use_max_completion {
            request_body["max_completion_tokens"] = json!(max);
        } else {
            request_body["max_tokens"] = json!(max);
        }

        // Prefer JSON object responses when possible
        request_body["response_format"] = json!({ "type": "json_object" });

        let response = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(PlanError::HttpError)?;

        if !response.status().is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            // Retry once by flipping the token parameter if the error indicates the wrong one
            let mut retried = false;
            let mut request_body_retry = request_body.clone();
            if error_text.contains("Use 'max_completion_tokens' instead") {
                request_body_retry
                    .as_object_mut()
                    .unwrap()
                    .remove("max_tokens");
                request_body_retry["max_completion_tokens"] = json!(max);
                retried = true;
            } else if error_text.contains("Use 'max_tokens' instead") {
                request_body_retry
                    .as_object_mut()
                    .unwrap()
                    .remove("max_completion_tokens");
                request_body_retry["max_tokens"] = json!(max);
                retried = true;
            }

            if retried {
                let retry_resp = self
                    .client
                    .post("https://api.openai.com/v1/chat/completions")
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .header("Content-Type", "application/json")
                    .json(&request_body_retry)
                    .send()
                    .await
                    .map_err(PlanError::HttpError)?;
                if !retry_resp.status().is_success() {
                    let err2 = retry_resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    return Err(PlanError::LLMError(format!(
                        "OpenAI API request failed: {}",
                        err2
                    )));
                }
                let response_json: Value = retry_resp.json().await.map_err(PlanError::HttpError)?;
                return Ok(response_json);
            } else {
                return Err(PlanError::LLMError(format!(
                    "OpenAI API request failed: {}",
                    error_text
                )));
            }
        }

        let response_json: Value = response.json().await.map_err(PlanError::HttpError)?;

        // Check for API errors
        if let Some(error) = response_json.get("error") {
            let error_msg = error["message"].as_str().unwrap_or("Unknown API error");
            let error_type = error["type"].as_str().unwrap_or("unknown_error");

            // For invalid API key or authentication errors, fail immediately
            if error_type == "invalid_request_error" && error_msg.contains("API key") {
                return Err(PlanError::LLMError("Invalid OpenAI API key. Please check your OPENAI_API_KEY environment variable.".to_string()));
            }

            return Err(PlanError::LLMError(format!(
                "OpenAI API error: {}",
                error_msg
            )));
        }

        Ok(response_json)
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

        Self::simple_chat_content_from_response(&response)
    }

    async fn responses_completion(
        &self,
        input: Vec<Value>,
        temperature: f32,
        tools: Option<Vec<Value>>,
        tool_choice: Option<Value>,
        _response_format: Option<Value>,
        max_tokens: Option<u32>,
    ) -> PlanResult<Value> {
        debug!(
            "OpenAI responses completion request with model: {}",
            self.model
        );

        // Map messages -> input for Responses API if needed
        let mut req_body = json!({
            "model": self.model,
            "input": input,
        });

        // Only include temperature for classic chat models; newer families (o1/o4/gpt-5/reasoning) reject it
        let is_fixed_temp = self.model.starts_with("o1-")
            || self.model.starts_with("o4-")
            || self.model.starts_with("gpt-5-")
            || self.model.contains("reasoning");
        if !is_fixed_temp {
            req_body["temperature"] = json!(temperature);
        }

        // Add tools/tool_choice if provided (convert to Responses API schema)
        if let Some(t) = tools {
            let converted = convert_tools_for_responses(t);
            req_body["tools"] = json!(converted);
        }
        if let Some(choice) = tool_choice {
            req_body["tool_choice"] = convert_tool_choice_for_responses(choice);
        }

        // JSON object output format (Responses API expects 'text.format')
        req_body["text"] = json!({ "format": { "type": "json_object" } });
        // Reasoning controls (optional, tolerated by API)
        req_body["reasoning"] = json!({ "effort": "low" });

        // Nudge the model to lower reasoning effort where supported
        req_body["reasoning"] = json!({ "effort": "low" });

        // Token limits: Responses prefers max_output_tokens; fall back to max_completion_tokens on error
        let env_max: Option<u32> = std::env::var("OPENAI_MAX_TOKENS")
            .ok()
            .and_then(|s| s.parse().ok());
        let max = max_tokens.or(env_max).unwrap_or(1000);
        req_body["max_output_tokens"] = json!(max);

        // Execute
        // Defensive: ensure no stray 'response_format' key (older code or upstream param)
        if let Some(obj) = req_body.as_object_mut() {
            obj.remove("response_format");
        }

        let resp = self
            .client
            .post("https://api.openai.com/v1/responses")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&req_body)
            .send()
            .await
            .map_err(crate::PlanError::HttpError)?;

        if !resp.status().is_success() {
            let error_text = resp
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            // Retry flipping tokens param if server asks for a different param name
            let mut retry_body = req_body.clone();
            let mut retried = false;
            if error_text.contains("max_completion_tokens") {
                retry_body
                    .as_object_mut()
                    .unwrap()
                    .remove("max_output_tokens");
                retry_body["max_completion_tokens"] = json!(max);
                retried = true;
            }
            // Handle legacy input block name (input_text -> text)
            if error_text.contains("Invalid value: 'input_text'")
                || error_text.to_lowercase().contains("input_text")
            {
                if let Some(arr) = retry_body.get_mut("input").and_then(|v| v.as_array_mut()) {
                    for item in arr {
                        if let Some(contents) =
                            item.get_mut("content").and_then(|c| c.as_array_mut())
                        {
                            for block in contents {
                                if block.get("type").and_then(|t| t.as_str()) == Some("input_text")
                                {
                                    block["type"] = json!("text");
                                }
                            }
                        }
                    }
                }
                retried = true;
            }
            // If server complains about 'response_format', strip it and set text.format (belt and suspenders)
            if error_text
                .to_lowercase()
                .contains("unsupported parameter: 'response_format'")
                || error_text.to_lowercase().contains("unsupported_parameter")
            {
                if let Some(obj) = retry_body.as_object_mut() {
                    obj.remove("response_format");
                    if !obj.contains_key("text") {
                        obj.insert(
                            "text".to_string(),
                            json!({ "format": { "type": "json_object" } }),
                        );
                    }
                }
                retried = true;
            }

            if retried {
                let resp2 = self
                    .client
                    .post("https://api.openai.com/v1/responses")
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .header("Content-Type", "application/json")
                    .json(&retry_body)
                    .send()
                    .await
                    .map_err(crate::PlanError::HttpError)?;
                if !resp2.status().is_success() {
                    let err2 = resp2
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    return Err(crate::PlanError::LLMError(format!(
                        "OpenAI Responses API failed: {}",
                        err2
                    )));
                }
                let json: Value = resp2.json().await.map_err(crate::PlanError::HttpError)?;
                return Ok(json);
            }
            return Err(crate::PlanError::LLMError(format!(
                "OpenAI Responses API failed: {}",
                error_text
            )));
        }

        let mut json: Value = resp.json().await.map_err(crate::PlanError::HttpError)?;

        // Auto-continue if output was truncated by token limit
        if json.get("status").and_then(|s| s.as_str()) == Some("incomplete") {
            let reason = json
                .get("incomplete_details")
                .and_then(|d| d.get("reason"))
                .and_then(|r| r.as_str())
                .unwrap_or("");
            if reason.contains("max_output_tokens") {
                // Increase token limit and retry once (cap at 4096 to protect cost)
                let current = req_body
                    .get("max_output_tokens")
                    .and_then(|v| v.as_u64())
                    .or_else(|| {
                        req_body
                            .get("max_completion_tokens")
                            .and_then(|v| v.as_u64())
                    })
                    .unwrap_or(1000);
                let bumped = std::cmp::min(current as u32 * 2, 4096);
                if req_body.get("max_output_tokens").is_some() {
                    req_body["max_output_tokens"] = json!(bumped);
                } else if req_body.get("max_completion_tokens").is_some() {
                    req_body["max_completion_tokens"] = json!(bumped);
                } else {
                    req_body["max_output_tokens"] = json!(bumped);
                }

                let resp3 = self
                    .client
                    .post("https://api.openai.com/v1/responses")
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .header("Content-Type", "application/json")
                    .json(&req_body)
                    .send()
                    .await
                    .map_err(crate::PlanError::HttpError)?;
                if !resp3.status().is_success() {
                    let err3 = resp3
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    return Err(crate::PlanError::LLMError(format!(
                        "OpenAI Responses API failed after bump: {}",
                        err3
                    )));
                }
                json = resp3.json().await.map_err(crate::PlanError::HttpError)?;
            }
        }

        Ok(json)
    }
}

/// Convert OpenAI Chat Completions tool schema to Responses API schema
fn convert_tools_for_responses(tools: Vec<Value>) -> Vec<Value> {
    tools
        .into_iter()
        .map(|tool| {
            if tool.get("type").and_then(|t| t.as_str()) == Some("function") {
                // Chat style sometimes nests under 'function'
                if let Some(func) = tool.get("function") {
                    let mut out = serde_json::Map::new();
                    out.insert("type".to_string(), json!("function"));
                    if let Some(name) = func.get("name") {
                        out.insert("name".to_string(), name.clone());
                    }
                    if let Some(desc) = func.get("description") {
                        out.insert("description".to_string(), desc.clone());
                    }
                    if let Some(params) = func.get("parameters") {
                        out.insert("parameters".to_string(), params.clone());
                    }
                    // Encourage strict JSON schema adherence when provided
                    out.insert("strict".to_string(), json!(true));
                    return Value::Object(out);
                }
            }
            // Already in Responses shape or unknown -> pass through
            tool
        })
        .collect()
}

/// Convert Chat-style tool_choice to Responses API shape
fn convert_tool_choice_for_responses(choice: Value) -> Value {
    if let Some(s) = choice.as_str() {
        return json!(s);
    }
    if let Some(obj) = choice.as_object() {
        if obj.get("type").and_then(|v| v.as_str()) == Some("function") {
            // Accept both { type, function: { name } } and { type, name }
            if let Some(func) = obj.get("function").and_then(|f| f.as_object()) {
                if let Some(name) = func.get("name").cloned() {
                    return json!({ "type": "function", "name": name });
                }
            }
            if let Some(name) = obj.get("name").cloned() {
                return json!({ "type": "function", "name": name });
            }
        }
    }
    // Fallback to 'auto'
    json!("auto")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_openai_simple_chat_refusal_is_error() {
        let err = OpenAIClient::simple_chat_content_from_response(&json!({
            "choices": [{
                "finish_reason": "content_filter",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "refusal": "I can't help with that."
                }
            }]
        }))
        .unwrap_err();

        match err {
            PlanError::LLMError(message) => {
                assert!(message.contains("refused content"));
                assert!(message.contains("content_filter"));
                assert!(message.contains("I can't help"));
            }
            other => panic!("expected LLMError, got {other}"),
        }
    }

    #[test]
    fn fix_openai_simple_chat_missing_content_is_error() {
        let err = OpenAIClient::simple_chat_content_from_response(&json!({
            "choices": [{
                "finish_reason": "length",
                "message": {
                    "role": "assistant"
                }
            }]
        }))
        .unwrap_err();

        match err {
            PlanError::LLMError(message) => {
                assert!(message.contains("missing content"));
                assert!(message.contains("length"));
            }
            other => panic!("expected LLMError, got {other}"),
        }
    }
}
