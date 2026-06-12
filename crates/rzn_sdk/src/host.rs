use crate::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeTransport {
    Native,
    Tcp,
    Pipe,
}

impl RuntimeTransport {
    pub fn from_env() -> Self {
        match std::env::var("RZN_TRANSPORT")
            .unwrap_or_else(|_| "native".to_string())
            .to_lowercase()
            .as_str()
        {
            "native" | "endpoint" | "auto" => Self::Native,
            "tcp" => Self::Tcp,
            _ => Self::Pipe,
        }
    }
}

impl From<RuntimeTransport> for rzn_plan::broker_client::Transport {
    fn from(value: RuntimeTransport) -> Self {
        match value {
            RuntimeTransport::Native => Self::Native,
            RuntimeTransport::Tcp => Self::Tcp,
            RuntimeTransport::Pipe => Self::Pipe,
        }
    }
}

pub type BrokerTransport = RuntimeTransport;

/// Host-side SDK configuration.
///
/// This is a stable, serializable wrapper over the engine's config. It is
/// intentionally shaped to work well as a Tauri command payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostConfig {
    pub llm_provider: String,
    pub openai_api_key: String,
    pub llm_api_key: String,
    pub model: String,
    pub execution_model: Option<String>,
    pub max_steps: u32,
    pub max_healing_attempts: u32,
    pub temperature: f32,
    pub workflows_dir: String,
    pub max_dom_size: usize,
    pub llm_timeout: u64,
    #[serde(default = "default_runtime_transport", alias = "broker_transport")]
    pub runtime_transport: String,
}

fn default_runtime_transport() -> String {
    "native".to_string()
}

impl HostConfig {
    pub fn from_env() -> Self {
        Self::default()
    }

    fn to_plan_config(&self) -> rzn_plan::PlanConfig {
        rzn_plan::PlanConfig {
            llm_provider: self.llm_provider.clone(),
            openai_api_key: self.openai_api_key.clone(),
            llm_api_key: self.llm_api_key.clone(),
            model: self.model.clone(),
            execution_model: self.execution_model.clone(),
            max_steps: self.max_steps,
            max_healing_attempts: self.max_healing_attempts,
            temperature: self.temperature,
            workflows_dir: self.workflows_dir.clone(),
            max_dom_size: self.max_dom_size,
            llm_timeout: self.llm_timeout,
            broker_transport: self.runtime_transport.clone(),
        }
    }
}

impl Default for HostConfig {
    fn default() -> Self {
        let cfg = rzn_plan::PlanConfig::default();
        Self {
            llm_provider: cfg.llm_provider,
            openai_api_key: cfg.openai_api_key,
            llm_api_key: cfg.llm_api_key,
            model: cfg.model,
            execution_model: cfg.execution_model,
            max_steps: cfg.max_steps,
            max_healing_attempts: cfg.max_healing_attempts,
            temperature: cfg.temperature,
            workflows_dir: cfg.workflows_dir,
            max_dom_size: cfg.max_dom_size,
            llm_timeout: cfg.llm_timeout,
            runtime_transport: cfg.broker_transport,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRequest {
    pub goal: String,
    pub start_url: Option<String>,
    #[serde(default)]
    pub parameters: HashMap<String, String>,
    #[serde(default)]
    pub save_workflow: bool,
    pub workflow_name: Option<String>,
}

impl From<PlanRequest> for rzn_plan::PlanRequest {
    fn from(value: PlanRequest) -> Self {
        Self {
            goal: value.goal,
            start_url: value.start_url,
            parameters: value.parameters,
            save_workflow: value.save_workflow,
            workflow_name: value.workflow_name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanResponse {
    pub success: bool,
    pub workflow: Option<rzn_core::Workflow>,
    pub data: Option<Value>,
    pub error: Option<String>,
    pub steps_executed: u32,
    pub workflow_path: Option<String>,
}

impl From<rzn_plan::PlanResponse> for PlanResponse {
    fn from(value: rzn_plan::PlanResponse) -> Self {
        Self {
            success: value.success,
            workflow: value.workflow,
            data: value.data,
            error: value.error,
            steps_executed: value.steps_executed,
            workflow_path: value.workflow_path,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRequest {
    pub workflow: String,
    #[serde(default)]
    pub parameters: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub auto_heal: bool,
}

fn default_true() -> bool {
    true
}

impl From<RunRequest> for rzn_plan::RunRequest {
    fn from(value: RunRequest) -> Self {
        Self {
            workflow: value.workflow,
            parameters: value.parameters,
            auto_heal: value.auto_heal,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResponse {
    pub success: bool,
    pub data: Option<Value>,
    pub error: Option<String>,
    pub steps_executed: u32,
    pub healing_attempted: bool,
    pub healing_successful: bool,
}

impl From<rzn_plan::RunResponse> for RunResponse {
    fn from(value: rzn_plan::RunResponse) -> Self {
        Self {
            success: value.success,
            data: value.data,
            error: value.error,
            steps_executed: value.steps_executed,
            healing_attempted: value.healing_attempted,
            healing_successful: value.healing_successful,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostErrorCode {
    Llm,
    Execution,
    Dom,
    WorkflowNotFound,
    HealingFailed,
    InvalidStep,
    Validation,
    Io,
    Serialization,
    Http,
    #[serde(alias = "broker")]
    Runtime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostError {
    pub code: HostErrorCode,
    pub message: String,
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for HostError {}

pub(crate) fn map_plan_error(err: rzn_plan::PlanError) -> HostError {
    use rzn_plan::PlanError as E;

    // Keep a forward-compatible fallback arm so the SDK doesn't break if the
    // engine adds new `PlanError` variants.
    #[allow(unreachable_patterns)]
    match err {
        E::LLMError(message) => HostError {
            code: HostErrorCode::Llm,
            message,
        },
        E::ExecutionError(message) => HostError {
            code: HostErrorCode::Execution,
            message,
        },
        E::DomError(message) => HostError {
            code: HostErrorCode::Dom,
            message,
        },
        E::WorkflowNotFound(message) => HostError {
            code: HostErrorCode::WorkflowNotFound,
            message,
        },
        E::HealingFailed { attempts } => HostError {
            code: HostErrorCode::HealingFailed,
            message: format!("Self-healing failed after {attempts} attempts"),
        },
        E::InvalidStep(message) => HostError {
            code: HostErrorCode::InvalidStep,
            message,
        },
        E::Validation(message) => HostError {
            code: HostErrorCode::Validation,
            message,
        },
        E::PolicyBlocked(message) => HostError {
            code: HostErrorCode::Validation,
            message,
        },
        E::IoError(e) => HostError {
            code: HostErrorCode::Io,
            message: e.to_string(),
        },
        E::SerializationError(e) => HostError {
            code: HostErrorCode::Serialization,
            message: e.to_string(),
        },
        E::HttpError(e) => HostError {
            code: HostErrorCode::Http,
            message: e.to_string(),
        },
        E::BrokerError(message) => HostError {
            code: HostErrorCode::Runtime,
            message,
        },
        other => HostError {
            code: HostErrorCode::Execution,
            message: other.to_string(),
        },
    }
}

/// A small wrapper around `rzn_plan::Orchestrator` intended for embedding.
pub struct Host {
    orchestrator: rzn_plan::Orchestrator,
}

impl Host {
    pub async fn from_env() -> Result<Self> {
        Self::new(HostConfig::default()).await
    }

    pub async fn new(config: HostConfig) -> Result<Self> {
        let orchestrator = rzn_plan::Orchestrator::new(config.to_plan_config())
            .await
            .map_err(map_plan_error)?;
        Ok(Self { orchestrator })
    }

    pub async fn plan_llm_only(&mut self, request: PlanRequest) -> Result<PlanResponse> {
        let resp = self
            .orchestrator
            .plan_llm_only(request.into())
            .await
            .map_err(map_plan_error)?;
        Ok(resp.into())
    }

    pub async fn plan_auto(&mut self, request: PlanRequest) -> Result<PlanResponse> {
        let resp = self
            .orchestrator
            .plan_auto(request.into())
            .await
            .map_err(map_plan_error)?;
        Ok(resp.into())
    }

    pub async fn run(&mut self, request: RunRequest) -> Result<RunResponse> {
        let resp = self
            .orchestrator
            .run(request.into())
            .await
            .map_err(map_plan_error)?;
        Ok(resp.into())
    }

    #[cfg(feature = "unstable")]
    pub fn orchestrator_mut(&mut self) -> &mut rzn_plan::Orchestrator {
        &mut self.orchestrator
    }
}
