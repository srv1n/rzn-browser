use crate::host::RuntimeTransport;
use crate::Result;
use rzn_contracts::v1::{
    ActionResultV1, ActionV1, CapabilitiesV1, DebugModeV1, ElementV1, SnapshotMetadataV1,
    SnapshotV1, SpatialInfoV1, TargetV1, TranscriptEntryV1, TranscriptV1, ViewportV1,
    CONTRACT_VERSION,
};
use serde::Deserialize;
use serde_json::{json, Value};
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

    #[error(
        "stale element id: {encoded_id} is from snapshot generation {id_generation}, current generation is {current_generation}"
    )]
    StaleElementId {
        encoded_id: String,
        id_generation: u64,
        current_generation: u64,
    },

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
    snapshot_generation: u64,
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
            snapshot_generation: 0,
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
        let request = Self::dom_snapshot_message(
            format!("snap-{}", self.snapshot_generation + 1),
            self.config.snapshot_max_elements,
        );
        let resp = tokio::time::timeout(
            self.config.snapshot_timeout,
            self.client.send_message(request),
        )
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
        let generation = self.snapshot_generation + 1;
        let snapshot = Self::snapshot_from_raw(
            raw,
            generation,
            self.config.snapshot_max_elements,
            capabilities,
        );

        self.selector_by_encoded_id = snapshot
            .elements
            .iter()
            .map(|e| (e.encoded_id.clone(), e.selector.clone()))
            .collect();
        self.snapshot_generation = generation;
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
                    self.client.enable_debug(mode_str, ttl_ms.map(|v| v)),
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

    fn dom_snapshot_message(request_id: String, max_elements: u32) -> rzn_core::dsl::Message {
        rzn_core::dsl::Message {
            action: "execute_static".to_string(),
            task_id: request_id,
            task: None,
            data: Some(json!({
                "cmd": "get_dom_snapshot",
                "payload": {
                    "options": {
                        "maxElements": max_elements,
                        "highlightElements": false,
                    }
                }
            })),
        }
    }

    fn snapshot_from_raw(
        raw: RawDomSnapshot,
        generation: u64,
        max_elements: u32,
        capabilities: Option<CapabilitiesV1>,
    ) -> SnapshotV1 {
        let elements: Vec<ElementV1> = raw
            .elements
            .into_iter()
            .take(max_elements as usize)
            .enumerate()
            .map(|(idx, el)| ElementV1 {
                encoded_id: element_encoded_id(generation, idx),
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
            })
            .collect();

        SnapshotV1 {
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
        }
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
            if let (Some(id_generation), Some(_)) = (
                element_id_generation(encoded_id),
                self.last_snapshot.as_ref(),
            ) {
                if id_generation != self.snapshot_generation {
                    return Err(Error::StaleElementId {
                        encoded_id: encoded_id.clone(),
                        id_generation,
                        current_generation: self.snapshot_generation,
                    });
                }
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
            current_tab_ref: raw
                .get("current_tab_ref")
                .or_else(|| raw.get("tab_ref"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            dom_hash,
            dom_snapshot: None,
            capabilities,
            raw: Some(raw),
        }
    }
}

fn element_encoded_id(generation: u64, index: usize) -> String {
    format!("snap_{generation}_elem_{index}")
}

fn element_id_generation(encoded_id: &str) -> Option<u64> {
    let rest = encoded_id.strip_prefix("snap_")?;
    let (generation, _) = rest.split_once("_elem_")?;
    generation.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_snapshot(element_count: usize) -> RawDomSnapshot {
        RawDomSnapshot {
            elements: (0..element_count)
                .map(|idx| RawElementStub {
                    tag: "button".to_string(),
                    text: Some(format!("button {idx}")),
                    attributes: HashMap::new(),
                    selector: format!("#button-{idx}"),
                    spatial_info: None,
                })
                .collect(),
            hash: "hash".to_string(),
            prompt: "prompt".to_string(),
            metadata: RawDomMetadata {
                timestamp: 123,
                url: "https://example.test".to_string(),
                title: "Example".to_string(),
                viewport: RawViewport {
                    width: 800,
                    height: 600,
                },
            },
        }
    }

    fn session_with_current_snapshot(
        generation: u64,
        encoded_id: String,
        selector: &str,
    ) -> Session {
        let element = ElementV1 {
            encoded_id: encoded_id.clone(),
            tag: "button".to_string(),
            text: None,
            attributes: HashMap::new(),
            selector: selector.to_string(),
            spatial_info: None,
        };

        Session {
            client: rzn_plan::broker_client::BrokerClient::new(
                rzn_plan::broker_client::Transport::Tcp,
            ),
            next_id: 1,
            snapshot_generation: generation,
            last_snapshot: Some(SnapshotV1 {
                version: CONTRACT_VERSION.to_string(),
                dom_hash: "hash".to_string(),
                metadata: SnapshotMetadataV1 {
                    timestamp: 123,
                    url: "https://example.test".to_string(),
                    title: "Example".to_string(),
                    viewport: ViewportV1 {
                        width: 800,
                        height: 600,
                    },
                },
                elements: vec![element],
                prompt: None,
                capabilities: None,
            }),
            selector_by_encoded_id: HashMap::from([(encoded_id, selector.to_string())]),
            transcript: TranscriptV1 {
                version: CONTRACT_VERSION.to_string(),
                entries: Vec::new(),
            },
            config: SessionConfig::default(),
        }
    }

    #[test]
    fn snapshot_message_uses_configured_max_elements() {
        let message = Session::dom_snapshot_message("snap-test".to_string(), 7);
        let data = message.data.as_ref().expect("snapshot message data");

        assert_eq!(message.action, "execute_static");
        assert_eq!(
            data.get("cmd").and_then(Value::as_str),
            Some("get_dom_snapshot")
        );
        assert_eq!(
            data.pointer("/payload/options/maxElements")
                .and_then(Value::as_u64),
            Some(7)
        );
        assert_eq!(
            data.pointer("/payload/options/highlightElements")
                .and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn snapshot_from_raw_honors_max_elements() {
        let snapshot = Session::snapshot_from_raw(raw_snapshot(3), 1, 2, None);

        assert_eq!(snapshot.elements.len(), 2);
        assert_eq!(snapshot.elements[0].encoded_id, "snap_1_elem_0");
        assert_eq!(snapshot.elements[1].encoded_id, "snap_1_elem_1");
    }

    #[test]
    fn stale_generation_stamped_id_errors_after_later_snapshot() {
        let session = session_with_current_snapshot(2, element_encoded_id(2, 0), "#current-button");
        let stale_id = element_encoded_id(1, 0);

        let err = session
            .resolve_target_selector(&TargetV1::from_encoded_id(stale_id.clone()))
            .expect_err("old snapshot element id must be rejected");

        match err {
            Error::StaleElementId {
                encoded_id,
                id_generation,
                current_generation,
            } => {
                assert_eq!(encoded_id, stale_id);
                assert_eq!(id_generation, 1);
                assert_eq!(current_generation, 2);
            }
            other => panic!("expected stale element id error, got {other:?}"),
        }
    }

    #[test]
    fn current_generation_stamped_id_resolves() {
        let current_id = element_encoded_id(2, 0);
        let session = session_with_current_snapshot(2, current_id.clone(), "#current-button");

        assert_eq!(
            session
                .resolve_target_selector(&TargetV1::from_encoded_id(current_id))
                .expect("current snapshot element id resolves"),
            "#current-button"
        );
    }
}
