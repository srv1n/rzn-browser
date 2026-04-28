//! Orchestrator crate for planning and executing browser workflows using LLM and broker.

pub mod action_error_handler;
pub mod action_registry;
// Single autonomous planner implementation
pub mod action_surface;
pub mod anthropic_client;
pub mod broker_client;
pub mod cli_client;
pub mod desktop_session;
pub mod desktop_tools;
pub mod dom_analyzer;
pub mod dom_compression;
pub mod dom_context;
pub mod dom_processor;
pub mod dom_representation;
pub mod dom_whitelist;
pub mod dummy_client;
pub mod element_ref;
pub mod failure_cache;
pub mod failure_recovery;
pub mod gemini_client;
pub mod groq_client;
pub mod llm;
pub mod llm_autonomous;
pub mod llm_provider;
pub mod mode_selector;
pub mod openai_client;
pub mod orchestrator;
pub mod plan_sanitizer;
pub mod planner;
pub mod policy_gate;
pub mod prompt_builder;
pub mod security_prompts;
pub mod self_healing;
pub mod telemetry;
pub mod tool_llm;
pub mod ui;
pub mod wait_strategies;
pub mod workflow_manager;

// Removed old autonomous_planner exports
pub use desktop_session::DesktopSession;
pub use desktop_tools::{DesktopErrorCode, DesktopToolError, DesktopToolResult};
pub use dom_analyzer::DomAnalyzer;
pub use dom_context::{
    create_debug_context, create_focused_context, create_minimal_context, DOMContextConfig,
    DOMContextFormatter, FormattedDOMContext,
};
pub use dom_processor::{DomContext, DomElement, DomProcessor, DomProcessorConfig};
pub use element_ref::{
    ElementBounds, EncodedId, InputRung, ResolvedElement, ResultEnvelope, TargetSpec,
};
pub use failure_recovery::{DomChangeDetector, ErrorCategory, FailureTracker, RecoveryStrategy};
pub use llm::{LLMClient, LLMResponse};
pub use mode_selector::{EscalationReason, ExecutionMode, ModeSelector, ModeStatistics};
pub use orchestrator::Orchestrator;
pub use plan_sanitizer::PlanSanitizer;
pub use policy_gate::{PolicyDecision, PolicyDecisionKind, PolicyGate, PolicyRequest};
pub use prompt_builder::PromptBuilder;
pub use self_healing::SelfHealer;
pub use workflow_manager::{
    WorkflowCache, WorkflowExecutionContext, WorkflowExecutionStats, WorkflowExecutionSummary,
    WorkflowManager,
};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Executes the iterative planning-execution loop for a user goal.
///
/// This is the main entry point for planning and executing workflows.
pub async fn plan_execute_loop(
    goal: &str,
    start_url: Option<&str>,
    config: PlanConfig,
) -> PlanResult<serde_json::Value> {
    let mut orchestrator = Orchestrator::new(config).await?;

    let request = PlanRequest {
        goal: goal.to_string(),
        start_url: start_url.map(|s| s.to_string()),
        parameters: std::collections::HashMap::new(),
        save_workflow: false,
        workflow_name: None,
    };

    let response = orchestrator.plan(request).await?;

    if response.success {
        Ok(response.data.unwrap_or(serde_json::Value::Null))
    } else {
        Err(PlanError::ExecutionError(
            response.error.unwrap_or("Unknown error".to_string()),
        ))
    }
}

/// Builds the LLM prompt messages from the user goal, DOM outline, and history.
pub fn build_prompt(
    goal: &str,
    dom_outline: &str,
    history: &[HistoryEntry],
) -> Vec<serde_json::Value> {
    let prompt_builder = PromptBuilder::new();
    // Convert HistoryEntry to StepExecution for compatibility
    let step_executions: Vec<StepExecution> = history
        .iter()
        .map(|_| {
            // TODO: Implement proper conversion from HistoryEntry to StepExecution
            StepExecution {
                step: rzn_core::Step {
                    id: "placeholder".to_string(),
                    name: "Placeholder step".to_string(),
                    kind: rzn_core::StepKind::WaitForTimeout { timeout_ms: 1000 },
                },
                result: ExecutionResult::Success { payload: None },
                timestamp: chrono::Utc::now(),
                dom_snapshot: None,
            }
        })
        .collect();

    prompt_builder.build_planning_prompt(goal, dom_outline, "about:blank", &step_executions)
}

/// Reduces a raw HTML snapshot into a trimmed outline (ids, classes, text snippets).
pub fn html_reduce(html: &str) -> String {
    let analyzer = DomAnalyzer::new(30_000); // 30KB limit
    analyzer
        .reduce_html(html)
        .unwrap_or_else(|_| html.to_string())
}

/// Entry recording a single step execution result for history tracking.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct HistoryEntry {
    // TODO: define fields for history entries (step args, result/error).
}

/// Errors returned by the planning-execution loop.
#[derive(Debug, Error)]
pub enum PlanError {
    /// Error from the LLM client.
    #[error("LLM API error: {0}")]
    LLMError(String),

    #[error("Workflow execution failed: {0}")]
    ExecutionError(String),

    #[error("DOM parsing error: {0}")]
    DomError(String),

    #[error("Workflow not found: {0}")]
    WorkflowNotFound(String),

    #[error("Self-healing failed after {attempts} attempts")]
    HealingFailed { attempts: u32 },

    #[error("Invalid step configuration: {0}")]
    InvalidStep(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Policy blocked action: {0}")]
    PolicyBlocked(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("Broker communication error: {0}")]
    BrokerError(String),
}

/// Configuration for the planning system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanConfig {
    /// LLM provider to use (openai or gemini)
    pub llm_provider: String,

    /// OpenAI API key (deprecated, use llm_api_key)
    pub openai_api_key: String,

    /// LLM API key (for the selected provider)
    pub llm_api_key: String,

    /// Model to use for planning (e.g., "o4-mini" or "gemini-2.0-flash")
    pub model: String,

    /// Model to use for execution (optional, defaults to planning model)
    pub execution_model: Option<String>,

    /// Maximum number of steps in a single planning session
    pub max_steps: u32,

    /// Maximum number of healing attempts
    pub max_healing_attempts: u32,

    /// Temperature for LLM responses (0.0 = deterministic)
    pub temperature: f32,

    /// Directory to store workflows
    pub workflows_dir: String,

    /// Maximum DOM size to send to LLM (in characters)
    pub max_dom_size: usize,

    /// Timeout for LLM requests (in seconds)
    pub llm_timeout: u64,

    /// Broker transport (tcp or pipe)
    pub broker_transport: String,
}

impl Default for PlanConfig {
    fn default() -> Self {
        // Read LLM_PROVIDER from environment, no auto-detection
        let llm_provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "openai".to_string()); // Default to OpenAI if not set

        // Get the appropriate API key, models, and temperature based on the provider
        use crate::llm_provider::ProviderType;
        let provider_type = ProviderType::from_str(&llm_provider).unwrap_or(ProviderType::OpenAI);

        let (api_key, model, execution_model, temperature) = match provider_type {
            ProviderType::Gemini => {
                let key = std::env::var("GEMINI_API_KEY").unwrap_or_default();
                let model = std::env::var("GEMINI_MODEL_PLANNING")
                    .unwrap_or_else(|_| "gemini-2.0-flash".to_string());
                let exec_model = std::env::var("GEMINI_MODEL_EXECUTION").ok();
                let temp = std::env::var("GEMINI_TEMPERATURE")
                    .ok()
                    .and_then(|t| t.parse::<f32>().ok())
                    .unwrap_or(1.0);
                (key, model, exec_model, temp)
            }
            ProviderType::Anthropic => {
                let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
                let model = std::env::var("ANTHROPIC_MODEL_PLANNING")
                    .unwrap_or_else(|_| "claude-3-5-sonnet-latest".to_string());
                let exec_model = std::env::var("ANTHROPIC_MODEL_EXECUTION").ok();
                let temp = std::env::var("ANTHROPIC_TEMPERATURE")
                    .ok()
                    .and_then(|t| t.parse::<f32>().ok())
                    .unwrap_or(1.0);
                (key, model, exec_model, temp)
            }
            ProviderType::Groq => {
                let key = std::env::var("GROQ_API_KEY").unwrap_or_default();
                let model = std::env::var("GROQ_MODEL_PLANNING")
                    .unwrap_or_else(|_| "llama-3.1-70b-versatile".to_string());
                let exec_model = std::env::var("GROQ_MODEL_EXECUTION").ok();
                let temp = std::env::var("GROQ_TEMPERATURE")
                    .ok()
                    .and_then(|t| t.parse::<f32>().ok())
                    .unwrap_or(1.0);
                (key, model, exec_model, temp)
            }
            ProviderType::OpenAI => {
                let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
                let model = std::env::var("OPENAI_MODEL_PLANNING")
                    .unwrap_or_else(|_| "gpt-4o-mini".to_string());
                let exec_model = std::env::var("OPENAI_MODEL_EXECUTION").ok();
                let temp = std::env::var("OPENAI_TEMPERATURE")
                    .ok()
                    .and_then(|t| t.parse::<f32>().ok())
                    .unwrap_or(1.0);
                (key, model, exec_model, temp)
            }
            ProviderType::GeminiCli => {
                // External Gemini CLI (Sierra). No API key required; let CLI default model when "auto".
                let model =
                    std::env::var("GEMINI_CLI_MODEL").unwrap_or_else(|_| "auto".to_string());
                (String::new(), model, None, 0.0)
            }
            ProviderType::ClaudeCli => {
                let model =
                    std::env::var("CLAUDE_CLI_MODEL").unwrap_or_else(|_| "auto".to_string());
                (String::new(), model, None, 0.0)
            }
            ProviderType::CodexCli => {
                let model = std::env::var("CODEX_CLI_MODEL").unwrap_or_else(|_| "auto".to_string());
                (String::new(), model, None, 0.0)
            }
            ProviderType::Dummy => {
                let model = std::env::var("DUMMY_MODEL").unwrap_or_else(|_| "dummy".to_string());
                (String::new(), model, None, 0.0)
            }
        };

        // HTTP timeout override via env (seconds)
        let http_timeout = std::env::var("RZN_LLM_HTTP_TIMEOUT")
            .ok()
            .or_else(|| std::env::var("LLM_HTTP_TIMEOUT").ok())
            .or_else(|| std::env::var("OPENAI_HTTP_TIMEOUT").ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(30);

        Self {
            llm_provider,
            openai_api_key: std::env::var("OPENAI_API_KEY").unwrap_or_default(), // Keep for backward compatibility
            llm_api_key: api_key,
            model,
            execution_model,
            max_steps: 25,
            max_healing_attempts: 3,
            temperature,
            workflows_dir: std::env::var("RZN_WORKFLOWS_DIR")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .or_else(|| {
                    dirs::data_local_dir().map(|p| {
                        p.join("RZN")
                            .join("workflows")
                            .join("user")
                            .to_string_lossy()
                            .to_string()
                    })
                })
                .unwrap_or_else(|| "./workflows".to_string()),
            max_dom_size: 30_000, // 30KB
            llm_timeout: http_timeout,
            broker_transport: "native".to_string(),
        }
    }
}

/// Request to plan a new workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRequest {
    /// Natural language description of the goal
    pub goal: String,

    /// Optional starting URL
    pub start_url: Option<String>,

    /// Optional parameters for the workflow
    pub parameters: HashMap<String, String>,

    /// Whether to save the workflow for future use
    pub save_workflow: bool,

    /// Optional workflow name (auto-generated if not provided)
    pub workflow_name: Option<String>,
}

/// Response from planning operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanResponse {
    /// Whether planning was successful
    pub success: bool,

    /// Generated workflow (if successful)
    pub workflow: Option<rzn_core::Workflow>,

    /// Extracted data (if any)
    pub data: Option<serde_json::Value>,

    /// Error message (if failed)
    pub error: Option<String>,

    /// Number of steps executed
    pub steps_executed: u32,

    /// Path to saved workflow (if saved)
    pub workflow_path: Option<String>,
}

/// Request to run an existing workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRequest {
    /// Path to workflow file or workflow name
    pub workflow: String,

    /// Parameters for the workflow
    pub parameters: HashMap<String, String>,

    /// Whether to attempt self-healing if the workflow fails
    pub auto_heal: bool,
}

/// Response from running a workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResponse {
    /// Whether execution was successful
    pub success: bool,

    /// Extracted data (if any)
    pub data: Option<serde_json::Value>,

    /// Error message (if failed)
    pub error: Option<String>,

    /// Number of steps executed successfully
    pub steps_executed: u32,

    /// Whether self-healing was attempted
    pub healing_attempted: bool,

    /// Whether self-healing was successful
    pub healing_successful: bool,
}

/// Planning session state
#[derive(Debug, Clone)]
pub struct PlanningSession {
    pub goal: String,
    pub steps: Vec<rzn_core::Step>,
    pub history: Vec<StepExecution>,
    pub current_dom: String,
    pub current_url: String,
    pub parameters: HashMap<String, String>,
    /// Tracks consecutive failures and recovery strategies
    pub failure_tracker: FailureTracker,
    /// Detects DOM changes and frame swaps
    pub dom_change_detector: DomChangeDetector,
}

/// Record of a step execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepExecution {
    pub step: rzn_core::Step,
    pub result: ExecutionResult,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub dom_snapshot: Option<String>,
}

/// Result of executing a step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutionResult {
    Success {
        payload: Option<serde_json::Value>,
    },
    Error {
        message: String,
        retry_suggested: bool,
    },
}

pub type PlanResult<T> = std::result::Result<T, PlanError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_config_default() {
        let config = PlanConfig::default();
        // Model depends on environment variables, just check it's not empty
        assert!(!config.model.is_empty());
        assert_eq!(config.max_steps, 25);
        assert_eq!(config.temperature, 1.0);
    }

    #[test]
    fn test_html_reduce() {
        let html = "<html><body><h1>Test</h1><p>Content</p></body></html>";
        let reduced = html_reduce(html);
        assert!(!reduced.is_empty());
    }

    #[test]
    fn test_build_prompt() {
        let goal = "Test goal";
        let dom = "Test DOM";
        let history = vec![];
        let prompt = build_prompt(goal, dom, &history);
        assert!(!prompt.is_empty());
    }
}
