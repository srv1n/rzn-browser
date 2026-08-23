pub mod ads_smoke;
pub mod errors;
pub mod executor;
pub mod framing;
pub mod runtime_paths;
pub mod secure_files;
pub mod workflow_contract {
    pub use rzn_contracts::workflow::*;
    use serde_json::Value;

    pub fn validate_manifest_str(
        json_str: &str,
    ) -> Result<WorkflowManifest, Vec<ContractValidationIssue>> {
        let value = serde_json::from_str::<Value>(json_str).map_err(|err| {
            vec![ContractValidationIssue::new(
                "",
                format!("invalid JSON: {err}"),
            )]
        })?;
        validate_manifest_value(&value)
    }

    pub fn validate_run_envelope_str(
        manifest: &WorkflowManifest,
        json_str: &str,
    ) -> Result<RunRequest, Vec<ContractValidationIssue>> {
        let value = serde_json::from_str::<Value>(json_str).map_err(|err| {
            vec![ContractValidationIssue::new(
                "",
                format!("invalid JSON: {err}"),
            )]
        })?;
        validate_run_envelope_value(manifest, &value)
    }

    pub fn normalize_manifest_params(
        manifest: &WorkflowManifest,
        input: &Value,
    ) -> Result<serde_json::Map<String, Value>, Vec<ParamValidationIssue>> {
        manifest.normalize_params(input)
    }
}

// Re-export commonly used error types
pub use errors::{
    DomError, ErrorContext, ExecutionError, NetworkError, PermissionError, RecoverySuggestion,
    RetryStrategy, RznError, RznResult, SystemError, ValidationError,
};

// Include the generated step definitions
include!(concat!(env!("OUT_DIR"), "/step.rs"));

pub mod dsl {
    use super::*;
    use jsonschema::{Draft, JSONSchema};
    use serde::{Deserialize, Serialize};

    /// Main Step structure that wraps the generated StepKind
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
    pub struct Step {
        pub id: String,
        pub name: String,
        #[serde(flatten)]
        pub kind: StepKind,
    }

    impl Step {
        pub fn new(id: String, name: String, kind: StepKind) -> Self {
            Self { id, name, kind }
        }
    }

    /// Top level workflow structure used by the CLI and broker
    #[derive(Serialize, Deserialize, Debug, Clone, Default)]
    pub struct Workflow {
        #[serde(default)]
        pub id: String,
        #[serde(default)]
        pub name: String,
        #[serde(default)]
        pub description: String,
        #[serde(default)]
        pub version: String,
        #[serde(default)]
        pub last_updated: String,
        pub browser_automation: BrowserAutomation,
    }

    #[derive(Serialize, Deserialize, Debug, Clone, Default)]
    pub struct BrowserAutomation {
        #[serde(default)]
        pub sequences: Vec<Sequence>,
    }

    #[derive(Serialize, Deserialize, Debug, Clone, Default)]
    pub struct Sequence {
        pub name: String,
        pub description: String,
        pub required_variables: Vec<Variable>,
        pub steps: Vec<Step>,
    }

    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct Variable {
        pub name: String,
        pub description: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub sensitive: Option<bool>,
    }

    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct Message {
        pub action: String,
        pub task_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub task: Option<Task>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub data: Option<serde_json::Value>,
    }

    #[derive(Serialize, Deserialize, Debug, Clone, Default)]
    pub struct Task {
        #[serde(default)]
        pub steps: Vec<Step>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub search_query: Option<String>,
    }

    /// Log message sent through native messaging
    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct LogMessage {
        pub timestamp: String,
        pub level: String,
        pub component: String,
        pub message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub data: Option<serde_json::Value>,
    }

    pub fn validate_action_value(value: &serde_json::Value) -> Result<(), String> {
        validate_against_schema(value, include_str!("../../../schema/actions.json"))
    }

    fn validate_against_schema(value: &serde_json::Value, schema_str: &str) -> Result<(), String> {
        let schema_value: serde_json::Value =
            serde_json::from_str(schema_str).map_err(|e| format!("Invalid schema JSON: {}", e))?;

        let schema = JSONSchema::options()
            .with_draft(Draft::Draft7)
            .compile(&schema_value)
            .map_err(|e| format!("Schema compilation error: {}", e))?;

        if let Err(errors) = schema.validate(value) {
            let error_messages: Vec<String> =
                errors.map(|e| format!("Validation error: {}", e)).collect();
            return Err(error_messages.join("; "));
        }

        Ok(())
    }
}

pub use dsl::*;
