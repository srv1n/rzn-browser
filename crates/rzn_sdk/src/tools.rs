use crate::host::RuntimeTransport;
use crate::session::{Session, SessionConfig};
use rzn_contracts::browser::{
    BrowserAction, BrowserActionResult, BrowserSnapshot, BrowserTarget, BrowserTranscript,
};
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

    #[error("stale_element_id: {0}")]
    StaleElementId(String),

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
/// - returns stable, structured browser contracts (`rzn_contracts::browser`)
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

    pub fn transcript(&self) -> &BrowserTranscript {
        self.session.transcript()
    }

    pub fn last_snapshot(&self) -> Option<&BrowserSnapshot> {
        self.session.last_snapshot()
    }

    pub async fn observe(&mut self) -> ToolResult<BrowserSnapshot> {
        self.session.snapshot().await.map_err(map_session_err)
    }

    pub async fn act(&mut self, action: BrowserAction) -> ToolResult<BrowserActionResult> {
        let res = self.session.apply(action).await.map_err(map_session_err)?;
        ensure_success(&res)?;
        Ok(res)
    }

    pub async fn execute_steps(
        &mut self,
        actions: Vec<BrowserAction>,
    ) -> ToolResult<Vec<BrowserActionResult>> {
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
    ) -> ToolResult<BrowserActionResult> {
        self.act(BrowserAction::ClickElement {
            target: BrowserTarget::from_encoded_id(encoded_id),
            random_offset: Some(true),
            timeout_ms: Some(5000),
        })
        .await
    }

    pub async fn fill_encoded(
        &mut self,
        encoded_id: impl Into<String>,
        value: impl Into<String>,
    ) -> ToolResult<BrowserActionResult> {
        self.act(BrowserAction::FillInputField {
            target: BrowserTarget::from_encoded_id(encoded_id),
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
    ) -> ToolResult<BrowserActionResult> {
        self.act(BrowserAction::PressSpecialKey {
            target: BrowserTarget::from_encoded_id(encoded_id),
            key: key.into(),
            timeout_ms: Some(5000),
        })
        .await
    }

    pub async fn wait_for_encoded(
        &mut self,
        encoded_id: impl Into<String>,
        timeout_ms: u32,
    ) -> ToolResult<BrowserActionResult> {
        self.act(BrowserAction::WaitForElement {
            target: BrowserTarget::from_encoded_id(encoded_id),
            timeout_ms: Some(timeout_ms),
        })
        .await
    }

    pub async fn get_page_source(&mut self) -> ToolResult<String> {
        let res = self
            .session
            .apply(BrowserAction::GetPageSource)
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
        crate::Error::Session(crate::session::Error::StaleElementId { .. }) => {
            ToolError::StaleElementId(err.to_string())
        }
        crate::Error::Session(crate::session::Error::InvalidTarget) => {
            ToolError::Validation("invalid target".to_string())
        }
        other => ToolError::Transport(other.to_string()),
    }
}

fn ensure_success(res: &BrowserActionResult) -> ToolResult<()> {
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

fn extract_page_source_html(res: &BrowserActionResult) -> Option<String> {
    let raw = res.raw.as_ref()?;
    raw.pointer("/result/html")
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn result(raw: Value) -> BrowserActionResult {
        BrowserActionResult {
            success: true,
            error_code: None,
            error: None,
            current_url: None,
            current_tab_id: None,
            current_tab_ref: None,
            dom_hash: None,
            dom_snapshot: None,
            capabilities: None,
            raw: Some(raw),
        }
    }

    #[test]
    fn page_source_uses_direct_result_shape() {
        let response = result(json!({
            "success": true,
            "result": {"type": "page_source", "html": "<html></html>"}
        }));

        assert_eq!(
            extract_page_source_html(&response).as_deref(),
            Some("<html></html>")
        );
    }

    #[test]
    fn page_source_ignores_retired_aggregate_shapes() {
        let response = result(json!({
            "html_content": "<html>top-level</html>",
            "steps": [{"data": {"html_content": "<html>step</html>"}}],
            "results": [{"type": "page_source", "html": "<html>results</html>"}],
            "result": {"results": [{"type": "page_source", "html": "<html>nested</html>"}]}
        }));

        assert_eq!(extract_page_source_html(&response), None);
    }
}
