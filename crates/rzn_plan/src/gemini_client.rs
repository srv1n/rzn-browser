use crate::{llm_provider::LLMProvider, PlanError, PlanResult};
use async_trait::async_trait;
use log::debug;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

/// Gemini API client for Google's AI models
pub struct GeminiClient {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
}

/// Gemini content part structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiContentPart {
    pub text: String,
}

/// Gemini content structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiContent {
    pub role: String,
    pub parts: Vec<GeminiContentPart>,
}

/// Gemini function declaration
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiFunctionDeclaration {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Gemini tool declaration
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiTool {
    pub function_declarations: Vec<GeminiFunctionDeclaration>,
}

/// Gemini function call
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiFunctionCall {
    pub name: String,
    pub args: Value,
}

/// Gemini function call part
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiFunctionCallPart {
    pub function_call: GeminiFunctionCall,
}

impl GeminiClient {
    pub fn new(api_key: String, model: String, timeout_secs: u64) -> PlanResult<Self> {
        if api_key.is_empty() {
            return Err(PlanError::LLMError(
                "Gemini API key not provided".to_string(),
            ));
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| PlanError::HttpError(e))?;

        // Use the v1beta API endpoint for function calling support
        let base_url = "https://generativelanguage.googleapis.com/v1beta".to_string();

        Ok(Self {
            client,
            api_key,
            model,
            base_url,
        })
    }

    /// Convert OpenAI messages format to Gemini format
    fn convert_messages(&self, openai_messages: Vec<Value>) -> PlanResult<Vec<GeminiContent>> {
        let mut gemini_contents = Vec::new();

        for msg in openai_messages {
            let role = msg
                .get("role")
                .and_then(|r| r.as_str())
                .ok_or_else(|| PlanError::LLMError("Message missing role".to_string()))?;

            let content = msg
                .get("content")
                .and_then(|c| c.as_str())
                .ok_or_else(|| PlanError::LLMError("Message missing content".to_string()))?;

            // Convert OpenAI roles to Gemini roles
            let gemini_role = match role {
                "system" => "user", // Gemini doesn't have system role, merge with user
                "user" => "user",
                "assistant" => "model",
                _ => return Err(PlanError::LLMError(format!("Unknown role: {}", role))),
            };

            // For system messages, prepend a note to the content
            let final_content = if role == "system" {
                format!("System instruction: {}", content)
            } else {
                content.to_string()
            };

            gemini_contents.push(GeminiContent {
                role: gemini_role.to_string(),
                parts: vec![GeminiContentPart {
                    text: final_content,
                }],
            });
        }

        // Merge consecutive user messages (since we converted system to user)
        let mut merged_contents = Vec::new();
        let mut current_user_parts = Vec::new();
        let mut in_user_block = false;

        for content in gemini_contents {
            if content.role == "user" {
                in_user_block = true;
                current_user_parts.extend(content.parts);
            } else {
                if in_user_block {
                    merged_contents.push(GeminiContent {
                        role: "user".to_string(),
                        parts: current_user_parts.clone(),
                    });
                    current_user_parts.clear();
                    in_user_block = false;
                }
                merged_contents.push(content);
            }
        }

        // Don't forget the last user block
        if in_user_block && !current_user_parts.is_empty() {
            merged_contents.push(GeminiContent {
                role: "user".to_string(),
                parts: current_user_parts,
            });
        }

        Ok(merged_contents)
    }

    /// Convert OpenAI tools format to Gemini tools format
    fn convert_tools(&self, openai_tools: Vec<Value>) -> PlanResult<Vec<GeminiTool>> {
        let mut function_declarations = Vec::new();

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

                    function_declarations.push(GeminiFunctionDeclaration {
                        name: name.to_string(),
                        description: description.to_string(),
                        parameters,
                    });
                }
            }
        }

        if function_declarations.is_empty() {
            Ok(vec![])
        } else {
            Ok(vec![GeminiTool {
                function_declarations,
            }])
        }
    }

    /// Convert Gemini response to OpenAI format
    fn convert_response(&self, gemini_response: Value) -> PlanResult<Value> {
        let candidates = gemini_response
            .get("candidates")
            .and_then(|c| c.as_array())
            .ok_or_else(|| PlanError::LLMError("No candidates in Gemini response".to_string()))?;

        if candidates.is_empty() {
            return Err(PlanError::LLMError("Empty candidates array".to_string()));
        }

        let candidate = &candidates[0];
        let content = candidate
            .get("content")
            .ok_or_else(|| PlanError::LLMError("No content in candidate".to_string()))?;

        let parts = content
            .get("parts")
            .and_then(|p| p.as_array())
            .ok_or_else(|| PlanError::LLMError("No parts in content".to_string()))?;

        if parts.is_empty() {
            return Err(PlanError::LLMError("Empty parts array".to_string()));
        }

        let part = &parts[0];

        // Check if this is a function call response
        if let Some(function_call) = part.get("functionCall") {
            let name = function_call
                .get("name")
                .and_then(|n| n.as_str())
                .ok_or_else(|| PlanError::LLMError("Function call missing name".to_string()))?;

            let args = function_call
                .get("args")
                .cloned()
                .unwrap_or_else(|| json!({}));

            // Convert to OpenAI format
            Ok(json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": serde_json::to_string(&args)
                                    .unwrap_or_else(|_| "{}".to_string())
                            }
                        }]
                    }
                }]
            }))
        } else if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
            // Regular text response
            Ok(json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": text
                    }
                }]
            }))
        } else {
            Err(PlanError::LLMError(
                "Unknown response format from Gemini".to_string(),
            ))
        }
    }
}

#[async_trait]
impl LLMProvider for GeminiClient {
    fn provider_name(&self) -> &str {
        "Gemini"
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
        debug!("Gemini chat completion request");

        // Convert messages
        let gemini_contents = self.convert_messages(messages)?;

        // Build request body
        let mut request_body = json!({
            "contents": gemini_contents,
            "generationConfig": {
                "temperature": temperature,
                "topP": 0.95,
                "topK": 40,
            }
        });

        // Add max tokens if specified
        if let Some(max_tokens) = max_tokens {
            request_body["generationConfig"]["maxOutputTokens"] = json!(max_tokens);
        }

        // Convert and add tools if provided
        if let Some(tools) = tools {
            let gemini_tools = self.convert_tools(tools)?;
            if !gemini_tools.is_empty() {
                request_body["tools"] = json!(gemini_tools);

                // Handle tool choice (Gemini uses toolConfig)
                if let Some(tool_choice) = tool_choice {
                    if let Some(tc_type) = tool_choice.get("type").and_then(|t| t.as_str()) {
                        match tc_type {
                            "function" => {
                                // Force specific function
                                if let Some(function_name) = tool_choice
                                    .get("function")
                                    .and_then(|f| f.get("name"))
                                    .and_then(|n| n.as_str())
                                {
                                    request_body["toolConfig"] = json!({
                                        "functionCallingConfig": {
                                            "mode": "ANY",
                                            "allowedFunctionNames": [function_name]
                                        }
                                    });
                                }
                            }
                            "auto" => {
                                request_body["toolConfig"] = json!({
                                    "functionCallingConfig": {
                                        "mode": "AUTO"
                                    }
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, self.model, self.api_key
        );

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| PlanError::HttpError(e))?;

        if !response.status().is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(PlanError::LLMError(format!(
                "Gemini API request failed: {}",
                error_text
            )));
        }

        let gemini_response: Value = response.json().await.map_err(|e| PlanError::HttpError(e))?;

        // Check for API errors
        if let Some(error) = gemini_response.get("error") {
            let error_msg = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown Gemini API error");
            return Err(PlanError::LLMError(format!(
                "Gemini API error: {}",
                error_msg
            )));
        }

        // Convert response to OpenAI format
        self.convert_response(gemini_response)
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

    #[test]
    fn test_message_conversion() {
        let client =
            GeminiClient::new("test-key".to_string(), "gemini-2.0-flash".to_string(), 30).unwrap();

        let openai_messages = vec![
            json!({
                "role": "system",
                "content": "You are a helpful assistant."
            }),
            json!({
                "role": "user",
                "content": "Hello!"
            }),
        ];

        let gemini_messages = client.convert_messages(openai_messages).unwrap();
        assert_eq!(gemini_messages.len(), 1); // System and user merged
        assert_eq!(gemini_messages[0].role, "user");
        assert_eq!(gemini_messages[0].parts.len(), 2);
    }
}
