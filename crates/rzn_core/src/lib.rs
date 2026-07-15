pub mod ads_smoke;
pub mod errors;
pub mod executor;
pub mod framing;
pub mod runtime_paths;
pub mod secure_files;
pub mod workflow_contract {
    pub use rzn_contracts::v2::*;
    use serde_json::Value;

    pub fn validate_manifest_str(
        json_str: &str,
    ) -> Result<WorkflowManifestV2, Vec<ContractValidationIssueV2>> {
        let value = serde_json::from_str::<Value>(json_str).map_err(|err| {
            vec![ContractValidationIssueV2::new(
                "",
                format!("invalid JSON: {err}"),
            )]
        })?;
        validate_manifest_value(&value)
    }

    pub fn validate_run_envelope_str(
        manifest: &WorkflowManifestV2,
        json_str: &str,
    ) -> Result<RunEnvelopeV1, Vec<ContractValidationIssueV2>> {
        let value = serde_json::from_str::<Value>(json_str).map_err(|err| {
            vec![ContractValidationIssueV2::new(
                "",
                format!("invalid JSON: {err}"),
            )]
        })?;
        validate_run_envelope_value(manifest, &value)
    }

    pub fn normalize_manifest_params(
        manifest: &WorkflowManifestV2,
        input: &Value,
    ) -> Result<serde_json::Map<String, Value>, Vec<ParamValidationIssueV2>> {
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
    use std::collections::HashMap;

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

    /// Request sent from the CLI to the broker describing a workflow to run
    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct WorkflowRequest {
        pub action: String,
        pub task_id: String,
        pub workflow: Workflow,
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

    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct SelectorConfig {
        pub name: String,
        pub selector: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub attribute: Option<String>,
        #[serde(default)]
        pub post_processing: Vec<String>,
    }

    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct ExtensionResponse {
        pub action: String,
        pub task_id: String,
        pub success: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub result: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub error: Option<String>,
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

    #[derive(Serialize, Deserialize, Debug, Clone)]
    #[serde(tag = "type")]
    pub enum StepResult {
        #[serde(rename = "ok")]
        Ok { step_id: String },

        #[serde(rename = "ok_with_payload")]
        OkWithPayload {
            step_id: String,
            payload: serde_json::Value,
        },

        #[serde(rename = "error")]
        Error { step_id: String, message: String },
    }

    /// Template for parameterized strings
    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct Template(pub String);

    impl Template {
        /// Render the template with the given parameters
        pub fn render(&self, params: &HashMap<String, String>) -> String {
            let mut result = self.0.clone();
            for (key, value) in params {
                result = result.replace(&format!("{{{}}}", key), value);
            }
            result
        }
    }

    /// Field definition for forms and validation
    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct FieldDef {
        pub name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub description: Option<String>,
        #[serde(default)]
        pub required: bool,
    }

    impl Workflow {
        /// Convert the workflow into a Task using the first sequence of
        /// browser automation steps.
        pub fn to_task(&self) -> Result<Task, String> {
            let steps = if let Some(seq) = self.browser_automation.sequences.first() {
                seq.steps.clone()
            } else {
                Vec::new()
            };

            Ok(Task {
                steps,
                search_query: None,
            })
        }
    }

    /// Validation functions for workflows
    pub fn validate_workflow_value(value: &serde_json::Value) -> Result<(), String> {
        validate_against_schema(value, include_str!("../../../schema/actions-v1.json"))
    }

    pub fn validate_action_value(value: &serde_json::Value) -> Result<(), String> {
        validate_against_schema(value, include_str!("../../../schema/actions-v1.json"))
    }

    pub fn validate_workflow_str(json_str: &str) -> Result<(), String> {
        let value: serde_json::Value =
            serde_json::from_str(json_str).map_err(|e| format!("Invalid JSON: {}", e))?;
        validate_workflow_value(&value)
    }

    pub fn validate_action_str(json_str: &str) -> Result<(), String> {
        let value: serde_json::Value =
            serde_json::from_str(json_str).map_err(|e| format!("Invalid JSON: {}", e))?;
        validate_action_value(&value)
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
