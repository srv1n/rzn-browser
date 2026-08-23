use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const BROWSER_CONTRACT: &str = "rzn.contracts";
pub const CLOUD_CONTRACT: &str = "rzn.cloud";

/// Runtime capabilities advertised by the substrate (extension/broker).
///
/// Each field is optional at the capability level because a runtime can expose
/// only the features that it supports.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Capabilities {
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
pub struct BrowserSnapshot {
    pub version: String,
    pub dom_hash: String,
    pub metadata: SnapshotMetadata,
    pub elements: Vec<BrowserElement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Capabilities>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotMetadata {
    pub timestamp: u64,
    pub url: String,
    pub title: String,
    pub viewport: Viewport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserElement {
    /// Stable element identifier within a snapshot (e.g. `elem_0`).
    pub encoded_id: String,
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub attributes: HashMap<String, String>,
    pub selector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial_info: Option<SpatialInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpatialInfo {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub area: i32,
    pub viewport_position: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserTarget {
    /// Prefer targeting by `encoded_id` derived from the latest `BrowserSnapshot`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoded_id: Option<String>,
    /// Optional direct selector escape hatch. Host apps may omit this entirely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    /// Optional frame identifier for actions inside nested frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
}

impl BrowserTarget {
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
pub enum DebugMode {
    Enrichment,
    Rescue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserAction {
    NavigateToUrl {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wait: Option<String>,
    },
    ClickElement {
        target: BrowserTarget,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        random_offset: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u32>,
    },
    FillInputField {
        target: BrowserTarget,
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
        target: BrowserTarget,
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u32>,
    },
    WaitForElement {
        target: BrowserTarget,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u32>,
    },
    GetElementText {
        target: BrowserTarget,
    },
    GetPageSource,
    EnableDebug {
        mode: DebugMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ttl_ms: Option<u32>,
    },
    DisableDebug,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserActionResult {
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
    pub current_tab_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dom_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dom_snapshot: Option<BrowserSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Capabilities>,
    /// Raw broker or extension payload for diagnostics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptEntry {
    pub id: String,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub action: BrowserAction,
    pub result: BrowserActionResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct BrowserTranscript {
    pub version: String,
    pub entries: Vec<TranscriptEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloudCommandKind {
    BrowserCommand,
    RunControl,
    PolicyResolution,
    HealthProbe,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloudBrowserCommand {
    pub cmd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloudCommandPayload {
    pub kind: CloudCommandKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<CloudBrowserCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_effecting: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloudCommandEnvelope {
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
    pub payload: CloudCommandPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloudCommandAck {
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
pub struct CloudCommandResult {
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
    pub result: Option<BrowserActionResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActorHello {
    pub version: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub actor_id: String,
    pub workspace_id: String,
    pub extension_version: String,
    pub capabilities: Capabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActorReady {
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
