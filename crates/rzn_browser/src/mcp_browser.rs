use crate::run_store::{AppendRun, RunStore};
use crate::supervisor::{self, SupervisorConfig};
use anyhow::Result;
use clap::Args;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const DEFAULT_MCP_REQUEST_TIMEOUT_MS: u64 = 30_000;
const MAX_QUEUED_REQUESTS: usize = 32;

#[derive(Args, Debug, Clone)]
pub struct BrowserMcpArgs {
    /// Override APP_BASE for supervisor socket/token/runtime files
    #[arg(long)]
    app_base: Option<String>,

    /// Timeout for proxied supervisor tool calls
    #[arg(long, default_value_t = DEFAULT_MCP_REQUEST_TIMEOUT_MS)]
    request_timeout_ms: u64,
}

impl BrowserMcpArgs {
    fn supervisor_config(self) -> SupervisorConfig {
        SupervisorConfig {
            app_base: self.app_base.map(Into::into),
        }
    }
}

pub async fn run_browser_mcp_server(args: BrowserMcpArgs) -> Result<()> {
    // Keep stdout as pure JSON-RPC. Native attach/spawn status lines must go to stderr here.
    std::env::set_var("RZN_BROWSER_MCP_STDIO", "1");

    let request_timeout_ms = args.request_timeout_ms;
    let backend = SupervisorBackend::new(args.supervisor_config(), request_timeout_ms);
    let mut server = BrowserMcpServer::new(backend);
    server.run_stdio().await
}

type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

trait BrowserRuntimeMcpBackend {
    fn call_tool<'a>(
        &'a mut self,
        tool_name: &'a str,
        arguments: Value,
    ) -> BackendFuture<'a, Result<Value>>;

    fn cancel_request<'a>(&'a mut self, _request_id: &'a str) -> BackendFuture<'a, ()> {
        Box::pin(async {})
    }

    fn shutdown<'a>(&'a mut self) -> BackendFuture<'a, ()>;
}

struct SupervisorBackend {
    config: SupervisorConfig,
    request_timeout_ms: u64,
}

impl SupervisorBackend {
    fn new(config: SupervisorConfig, request_timeout_ms: u64) -> Self {
        Self {
            config,
            request_timeout_ms,
        }
    }

    async fn ensure_ready(&mut self) -> Result<()> {
        supervisor::ensure_running(self.config.clone()).await?;
        Ok(())
    }
}

impl BrowserRuntimeMcpBackend for SupervisorBackend {
    fn call_tool<'a>(
        &'a mut self,
        tool_name: &'a str,
        arguments: Value,
    ) -> BackendFuture<'a, Result<Value>> {
        Box::pin(async move {
            self.ensure_ready().await?;
            let started_at = chrono::Utc::now().timestamp_millis();
            let stored_params = arguments.clone();
            let (method, params) = (
                "tools/call",
                json!({
                    "name": tool_name,
                    "arguments": arguments,
                    "timeout_ms": self.request_timeout_ms
                }),
            );
            let structured = supervisor::call(self.config.clone(), method, params).await?;
            let run_result = supervisor::run_result_for_tool(tool_name, &structured);
            if let Ok(typed) =
                serde_json::from_value::<rzn_contracts::workflow::RunResult>(run_result.clone())
            {
                let base = self
                    .config
                    .app_base
                    .clone()
                    .unwrap_or_else(rzn_core::runtime_paths::default_app_base_dir);
                let appended = RunStore::open(base).and_then(|store| {
                    store
                        .append(AppendRun {
                            origin: "mcp",
                            workflow_hash: None,
                            started_at,
                            ended_at: chrono::Utc::now().timestamp_millis(),
                            params: &stored_params,
                            result: &typed,
                        })
                        .map(|_| ())
                });
                if appended.is_ok() {
                    let _ =
                        supervisor::call(self.config.clone(), "status.snapshot.refresh", json!({}))
                            .await;
                }
            }
            let is_error =
                run_result.get("status").and_then(|value| value.as_str()) != Some("succeeded");
            Ok(build_tool_result(
                tool_result_text(tool_name, &run_result),
                run_result,
                is_error,
                HashMap::from([("rzn_raw_supervisor_response".to_string(), structured)]),
            ))
        })
    }

    fn cancel_request<'a>(&'a mut self, request_id: &'a str) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            let _ = supervisor::call(
                self.config.clone(),
                "browser.cancel_pending",
                json!({ "mcp_request_id": request_id }),
            )
            .await;
        })
    }

    fn shutdown<'a>(&'a mut self) -> BackendFuture<'a, ()> {
        Box::pin(async move {})
    }
}

struct BrowserMcpServer<B> {
    backend: B,
    shutdown_requested: bool,
    /// Session ids returned by `browser.session_open` that have not yet been
    /// closed via `browser.session_close`. Closed best-effort on shutdown/EOF
    /// so an MCP client that disconnects without cleaning up doesn't leak
    /// sessions in the backend/extension.
    open_sessions: std::collections::HashSet<String>,
}

impl<B: BrowserRuntimeMcpBackend> BrowserMcpServer<B> {
    fn new(backend: B) -> Self {
        Self {
            backend,
            shutdown_requested: false,
            open_sessions: std::collections::HashSet::new(),
        }
    }

    fn track_session_lifecycle(&mut self, tool_name: &str, arguments: &Value, result: &Value) {
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        match tool_name {
            "browser.session_open" if !is_error => {
                if let Some(session_id) = extract_opened_session_id(result) {
                    self.open_sessions.insert(session_id);
                }
            }
            "browser.session_close" => {
                if let Some(session_id) = arguments.get("session_id").and_then(Value::as_str) {
                    self.open_sessions.remove(session_id);
                }
            }
            _ => {}
        }
    }

    /// Best-effort close of any sessions this server opened but never closed.
    /// Each close is bounded by a short timeout so a wedged backend can't
    /// hang process shutdown.
    async fn close_open_sessions(&mut self) {
        let session_ids: Vec<String> = self.open_sessions.drain().collect();
        for session_id in session_ids {
            let call = self
                .backend
                .call_tool("browser.session_close", json!({ "session_id": session_id }));
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), call).await;
        }
    }

    async fn run_stdio(&mut self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut stdout = tokio::io::stdout();
        self.run_io(&mut reader, &mut stdout).await
    }

    async fn run_io<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin>(
        &mut self,
        reader: &mut R,
        writer: &mut W,
    ) -> Result<()> {
        let result = self.process_io(reader, writer).await;
        self.close_open_sessions().await;
        self.backend.shutdown().await;
        result
    }

    async fn process_io<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin>(
        &mut self,
        reader: &mut R,
        writer: &mut W,
    ) -> Result<()> {
        let mut line = Vec::new();
        let mut queued = VecDeque::new();

        while !self.shutdown_requested {
            let request = if let Some(request) = queued.pop_front() {
                request
            } else {
                if reader.read_until(b'\n', &mut line).await? == 0 && line.is_empty() {
                    break;
                }
                if line.iter().all(u8::is_ascii_whitespace) {
                    line.clear();
                    continue;
                }
                let parsed = serde_json::from_slice::<Value>(&line);
                line.clear();
                match parsed {
                    Ok(request) => request,
                    Err(err) => {
                        write_response(
                            writer,
                            &jsonrpc_error(None, -32700, &format!("Parse error: {err}")),
                        )
                        .await?;
                        continue;
                    }
                }
            };

            if request.get("method").and_then(Value::as_str) == Some("tools/call")
                && request.get("id").is_some()
            {
                let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
                let tool_name = params.get("name").and_then(Value::as_str).unwrap_or("");
                if is_browser_tool(tool_name) {
                    let id = request["id"].clone();
                    let mut arguments = params
                        .get("arguments")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    let Some(argument_map) = arguments.as_object_mut() else {
                        write_response(
                            writer,
                            &jsonrpc_error(Some(id), -32602, "Tool arguments must be an object"),
                        )
                        .await?;
                        continue;
                    };
                    let request_token = uuid::Uuid::new_v4().to_string();
                    if tool_name.starts_with("browser.") {
                        argument_map.insert("mcp_request_id".into(), json!(request_token));
                    }
                    let tool_name = tool_name.to_string();
                    let mut call = Box::pin(self.backend.call_tool(&tool_name, arguments.clone()));
                    enum Outcome {
                        Complete(Result<Value>),
                        Cancelled,
                        Eof,
                    }
                    let outcome = loop {
                        tokio::select! {
                            result = &mut call => break Outcome::Complete(result),
                            bytes = reader.read_until(b'\n', &mut line) => {
                                if bytes? == 0 && line.is_empty() { break Outcome::Eof; }
                                if line.iter().all(u8::is_ascii_whitespace) {
                                    line.clear();
                                    continue;
                                }
                                let parsed = serde_json::from_slice::<Value>(&line);
                                line.clear();
                                let incoming = match parsed {
                                    Ok(value) => value,
                                    Err(err) => {
                                        write_response(writer, &jsonrpc_error(None, -32700, &format!("Parse error: {err}"))).await?;
                                        continue;
                                    }
                                };
                                if incoming.get("method").and_then(Value::as_str) == Some("notifications/cancelled") {
                                    if incoming.pointer("/params/requestId") == Some(&id) {
                                        break Outcome::Cancelled;
                                    }
                                    if let Some(cancelled_id) = incoming.pointer("/params/requestId") {
                                        if let Some(position) = queued.iter().position(|request: &Value| request.get("id") == Some(cancelled_id)) {
                                            queued.remove(position);
                                            write_response(writer, &jsonrpc_error(Some(cancelled_id.clone()), -32800, "Request cancelled")).await?;
                                        }
                                    }
                                } else if incoming.get("id").is_some() {
                                    if queued.len() < MAX_QUEUED_REQUESTS {
                                        queued.push_back(incoming);
                                    } else {
                                        write_response(writer, &jsonrpc_error(incoming.get("id").cloned(), -32000, "MCP request queue full")).await?;
                                    }
                                }
                            }
                        }
                    };
                    drop(call);
                    match outcome {
                        Outcome::Complete(Ok(result)) => {
                            self.track_session_lifecycle(&tool_name, &arguments, &result);
                            write_response(writer, &jsonrpc_result(id, result)).await?;
                        }
                        Outcome::Complete(Err(err)) => {
                            let result =
                                backend_unavailable_tool_result(&tool_name, &err.to_string());
                            write_response(writer, &jsonrpc_result(id, result)).await?;
                        }
                        Outcome::Cancelled => {
                            let _ = tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                self.backend.cancel_request(&request_token),
                            )
                            .await;
                            write_response(
                                writer,
                                &jsonrpc_error(Some(id), -32800, "Request cancelled"),
                            )
                            .await?;
                        }
                        Outcome::Eof => {
                            let _ = tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                self.backend.cancel_request(&request_token),
                            )
                            .await;
                            break;
                        }
                    }
                    continue;
                }
            }

            if let Some(response) = self.handle_request(request).await {
                write_response(writer, &response).await?;
            }
        }
        Ok(())
    }

    async fn handle_request(&mut self, request: Value) -> Option<Value> {
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = request.get("id").cloned();
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let is_notification = id.is_none();

        if is_notification {
            return None;
        }

        match method {
            "initialize" => Some(jsonrpc_result(
                id.unwrap_or(Value::Null),
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "serverInfo": {
                        "name": "rzn-browser",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": {
                        "tools": { "listChanged": false }
                    }
                }),
            )),
            "notifications/initialized" => None,
            "tools/list" => Some(jsonrpc_result(
                id.unwrap_or(Value::Null),
                json!({ "tools": browser_tool_list() }),
            )),
            "tools/call" => {
                let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));

                let result = if is_browser_tool(tool_name) {
                    match self.backend.call_tool(tool_name, arguments.clone()).await {
                        Ok(result) => {
                            self.track_session_lifecycle(tool_name, &arguments, &result);
                            result
                        }
                        Err(err) => backend_unavailable_tool_result(tool_name, &err.to_string()),
                    }
                } else {
                    unknown_tool_result(tool_name)
                };

                Some(jsonrpc_result(id.unwrap_or(Value::Null), result))
            }
            _ => Some(jsonrpc_error(
                id,
                -32601,
                &format!("Method not found: {}", method),
            )),
        }
    }
}

fn browser_tool_names() -> &'static [&'static str] {
    &[
        "browser.session_open",
        "browser.session_close",
        "browser.snapshot",
        "browser.execute_step",
        "browser.poll_events",
        "rzn.supervisor.health",
    ]
}

fn is_browser_tool(tool_name: &str) -> bool {
    browser_tool_names().contains(&tool_name)
}

fn browser_tool_list() -> Value {
    json!([
        {
            "name": "browser.session_open",
            "description": "Open a browser session via the extension/native host",
            "inputSchema": {
                "type": "object",
                "properties": { "url": { "type": "string" } },
                "additionalProperties": true
            }
        },
        {
            "name": "browser.session_close",
            "description": "Close a browser session",
            "inputSchema": {
                "type": "object",
                "properties": { "session_id": { "type": "string" } },
                "required": ["session_id"]
            }
        },
        {
            "name": "browser.snapshot",
            "description": "Get a page snapshot for a session",
            "inputSchema": {
                "type": "object",
                "properties": { "session_id": { "type": "string" } },
                "required": ["session_id"]
            }
        },
        {
            "name": "browser.execute_step",
            "description": "Execute an action step in the browser session",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "step": { "type": "object" }
                },
                "required": ["session_id", "step"],
                "additionalProperties": true
            }
        },
        {
            "name": "browser.poll_events",
            "description": "Poll for any pending browser events",
            "inputSchema": {
                "type": "object",
                "properties": { "session_id": { "type": "string" } },
                "required": ["session_id"]
            }
        },
        {
            "name": "rzn.supervisor.health",
            "description": "Return supervisor runtime diagnostics",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false
            }
        }
    ])
}

/// Pull the freshly-minted session id out of a `browser.session_open` tool
/// result. The real supervisor backend nests it under the raw response
/// (`metadata.rzn_raw_supervisor_response.session_id`) or under the run
/// result's `output`; test/fake backends may put it directly on
/// `structuredContent`. Check all three.
fn extract_opened_session_id(result: &Value) -> Option<String> {
    result
        .pointer("/metadata/rzn_raw_supervisor_response/session_id")
        .or_else(|| result.pointer("/structuredContent/session_id"))
        .or_else(|| result.pointer("/structuredContent/output/session_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn unknown_tool_result(tool_name: &str) -> Value {
    build_tool_result(
        "unknown tool".to_string(),
        json!({
            "ok": false,
            "error": format!("unknown tool: {}", tool_name)
        }),
        true,
        HashMap::new(),
    )
}

fn backend_unavailable_tool_result(tool_name: &str, error: &str) -> Value {
    let is_health = tool_name == "rzn.supervisor.health";
    build_tool_result(
        if is_health {
            "browser runtime health unavailable".to_string()
        } else {
            "browser runtime unavailable".to_string()
        },
        json!({
            "ok": false,
            "ready": false,
            "error": error,
            "details": {
                "backend": "rzn_supervisor",
                "supervisor_ipc": {
                    "available": false,
                    "status": "unavailable_or_not_ready",
                    "note": "MCP calls route through the rzn-browser supervisor and require the extension native-host bridge."
                },
                "remediation": [
                    "Run `rzn-browser supervisor ensure-ready`.",
                    "Confirm Chrome is open with the RZN extension enabled if browser calls need a live page."
                ]
            }
        }),
        !is_health,
        HashMap::new(),
    )
}

fn tool_result_text(tool_name: &str, structured: &Value) -> String {
    let status = structured.get("status").and_then(|value| value.as_str());
    if status.is_some_and(|status| status != "succeeded")
        || structured.get("success").and_then(|value| value.as_bool()) == Some(false)
        || structured.get("ok").and_then(|value| value.as_bool()) == Some(false)
    {
        return structured
            .get("error")
            .and_then(|value| value.as_str())
            .or_else(|| {
                structured
                    .pointer("/error/message")
                    .and_then(|value| value.as_str())
            })
            .unwrap_or("browser tool failed")
            .to_string();
    }
    match tool_name {
        "browser.session_open" => "session opened",
        "browser.session_close" => "session closed",
        "browser.snapshot" => "snapshot captured",
        "browser.execute_step" => "step executed",
        "browser.poll_events" => "events polled",
        "rzn.supervisor.health" => "runtime health",
        _ => "ok",
    }
    .to_string()
}

fn build_tool_result(
    text: String,
    structured: Value,
    is_error: bool,
    metadata: HashMap<String, Value>,
) -> Value {
    let mut obj = Map::new();
    obj.insert(
        "content".to_string(),
        json!([{ "type": "text", "text": text }]),
    );
    obj.insert("isError".to_string(), Value::Bool(is_error));
    obj.insert("structuredContent".to_string(), structured);
    if !metadata.is_empty() {
        let meta_obj: Map<String, Value> = metadata.into_iter().collect();
        obj.insert("metadata".to_string(), Value::Object(meta_obj));
    }
    Value::Object(obj)
}

fn jsonrpc_error(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": {
            "code": code,
            "message": message
        }
    })
}

fn jsonrpc_result(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

async fn write_response<W: AsyncWrite + Unpin>(writer: &mut W, response: &Value) -> Result<()> {
    writer
        .write_all(serde_json::to_string(response)?.as_bytes())
        .await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use tokio::io::AsyncReadExt;

    struct FakeBackend {
        calls: Vec<(String, Value)>,
        response: Value,
        error: Option<String>,
        hang_on: Option<String>,
        started: Option<std::sync::Arc<tokio::sync::Notify>>,
        release: Option<std::sync::Arc<tokio::sync::Notify>>,
        cancelled_requests: Vec<String>,
        shutdown_called: bool,
    }

    impl FakeBackend {
        fn ok(response: Value) -> Self {
            Self {
                calls: Vec::new(),
                response,
                error: None,
                hang_on: None,
                started: None,
                release: None,
                cancelled_requests: Vec::new(),
                shutdown_called: false,
            }
        }

        fn failing(error: &str) -> Self {
            Self {
                calls: Vec::new(),
                response: json!({}),
                error: Some(error.to_string()),
                hang_on: None,
                started: None,
                release: None,
                cancelled_requests: Vec::new(),
                shutdown_called: false,
            }
        }
    }

    impl BrowserRuntimeMcpBackend for FakeBackend {
        fn call_tool<'a>(
            &'a mut self,
            tool_name: &'a str,
            arguments: Value,
        ) -> BackendFuture<'a, Result<Value>> {
            Box::pin(async move {
                self.calls.push((tool_name.to_string(), arguments));
                if let Some(started) = &self.started {
                    started.notify_one();
                }
                if self.hang_on.as_deref() == Some(tool_name) {
                    if let Some(release) = &self.release {
                        release.notified().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                }
                if let Some(error) = self.error.clone() {
                    Err(anyhow!(error))
                } else {
                    Ok(self.response.clone())
                }
            })
        }

        fn cancel_request<'a>(&'a mut self, request_id: &'a str) -> BackendFuture<'a, ()> {
            Box::pin(async move {
                self.cancelled_requests.push(request_id.to_string());
            })
        }

        fn shutdown<'a>(&'a mut self) -> BackendFuture<'a, ()> {
            Box::pin(async move {
                self.shutdown_called = true;
            })
        }
    }

    #[tokio::test]
    async fn tools_list_preserves_browser_tool_names() {
        let mut server = BrowserMcpServer::new(FakeBackend::ok(json!({})));
        let response = server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list"
            }))
            .await
            .expect("response");

        let names: Vec<String> = response
            .pointer("/result/tools")
            .and_then(|value| value.as_array())
            .expect("tools")
            .iter()
            .map(|tool| {
                tool.get("name")
                    .and_then(|value| value.as_str())
                    .expect("name")
                    .to_string()
            })
            .collect();

        assert_eq!(names, browser_tool_names());
    }

    #[tokio::test]
    async fn tools_call_forwards_known_browser_tool_and_arguments() {
        let result = build_tool_result(
            "ok".to_string(),
            json!({ "ok": true, "session_id": "s1" }),
            false,
            HashMap::new(),
        );
        let mut server = BrowserMcpServer::new(FakeBackend::ok(result));
        let response = server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": "call-1",
                "method": "tools/call",
                "params": {
                    "name": "browser.snapshot",
                    "arguments": { "session_id": "s1" }
                }
            }))
            .await
            .expect("response");

        assert_eq!(server.backend.calls.len(), 1);
        assert_eq!(server.backend.calls[0].0, "browser.snapshot");
        assert_eq!(server.backend.calls[0].1, json!({ "session_id": "s1" }));
        assert_eq!(
            response.pointer("/result/structuredContent/session_id"),
            Some(&json!("s1"))
        );
    }

    #[tokio::test]
    async fn health_backend_failure_returns_non_error_diagnostic() {
        let mut server = BrowserMcpServer::new(FakeBackend::failing("no supervisor socket"));
        let response = server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": "health-1",
                "method": "tools/call",
                "params": {
                    "name": "rzn.supervisor.health",
                    "arguments": {}
                }
            }))
            .await
            .expect("response");

        assert_eq!(response.pointer("/result/isError"), Some(&json!(false)));
        assert_eq!(
            response.pointer("/result/structuredContent/details/supervisor_ipc/available"),
            Some(&json!(false))
        );
        assert_eq!(
            response.pointer("/result/structuredContent/error"),
            Some(&json!("no supervisor socket"))
        );
    }

    #[tokio::test]
    async fn non_health_backend_failure_returns_tool_error() {
        let mut server = BrowserMcpServer::new(FakeBackend::failing("no worker"));
        let response = server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": "snapshot-1",
                "method": "tools/call",
                "params": {
                    "name": "browser.snapshot",
                    "arguments": { "session_id": "s1" }
                }
            }))
            .await
            .expect("response");

        assert_eq!(response.pointer("/result/isError"), Some(&json!(true)));
        assert_eq!(
            response.pointer("/result/structuredContent/ready"),
            Some(&json!(false))
        );
    }

    #[tokio::test]
    async fn session_open_is_tracked_and_closed_on_shutdown() {
        let open_result = build_tool_result(
            "session opened".to_string(),
            json!({ "ok": true, "session_id": "leaked-1" }),
            false,
            HashMap::new(),
        );
        let mut server = BrowserMcpServer::new(FakeBackend::ok(open_result));
        server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": "browser.session_open", "arguments": {} }
            }))
            .await;

        assert!(server.open_sessions.contains("leaked-1"));

        server.close_open_sessions().await;

        assert!(server.open_sessions.is_empty());
        assert_eq!(server.backend.calls.len(), 2);
        assert_eq!(server.backend.calls[1].0, "browser.session_close");
        assert_eq!(
            server.backend.calls[1].1,
            json!({ "session_id": "leaked-1" })
        );
    }

    #[tokio::test]
    async fn explicit_session_close_untracks_session() {
        let open_result = build_tool_result(
            "session opened".to_string(),
            json!({ "ok": true, "session_id": "s-explicit" }),
            false,
            HashMap::new(),
        );
        let mut server = BrowserMcpServer::new(FakeBackend::ok(open_result));
        server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": "browser.session_open", "arguments": {} }
            }))
            .await;
        assert!(server.open_sessions.contains("s-explicit"));

        server
            .handle_request(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "browser.session_close",
                    "arguments": { "session_id": "s-explicit" }
                }
            }))
            .await;

        assert!(server.open_sessions.is_empty());

        // Shutdown should not attempt to close it again since it's untracked.
        server.close_open_sessions().await;
        assert_eq!(server.backend.calls.len(), 2);
    }

    #[tokio::test]
    async fn matching_cancel_interrupts_hung_call_without_replying_to_notifications() {
        let mut backend = FakeBackend::ok(json!({}));
        backend.hang_on = Some("browser.snapshot".into());
        let started = std::sync::Arc::new(tokio::sync::Notify::new());
        backend.started = Some(started.clone());
        let mut server = BrowserMcpServer::new(backend);
        let (client, server_io) = tokio::io::duplex(8192);
        let (server_read, mut server_write) = tokio::io::split(server_io);
        let mut server_read = BufReader::new(server_read);
        let (mut client_read, mut client_write) = tokio::io::split(client);

        let (server_result, output) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(async {
                let result = server.run_io(&mut server_read, &mut server_write).await;
                server_write.shutdown().await.unwrap();
                result
            }, async {
                client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":\"call-1\",\"method\":\"tools/call\",\"params\":{\"name\":\"browser.snapshot\",\"arguments\":{\"session_id\":\"owned\"}}}\n").await.unwrap();
                started.notified().await;
                client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":\"other\"}}\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":\"queued\",\"method\":\"tools/call\",\"params\":{\"name\":\"browser.snapshot\",\"arguments\":{\"session_id\":\"other\"}}}\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":\"queued\"}}\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":\"call-1\"}}\n{\"jsonrpc\":\"2.0\",\"id\":\"list-1\",\"method\":\"tools/list\"}\n").await.unwrap();
                client_write.shutdown().await.unwrap();
                let mut output = String::new();
                client_read.read_to_string(&mut output).await.unwrap();
                output
            })
        }).await.expect("hung call must cancel promptly");

        server_result.unwrap();
        let responses: Vec<Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(responses.len(), 3);
        assert_eq!(responses[0].pointer("/error/code"), Some(&json!(-32800)));
        assert_eq!(responses[0]["id"], "queued");
        assert_eq!(responses[1].pointer("/error/code"), Some(&json!(-32800)));
        assert_eq!(responses[1]["id"], "call-1");
        assert_eq!(responses[2]["id"], "list-1");
        assert_eq!(server.backend.calls.len(), 1);
        let token = server.backend.calls[0].1["mcp_request_id"]
            .as_str()
            .unwrap();
        assert_eq!(server.backend.cancelled_requests.len(), 1);
        assert_eq!(server.backend.cancelled_requests[0], token);
        assert!(server.backend.shutdown_called);
    }

    #[tokio::test]
    async fn eof_interrupts_hung_call_and_closes_only_owned_sessions() {
        let mut backend = FakeBackend::ok(json!({}));
        backend.hang_on = Some("browser.snapshot".into());
        let started = std::sync::Arc::new(tokio::sync::Notify::new());
        backend.started = Some(started.clone());
        let mut server = BrowserMcpServer::new(backend);
        server.open_sessions.insert("owned".into());
        let (client, server_io) = tokio::io::duplex(8192);
        let (server_read, mut server_write) = tokio::io::split(server_io);
        let mut server_read = BufReader::new(server_read);
        let (mut client_read, mut client_write) = tokio::io::split(client);

        let (server_result, output) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(async {
                let result = server.run_io(&mut server_read, &mut server_write).await;
                server_write.shutdown().await.unwrap();
                result
            }, async {
                client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"browser.snapshot\",\"arguments\":{\"session_id\":\"unrelated\"}}}\n").await.unwrap();
                started.notified().await;
                client_write.shutdown().await.unwrap();
                let mut output = String::new();
                client_read.read_to_string(&mut output).await.unwrap();
                output
            })
        }).await.expect("EOF must interrupt hung call promptly");

        server_result.unwrap();
        assert!(output.is_empty());
        assert_eq!(server.backend.cancelled_requests.len(), 1);
        assert_eq!(server.backend.calls.len(), 2);
        assert_eq!(
            server.backend.calls[1],
            (
                "browser.session_close".into(),
                json!({"session_id":"owned"})
            )
        );
        assert!(server.backend.shutdown_called);
    }

    #[tokio::test]
    async fn partial_next_request_survives_active_call_completion() {
        let mut backend = FakeBackend::ok(json!({ "ok": true }));
        backend.hang_on = Some("browser.snapshot".into());
        let started = std::sync::Arc::new(tokio::sync::Notify::new());
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        backend.started = Some(started.clone());
        backend.release = Some(release.clone());
        let mut server = BrowserMcpServer::new(backend);
        let (client, server_io) = tokio::io::duplex(8192);
        let (server_read, mut server_write) = tokio::io::split(server_io);
        let mut server_read = BufReader::new(server_read);
        let (client_read, mut client_write) = tokio::io::split(client);
        let mut client_read = BufReader::new(client_read);

        let (server_result, responses) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(async {
                let result = server.run_io(&mut server_read, &mut server_write).await;
                server_write.shutdown().await.unwrap();
                result
            }, async {
                client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":\"first\",\"method\":\"tools/call\",\"params\":{\"name\":\"browser.snapshot\",\"arguments\":{}}}\n").await.unwrap();
                started.notified().await;
                client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":\"next\",").await.unwrap();
                tokio::task::yield_now().await;
                tokio::task::yield_now().await;
                release.notify_one();
                let mut first = String::new();
                client_read.read_line(&mut first).await.unwrap();
                client_write.write_all(b"\"method\":\"tools/list\"}\n").await.unwrap();
                client_write.shutdown().await.unwrap();
                let mut rest = String::new();
                client_read.read_to_string(&mut rest).await.unwrap();
                format!("{first}{rest}")
            })
        }).await.expect("partial request must survive call completion");

        server_result.unwrap();
        let responses: Vec<Value> = responses
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0]["id"], "first");
        assert_eq!(responses[1]["id"], "next");
        assert_eq!(server.backend.calls.len(), 1);
    }
}
