use crate::host::RuntimeTransport;
use crate::session::{Session, SessionConfig};
use rzn_contracts::v1::{ActionResultV1, ActionV1, SnapshotV1, TargetV1, TranscriptV1};
use tokio::time::Duration;

#[derive(Debug, Clone)]
pub struct ObserveOptions {
    pub max_elements: u32,
    pub timeout: Duration,
}

impl Default for ObserveOptions {
    fn default() -> Self {
        Self {
            max_elements: 120,
            timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("timeout")]
    Timeout,

    #[error("target_not_found: {0}")]
    TargetNotFound(String),

    #[error("validation: {0}")]
    Validation(String),

    #[error("transport: {0}")]
    Transport(String),

    #[error("extension_error: {code:?}: {message}")]
    ExtensionError {
        code: Option<String>,
        message: String,
    },

    #[error("unexpected response shape")]
    UnexpectedResponse,
}

pub type ToolResult<T> = std::result::Result<T, ToolError>;

/// Stable, embedding-friendly tool surface for host apps (CLI, Tauri, etc.).
///
/// This is the recommended entrypoint for downstream apps. It intentionally:
/// - avoids exposing `rzn_plan` internals
/// - returns stable, structured contracts (`rzn_contracts::v1`)
/// - keeps LLM prompting/planning ownership in the host application
pub struct BrowserTools {
    session: Session,
}

impl BrowserTools {
    pub async fn connect(transport: RuntimeTransport) -> crate::Result<Self> {
        let session = Session::connect(transport).await?;
        Ok(Self { session })
    }

    pub async fn connect_with_options(
        transport: RuntimeTransport,
        observe: ObserveOptions,
        action_timeout: Duration,
    ) -> crate::Result<Self> {
        let cfg = SessionConfig {
            snapshot_timeout: observe.timeout,
            action_timeout,
            snapshot_max_elements: observe.max_elements,
        };
        let session = Session::connect_with_config(transport, cfg).await?;
        Ok(Self { session })
    }

    pub fn transcript(&self) -> &TranscriptV1 {
        self.session.transcript()
    }

    pub fn last_snapshot(&self) -> Option<&SnapshotV1> {
        self.session.last_snapshot()
    }

    pub async fn observe(&mut self) -> ToolResult<SnapshotV1> {
        self.session.snapshot().await.map_err(map_session_err)
    }

    pub async fn act(&mut self, action: ActionV1) -> ToolResult<ActionResultV1> {
        let res = self.session.apply(action).await.map_err(map_session_err)?;
        ensure_success(&res)?;
        Ok(res)
    }

    pub async fn execute_steps(
        &mut self,
        actions: Vec<ActionV1>,
    ) -> ToolResult<Vec<ActionResultV1>> {
        let mut out = Vec::with_capacity(actions.len());
        for action in actions {
            let res = self.session.apply(action).await.map_err(map_session_err)?;
            ensure_success(&res)?;
            out.push(res);
        }
        Ok(out)
    }

    pub async fn click_encoded(
        &mut self,
        encoded_id: impl Into<String>,
    ) -> ToolResult<ActionResultV1> {
        self.act(ActionV1::ClickElement {
            target: TargetV1::from_encoded_id(encoded_id),
            random_offset: Some(true),
            timeout_ms: Some(5000),
        })
        .await
    }

    pub async fn fill_encoded(
        &mut self,
        encoded_id: impl Into<String>,
        value: impl Into<String>,
    ) -> ToolResult<ActionResultV1> {
        self.act(ActionV1::FillInputField {
            target: TargetV1::from_encoded_id(encoded_id),
            value: value.into(),
            clear_first: Some(true),
            simulate_typing: Some(true),
            delay_ms: Some(25),
            timeout_ms: Some(8000),
        })
        .await
    }

    pub async fn press_encoded(
        &mut self,
        encoded_id: impl Into<String>,
        key: impl Into<String>,
    ) -> ToolResult<ActionResultV1> {
        self.act(ActionV1::PressSpecialKey {
            target: TargetV1::from_encoded_id(encoded_id),
            key: key.into(),
            timeout_ms: Some(5000),
        })
        .await
    }

    pub async fn wait_for_encoded(
        &mut self,
        encoded_id: impl Into<String>,
        timeout_ms: u32,
    ) -> ToolResult<ActionResultV1> {
        self.act(ActionV1::WaitForElement {
            target: TargetV1::from_encoded_id(encoded_id),
            timeout_ms: Some(timeout_ms),
        })
        .await
    }

    pub async fn get_page_source(&mut self) -> ToolResult<String> {
        let res = self
            .session
            .apply(ActionV1::GetPageSource)
            .await
            .map_err(map_session_err)?;
        ensure_success(&res)?;
        extract_page_source_html(&res).ok_or(ToolError::UnexpectedResponse)
    }

    pub async fn close(&mut self) -> crate::Result<()> {
        self.session.close().await
    }
}

fn map_session_err(err: crate::Error) -> ToolError {
    match err {
        crate::Error::Session(crate::session::Error::Timeout(_)) => ToolError::Timeout,
        crate::Error::Session(crate::session::Error::TargetNotFound(id)) => {
            ToolError::TargetNotFound(id)
        }
        crate::Error::Session(crate::session::Error::InvalidTarget) => {
            ToolError::Validation("invalid target".to_string())
        }
        other => ToolError::Transport(other.to_string()),
    }
}

fn ensure_success(res: &ActionResultV1) -> ToolResult<()> {
    if res.success {
        return Ok(());
    }

    let code = res.error_code.clone();
    let message = res
        .error
        .clone()
        .or_else(|| {
            res.raw
                .as_ref()
                .and_then(|v| v.get("error_msg"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "unknown extension error".to_string());

    Err(ToolError::ExtensionError { code, message })
}

fn extract_page_source_html(res: &ActionResultV1) -> Option<String> {
    let raw = res.raw.as_ref()?;
    if let Some(s) = raw.get("html_content").and_then(|v| v.as_str()) {
        return Some(s.to_string());
    }
    if let Some(steps) = raw.get("steps").and_then(|v| v.as_array()) {
        for step in steps {
            if let Some(s) = step.pointer("/data/html_content").and_then(|v| v.as_str()) {
                return Some(s.to_string());
            }
        }
    }
    // Legacy results array
    if let Some(results) = raw.get("results").and_then(|v| v.as_array()) {
        for r in results {
            let is_page_source = r.get("type").and_then(|v| v.as_str()) == Some("page_source");
            if is_page_source {
                if let Some(s) = r.get("html").and_then(|v| v.as_str()) {
                    return Some(s.to_string());
                }
            }
        }
    }
    // Nested under result.results
    if let Some(results) = raw.pointer("/result/results").and_then(|v| v.as_array()) {
        for r in results {
            let is_page_source = r.get("type").and_then(|v| v.as_str()) == Some("page_source");
            if is_page_source {
                if let Some(s) = r.get("html").and_then(|v| v.as_str()) {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}
