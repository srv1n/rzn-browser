use crate::PlanResult;
use async_trait::async_trait;
use serde_json::Value;

/// Trait for abstracting LLM provider operations
#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Get the provider name
    fn provider_name(&self) -> &str;

    /// Get the current model name
    fn model_name(&self) -> &str;

    /// Send a chat completion request
    async fn chat_completion(
        &self,
        messages: Vec<Value>,
        temperature: f32,
        tools: Option<Vec<Value>>,
        tool_choice: Option<Value>,
        max_tokens: Option<u32>,
    ) -> PlanResult<Value>;

    /// Simple chat method for external use
    async fn simple_chat(
        &self,
        messages: Vec<Value>,
        temperature: Option<f32>,
    ) -> PlanResult<String>;

    /// OpenAI Responses-compatible completion (optional; default unsupported)
    async fn responses_completion(
        &self,
        _input: Vec<Value>,
        _temperature: f32,
        _tools: Option<Vec<Value>>,
        _tool_choice: Option<Value>,
        _response_format: Option<Value>,
        _max_tokens: Option<u32>,
    ) -> PlanResult<Value> {
        Err(crate::PlanError::LLMError(
            "responses_completion not supported by this provider".to_string(),
        ))
    }
}

/// Supported LLM providers
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderType {
    OpenAI,
    Gemini,
    Anthropic,
    Groq,
    Dummy,
    ClaudeCli,
    GeminiCli,
    CodexCli,
}

impl ProviderType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "openai" => Some(ProviderType::OpenAI),
            "gemini" | "google" => Some(ProviderType::Gemini),
            "anthropic" | "claude" => Some(ProviderType::Anthropic),
            "groq" => Some(ProviderType::Groq),
            "dummy" => Some(ProviderType::Dummy),
            "claude-cli" | "claude_cli" | "claude-code" | "claude_code" => {
                Some(ProviderType::ClaudeCli)
            }
            "gemini-cli" | "gemini_cli" => Some(ProviderType::GeminiCli),
            "codex-cli" | "codex_cli" => Some(ProviderType::CodexCli),
            _ => None,
        }
    }

    pub fn requires_api_key(&self) -> bool {
        match self {
            ProviderType::OpenAI
            | ProviderType::Gemini
            | ProviderType::Anthropic
            | ProviderType::Groq => true,
            ProviderType::Dummy
            | ProviderType::ClaudeCli
            | ProviderType::GeminiCli
            | ProviderType::CodexCli => false,
        }
    }
}
