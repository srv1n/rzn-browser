use crate::host::RuntimeTransport;
use crate::Result;
use rzn_contracts::v1::{
    ActionResultV1, ActionV1, CapabilitiesV1, DebugModeV1, ElementV1, SnapshotMetadataV1,
    SnapshotV1, SpatialInfoV1, TargetV1, TranscriptEntryV1, TranscriptV1, ViewportV1,
    CONTRACT_VERSION,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("operation timed out after {0:?}")]
    Timeout(Duration),

    #[error("runtime error: {0}")]
    Runtime(String),

    #[error("missing dom_snapshot in response")]
    MissingDomSnapshot,

    #[error("target not found in last snapshot: {0}")]
    TargetNotFound(String),

    #[error("invalid target: must provide encoded_id or selector")]
    InvalidTarget,

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug, Clone, Deserialize)]
struct RawSpatialInfo {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    area: i32,
    viewport_position: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RawElementStub {
    tag: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    attributes: HashMap<String, String>,
    selector: String,
    #[serde(default)]
    spatial_info: Option<RawSpatialInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawViewport {
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct RawDomMetadata {
    timestamp: u64,
    url: String,
    #[serde(default)]
    title: String,
    viewport: RawViewport,
}

#[derive(Debug, Clone, Deserialize)]
struct RawDomSnapshot {
    elements: Vec<RawElementStub>,
    hash: String,
    #[serde(default)]
    prompt: String,
    metadata: RawDomMetadata,
}

/// Embedding-friendly runtime session.
///
/// This provides deterministic actor-grade primitives: snapshot + apply action.
/// LLM prompting/planning loops are intentionally out-of-scope.
pub struct Session {
    client: rzn_plan::broker_client::BrokerClient,
    next_id: u64,
    last_snapshot: Option<SnapshotV1>,
    selector_by_encoded_id: HashMap<String, String>,
    transcript: TranscriptV1,
    config: SessionConfig,
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub snapshot_timeout: Duration,
    pub action_timeout: Duration,
    pub snapshot_max_elements: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            snapshot_timeout: Duration::from_secs(10),
            action_timeout: Duration::from_secs(30),
            snapshot_max_elements: 120,
        }
    }
}

impl Session {
    pub async fn connect(transport: RuntimeTransport) -> Result<Self> {
        Self::connect_with_config(transport, SessionConfig::default()).await
    }

    pub async fn connect_with_config(
        transport: RuntimeTransport,
        config: SessionConfig,
    ) -> Result<Self> {
        let mut client = rzn_plan::broker_client::BrokerClient::new(transport.into());
        client
            .connect()
            .await
            .map_err(crate::host::map_plan_error)?;

        Ok(Self {
            client,
            next_id: 1,
            last_snapshot: None,
            selector_by_encoded_id: HashMap::new(),
            transcript: TranscriptV1 {
                version: CONTRACT_VERSION.to_string(),
                entries: Vec::new(),
            },
            config,
        })
    }

    pub async fn close(&mut self) -> Result<()> {
        self.client
            .disconnect()
            .await
            .map_err(crate::host::map_plan_error)?;
        Ok(())
    }

    pub fn transcript(&self) -> &TranscriptV1 {
        &self.transcript
    }

    pub fn last_snapshot(&self) -> Option<&SnapshotV1> {
        self.last_snapshot.as_ref()
    }

    pub async fn snapshot(&mut self) -> Result<SnapshotV1> {
        let resp =
            tokio::time::timeout(self.config.snapshot_timeout, self.client.get_dom_snapshot())
                .await
                .map_err(|_| Error::Timeout(self.config.snapshot_timeout))?
                .map_err(crate::host::map_plan_error)?;

        let capabilities: Option<CapabilitiesV1> = resp
            .get("capabilities")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok());

        // Prefer top-level dom_snapshot; tolerate nested responses.
        let dom_snapshot_value = resp
            .get("dom_snapshot")
            .cloned()
            .or_else(|| resp.pointer("/result/dom_snapshot").cloned())
            .ok_or(Error::MissingDomSnapshot)?;

        let raw: RawDomSnapshot = serde_json::from_value(dom_snapshot_value)?;

        let elements: Vec<ElementV1> = raw
            .elements
            .into_iter()
            .enumerate()
            .map(|(idx, el)| {
                let encoded_id = format!("elem_{}", idx);
                ElementV1 {
                    encoded_id,
                    tag: el.tag,
                    text: el.text,
                    attributes: el.attributes,
                    selector: el.selector,
                    spatial_info: el.spatial_info.map(|s| SpatialInfoV1 {
                        x: s.x,
                        y: s.y,
                        width: s.width,
                        height: s.height,
                        area: s.area,
                        viewport_position: s.viewport_position,
                    }),
                }
            })
            .collect();

        let snapshot = SnapshotV1 {
            version: CONTRACT_VERSION.to_string(),
            dom_hash: raw.hash,
            metadata: SnapshotMetadataV1 {
                timestamp: raw.metadata.timestamp,
                url: raw.metadata.url,
                title: raw.metadata.title,
                viewport: ViewportV1 {
                    width: raw.metadata.viewport.width,
                    height: raw.metadata.viewport.height,
                },
            },
            elements,
            prompt: Some(raw.prompt),
            capabilities,
        };

        self.selector_by_encoded_id = snapshot
            .elements
            .iter()
            .map(|e| (e.encoded_id.clone(), e.selector.clone()))
            .collect();
        self.last_snapshot = Some(snapshot.clone());

        Ok(snapshot)
    }

    pub async fn apply(&mut self, action: ActionV1) -> Result<ActionResultV1> {
        let started_at_ms = now_ms();
        let id = self.alloc_id("act");

        let raw_response = match &action {
            ActionV1::EnableDebug { mode, ttl_ms } => {
                let mode_str = match mode {
                    DebugModeV1::Enrichment => "enrichment",
                    DebugModeV1::Rescue => "rescue",
                };
                tokio::time::timeout(
                    self.config.action_timeout,
                    self.client.enable_debug(mode_str, ttl_ms.map(|v| v as u32)),
                )
                .await
                .map_err(|_| Error::Timeout(self.config.action_timeout))?
                .map_err(crate::host::map_plan_error)?
            }
            ActionV1::DisableDebug => {
                tokio::time::timeout(self.config.action_timeout, self.client.disable_debug())
                    .await
                    .map_err(|_| Error::Timeout(self.config.action_timeout))?
                    .map_err(crate::host::map_plan_error)?
            }
            _ => {
                let step = self.action_to_step(&id, &action)?;
                let task_id = id.clone();
                let message = rzn_core::dsl::Message {
                    action: "perform_task".to_string(),
                    task_id: task_id.clone(),
                    task: Some(rzn_core::dsl::Task {
                        steps: vec![step],
                        search_query: None,
                    }),
                    data: None,
                };

                tokio::time::timeout(
                    self.config.action_timeout,
                    self.client.send_message(message),
                )
                .await
                .map_err(|_| Error::Timeout(self.config.action_timeout))?
                .map_err(crate::host::map_plan_error)?
            }
        };

        let result = Self::map_action_result(raw_response);
        let finished_at_ms = now_ms();

        self.transcript.entries.push(TranscriptEntryV1 {
            id,
            started_at_ms,
            finished_at_ms,
            action,
            result: result.clone(),
        });

        Ok(result)
    }

    fn alloc_id(&mut self, prefix: &str) -> String {
        let id = self.next_id;
        self.next_id += 1;
        format!("{prefix}-{id}")
    }

    fn resolve_target_selector(&self, target: &TargetV1) -> std::result::Result<String, Error> {
        if let Some(selector) = target.selector.as_ref() {
            if !selector.trim().is_empty() {
                return Ok(selector.clone());
            }
        }

        if let Some(encoded_id) = target.encoded_id.as_ref() {
            if let Some(selector) = self.selector_by_encoded_id.get(encoded_id) {
                return Ok(selector.clone());
            }
            return Err(Error::TargetNotFound(encoded_id.clone()));
        }

        Err(Error::InvalidTarget)
    }

    fn action_to_step(
        &self,
        id: &str,
        action: &ActionV1,
    ) -> std::result::Result<rzn_core::Step, Error> {
        use rzn_core::StepKind;

        let name = match action {
            ActionV1::NavigateToUrl { .. } => "Navigate",
            ActionV1::ClickElement { .. } => "Click",
            ActionV1::FillInputField { .. } => "Fill",
            ActionV1::PressSpecialKey { .. } => "Press key",
            ActionV1::WaitForElement { .. } => "Wait",
            ActionV1::GetElementText { .. } => "Get text",
            ActionV1::GetPageSource => "Get page source",
            ActionV1::EnableDebug { .. } => "Enable debug",
            ActionV1::DisableDebug => "Disable debug",
        }
        .to_string();

        let kind = match action {
            ActionV1::NavigateToUrl { url, wait } => StepKind::NavigateToUrl {
                url: url.clone(),
                wait: wait.clone(),
            },
            ActionV1::ClickElement {
                target,
                random_offset,
                timeout_ms,
            } => StepKind::ClickElement {
                selector: self.resolve_target_selector(target)?,
                frame_id: target.frame_id.clone(),
                random_offset: *random_offset,
                timeout_ms: *timeout_ms,
            },
            ActionV1::FillInputField {
                target,
                value,
                clear_first,
                simulate_typing,
                delay_ms,
                timeout_ms,
            } => StepKind::FillInputField {
                selector: self.resolve_target_selector(target)?,
                value: value.clone(),
                frame_id: target.frame_id.clone(),
                clear_first: *clear_first,
                simulate_typing: *simulate_typing,
                delay_ms: *delay_ms,
                timeout_ms: *timeout_ms,
            },
            ActionV1::PressSpecialKey {
                target,
                key,
                timeout_ms,
            } => StepKind::PressSpecialKey {
                key: key.clone(),
                selector: Some(self.resolve_target_selector(target)?),
                frame_id: target.frame_id.clone(),
                timeout_ms: *timeout_ms,
            },
            ActionV1::WaitForElement { target, timeout_ms } => StepKind::WaitForElement {
                selector: self.resolve_target_selector(target)?,
                frame_id: target.frame_id.clone(),
                timeout_ms: *timeout_ms,
                condition: Some("visible".to_string()),
            },
            ActionV1::GetElementText { target } => StepKind::GetElementText {
                selector: self.resolve_target_selector(target)?,
                frame_id: target.frame_id.clone(),
            },
            ActionV1::GetPageSource => StepKind::GetPageSource,
            ActionV1::EnableDebug { .. } | ActionV1::DisableDebug => {
                return Err(Error::Runtime(
                    "debug enable/disable is not a StepKind; call Session.apply(ActionV1::EnableDebug/DisableDebug)".to_string(),
                ))
            }
        };

        Ok(rzn_core::Step::new(id.to_string(), name, kind))
    }

    fn map_action_result(raw: Value) -> ActionResultV1 {
        let success = raw
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let error_code = raw
            .get("error_code")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let error = raw
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let current_url = raw
            .get("current_url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let current_tab_id = raw
            .get("current_tab_id")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32);
        let dom_hash = raw
            .get("dom_hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let capabilities = raw
            .get("capabilities")
            .cloned()
            .and_then(|v| serde_json::from_value::<CapabilitiesV1>(v).ok());

        ActionResultV1 {
            success,
            error_code,
            error,
            current_url,
            current_tab_id,
            dom_hash,
            dom_snapshot: None,
            capabilities,
            raw: Some(raw),
        }
    }
}
