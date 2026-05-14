use crate::native_runner::NativeRunMode;
use crate::supervisor::{self, SupervisorConfig};
use anyhow::Result;
use clap::{Args, ValueEnum};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const DEFAULT_MCP_REQUEST_TIMEOUT_MS: u64 = 30_000;

#[derive(Args, Debug, Clone)]
pub struct BrowserMcpArgs {
    /// Browser worker connection mode: auto | attach | spawn
    #[arg(long, value_enum, default_value_t = BrowserMcpMode::Auto)]
    mode: BrowserMcpMode,

    /// Override APP_BASE used to find broker_endpoint_v1.json
    #[arg(long)]
    app_base: Option<String>,

    /// Override endpoint path for broker_endpoint_v1.json
    #[arg(long)]
    endpoint_path: Option<String>,

    /// Worker command for spawn mode
    #[arg(long)]
    worker_cmd: Option<String>,

    /// Worker args for spawn mode (repeatable)
    #[arg(long = "worker-arg")]
    worker_args: Vec<String>,

    /// Allow browser calls to fall back to the legacy browser worker
    #[arg(long)]
    allow_legacy_worker_fallback: bool,

    /// Timeout for proxied worker tool calls
    #[arg(long, default_value_t = DEFAULT_MCP_REQUEST_TIMEOUT_MS)]
    request_timeout_ms: u64,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum BrowserMcpMode {
    Auto,
    Attach,
    Spawn,
}

impl BrowserMcpMode {
    fn into_native(self) -> NativeRunMode {
        match self {
            BrowserMcpMode::Auto => NativeRunMode::Auto,
            BrowserMcpMode::Attach => NativeRunMode::Attach,
            BrowserMcpMode::Spawn => NativeRunMode::Spawn,
        }
    }
}

impl BrowserMcpArgs {
    fn supervisor_config(self) -> SupervisorConfig {
        SupervisorConfig {
            app_base: self.app_base.map(Into::into),
            endpoint_path: self.endpoint_path,
            mode: self.mode.into_native(),
            worker_cmd: self.worker_cmd,
            worker_args: self.worker_args,
            allow_legacy_worker_fallback: self.allow_legacy_worker_fallback,
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

    fn shutdown<'a>(&'a mut self) -> BackendFuture<'a, ()>;
}

struct SupervisorBackend {
    config: SupervisorConfig,
    request_timeout_ms: u64,
    ready_checked: bool,
}

impl SupervisorBackend {
    fn new(config: SupervisorConfig, request_timeout_ms: u64) -> Self {
        Self {
            config,
            request_timeout_ms,
            ready_checked: false,
        }
    }

    async fn ensure_ready(&mut self) -> Result<()> {
        if !self.ready_checked {
            supervisor::ensure_running(self.config.clone()).await?;
            self.ready_checked = true;
        }
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
            let (method, params) = (
                "tools/call",
                json!({
                    "name": tool_name,
                    "arguments": arguments,
                    "timeout_ms": self.request_timeout_ms
                }),
            );
            let structured = supervisor::call(self.config.clone(), method, params).await?;
            Ok(build_tool_result(
                tool_result_text(tool_name, &structured),
                structured,
                false,
                HashMap::new(),
            ))
        })
    }

    fn shutdown<'a>(&'a mut self) -> BackendFuture<'a, ()> {
        Box::pin(async move {})
    }
}

struct BrowserMcpServer<B> {
    backend: B,
    shutdown_requested: bool,
}

impl<B: BrowserRuntimeMcpBackend> BrowserMcpServer<B> {
    fn new(backend: B) -> Self {
        Self {
            backend,
            shutdown_requested: false,
        }
    }

    async fn run_stdio(&mut self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut stdout = tokio::io::stdout();
        let mut line = String::new();

        while !self.shutdown_requested {
            line.clear();
            let bytes_read = reader.read_line(&mut line).await?;
            if bytes_read == 0 {
                break;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let response = match serde_json::from_str::<Value>(trimmed) {
                Ok(request) => self.handle_request(request).await,
                Err(err) => Some(jsonrpc_error(
                    None,
                    -32700,
                    &format!("Parse error: {}", err),
                )),
            };

            if let Some(response) = response {
                stdout
                    .write_all(serde_json::to_string(&response)?.as_bytes())
                    .await?;
                stdout.write_all(b"\n").await?;
                stdout.flush().await?;
            }
        }

        self.backend.shutdown().await;
        Ok(())
    }

    async fn handle_request(&mut self, request: Value) -> Option<Value> {
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = request.get("id").cloned();
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let is_notification = id.is_none();

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
                    match self.backend.call_tool(tool_name, arguments).await {
                        Ok(result) => result,
                        Err(err) => backend_unavailable_tool_result(tool_name, &err.to_string()),
                    }
                } else {
                    unknown_tool_result(tool_name)
                };

                if tool_name == "rzn.worker.shutdown" {
                    self.shutdown_requested = true;
                }

                Some(jsonrpc_result(id.unwrap_or(Value::Null), result))
            }
            _ if is_notification => None,
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
        "rzn.worker.health",
        "rzn.worker.shutdown",
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
            "name": "rzn.worker.health",
            "description": "Return worker health diagnostics",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false
            }
        },
        {
            "name": "rzn.worker.shutdown",
            "description": "Request the worker to shut down",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false
            }
        }
    ])
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
    let is_health = tool_name == "rzn.worker.health";
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
                    "note": "MCP calls route through the rzn-browser supervisor. The supervisor may still use the legacy worker as a compatibility backend."
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
    if structured.get("ok").and_then(|value| value.as_bool()) == Some(false) {
        return structured
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("browser tool failed")
            .to_string();
    }
    match tool_name {
        "browser.session_open" => "session opened",
        "browser.session_close" => "session closed",
        "browser.snapshot" => "snapshot captured",
        "browser.execute_step" => "step executed",
        "browser.poll_events" => "events polled",
        "rzn.worker.health" => "runtime health",
        "rzn.worker.shutdown" => "legacy worker shutdown requested",
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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    struct FakeBackend {
        calls: Vec<(String, Value)>,
        response: Value,
        error: Option<String>,
        shutdown_called: bool,
    }

    impl FakeBackend {
        fn ok(response: Value) -> Self {
            Self {
                calls: Vec::new(),
                response,
                error: None,
                shutdown_called: false,
            }
        }

        fn failing(error: &str) -> Self {
            Self {
                calls: Vec::new(),
                response: json!({}),
                error: Some(error.to_string()),
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
                if let Some(error) = self.error.clone() {
                    Err(anyhow!(error))
                } else {
                    Ok(self.response.clone())
                }
            })
        }

        fn shutdown<'a>(&'a mut self) -> BackendFuture<'a, ()> {
            Box::pin(async move {
                self.shutdown_called = true;
            })
        }
    }

    #[tokio::test]
    async fn tools_list_preserves_worker_tool_names() {
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
    async fn tools_call_forwards_known_worker_tool_and_arguments() {
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
                    "name": "rzn.worker.health",
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
}
