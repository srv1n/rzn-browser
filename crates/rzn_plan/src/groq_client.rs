use crate::{llm_provider::LLMProvider, PlanError, PlanResult};
use async_trait::async_trait;
use log::debug;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

/// Groq client using OpenAI-compatible Chat Completions API
pub struct GroqClient {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl GroqClient {
    pub fn new(api_key: String, model: String, timeout_secs: u64) -> PlanResult<Self> {
        if api_key.is_empty() {
            return Err(PlanError::LLMError("Groq API key not provided".to_string()));
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(PlanError::HttpError)?;

        Ok(Self {
            client,
            api_key,
            model,
            base_url: "https://api.groq.com/openai/v1".to_string(),
        })
    }
}

#[async_trait]
impl LLMProvider for GroqClient {
    fn provider_name(&self) -> &str {
        "Groq"
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
        debug!("Groq chat completion request with model: {}", self.model);

        let mut req_body = json!({
            "model": self.model,
            "messages": messages,
            "temperature": temperature,
        });

        if let Some(max) = max_tokens {
            req_body["max_tokens"] = json!(max);
        }
        if let Some(t) = tools {
            req_body["tools"] = json!(t);
        }
        if let Some(choice) = tool_choice {
            req_body["tool_choice"] = choice;
        }

        let url = format!("{}/chat/completions", self.base_url);
        let resp = self
            .client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&req_body)
            .send()
            .await
            .map_err(PlanError::HttpError)?;

        if !resp.status().is_success() {
            let text = resp
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(PlanError::LLMError(format!("Groq API error: {}", text)));
        }

        let json: Value = resp.json().await.map_err(PlanError::HttpError)?;
        Ok(json)
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
