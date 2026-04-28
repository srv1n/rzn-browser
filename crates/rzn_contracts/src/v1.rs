use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const CONTRACT_VERSION: &str = "rzn.contracts.v1";
pub const CLOUD_CONTRACT_VERSION: &str = "rzn.cloud.v1";

/// Runtime capabilities advertised by the substrate (extension/broker).
///
/// These are best-effort and may be omitted by older runtimes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CapabilitiesV1 {
    /// Deterministic DOM actor (extension/content-script) is available.
    #[serde(default)]
    pub extension_actor: bool,

    /// CDP is supported in principle (e.g. chrome.debugger API exists + extension has permission).
    #[serde(default)]
    pub cdp_available: bool,

    /// CDP is currently enabled (either via per-domain flags or a break-glass lease).
    #[serde(default)]
    pub cdp_enabled: bool,

    /// CDP is currently attached to the active workflow tab.
    #[serde(default)]
    pub cdp_attached: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotV1 {
    pub version: String,
    pub dom_hash: String,
    pub metadata: SnapshotMetadataV1,
    pub elements: Vec<ElementV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<CapabilitiesV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotMetadataV1 {
    pub timestamp: u64,
    pub url: String,
    pub title: String,
    pub viewport: ViewportV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ViewportV1 {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ElementV1 {
    /// Stable element identifier within a snapshot (e.g. `elem_0`).
    pub encoded_id: String,
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub attributes: HashMap<String, String>,
    pub selector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial_info: Option<SpatialInfoV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpatialInfoV1 {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub area: i32,
    pub viewport_position: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetV1 {
    /// Prefer targeting by `encoded_id` derived from the latest `SnapshotV1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoded_id: Option<String>,
    /// Optional direct selector escape hatch. Host apps may omit this entirely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    /// Optional frame hint (stringified ordinal) for legacy handlers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
}

impl TargetV1 {
    pub fn from_encoded_id(encoded_id: impl Into<String>) -> Self {
        Self {
            encoded_id: Some(encoded_id.into()),
            selector: None,
            frame_id: None,
        }
    }

    pub fn from_selector(selector: impl Into<String>) -> Self {
        Self {
            encoded_id: None,
            selector: Some(selector.into()),
            frame_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DebugModeV1 {
    Enrichment,
    Rescue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionV1 {
    NavigateToUrl {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wait: Option<String>,
    },
    ClickElement {
        target: TargetV1,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        random_offset: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u32>,
    },
    FillInputField {
        target: TargetV1,
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clear_first: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        simulate_typing: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delay_ms: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u32>,
    },
    PressSpecialKey {
        target: TargetV1,
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u32>,
    },
    WaitForElement {
        target: TargetV1,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u32>,
    },
    GetElementText {
        target: TargetV1,
    },
    GetPageSource,
    EnableDebug {
        mode: DebugModeV1,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ttl_ms: Option<u32>,
    },
    DisableDebug,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionResultV1 {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tab_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dom_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dom_snapshot: Option<SnapshotV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<CapabilitiesV1>,
    /// Raw broker/extension payload (opaque; useful for debugging and forward compatibility).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptEntryV1 {
    pub id: String,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub action: ActionV1,
    pub result: ActionResultV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TranscriptV1 {
    pub version: String,
    pub entries: Vec<TranscriptEntryV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloudCommandKindV1 {
    BrowserCommand,
    RunControl,
    PolicyResolution,
    HealthProbe,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloudBrowserCommandV1 {
    pub cmd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloudCommandPayloadV1 {
    pub kind: CloudCommandKindV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<CloudBrowserCommandV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_effecting: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloudCommandEnvelopeV1 {
    pub version: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub actor_id: String,
    pub run_id: String,
    pub session_id: String,
    pub command_id: String,
    pub lease_id: String,
    pub deadline_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_command_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner_step_index: Option<u32>,
    pub payload: CloudCommandPayloadV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloudCommandAckV1 {
    pub version: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub actor_id: String,
    pub run_id: String,
    pub session_id: String,
    pub command_id: String,
    pub lease_id: String,
    pub accepted_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloudCommandResultV1 {
    pub version: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub actor_id: String,
    pub run_id: String,
    pub session_id: String,
    pub command_id: String,
    pub lease_id: String,
    pub success: bool,
    pub finished_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ActionResultV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActorHelloV1 {
    pub version: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub actor_id: String,
    pub workspace_id: String,
    pub extension_version: String,
    pub capabilities: CapabilitiesV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActorReadyV1 {
    pub version: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub actor_id: String,
    pub heartbeat_interval_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}
