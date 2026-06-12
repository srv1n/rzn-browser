use crate::dom_processor::{DomContext, DomElement};
use crate::element_ref::{ElementBounds, InputRung, ResolvedElement, ResultEnvelope, TargetSpec};
use crate::{PlanError, PlanResult};
use base64::{engine::general_purpose, Engine as _};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use interprocess::local_socket::{
    tokio::Stream as LocalSocketStream, traits::tokio::Stream as _, GenericFilePath,
    GenericNamespaced, ToFsName, ToNsName,
};
use log::{debug, error, info, warn};
use rzn_core::framing::{read_required_frame, write_frame, MAX_FRAME_SIZE};
use rzn_core::runtime_paths::{
    candidate_app_bases, default_app_base_dir, env_trimmed as shared_env_trimmed, first_env_path,
    infer_app_base_from_executable, supervisor_paths_for_app_base, APP_BASE_ENV_KEYS,
    SUPERVISOR_SOCKET_ENV_KEYS, SUPERVISOR_TOKEN_ENV_KEYS,
};
use rzn_core::{Step, StepKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{
    collections::HashMap,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use uuid::Uuid;

const BROKER_PORT: u16 = 30123;
const COMPRESSION_THRESHOLD: usize = 40 * 1024; // 40KB - compress messages larger than this
const SUPERVISOR_ATTACH_TIMEOUT_MS: u64 = 2_500;
const SUPERVISOR_REQUEST_TIMEOUT_MS: u64 = 45_000;

// Optimization #9: Const strings for repeated operations
const ACTION_PERFORM_TASK: &str = "perform_task";
const ACTION_PING: &str = "ping";
const TASK_ID_PING: &str = "ping";
const GET_HTML_STEP_ID: &str = "get_html";
const GET_HTML_STEP_NAME: &str = "Get page HTML";

/// Transform Option<T> fields for Chrome extension compatibility.
/// Converts Rust Option serialization from {"Some": value} to value, and {"None": null} to null
fn transform_options_for_extension(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            // IMPORTANT: Do not rename keys here.
            // The broker + extension protocol uses snake_case (task_id, req_id, timeout_ms, etc.).
            // Renaming to camelCase breaks routing/correlation and causes "requestId: unknown".
            for (_key, field_value) in map.iter_mut() {
                if let serde_json::Value::Object(inner_map) = field_value {
                    if inner_map.len() == 1 && inner_map.contains_key("Some") {
                        *field_value = inner_map.remove("Some").unwrap_or(serde_json::Value::Null);
                    } else if inner_map.len() == 1 && inner_map.contains_key("None") {
                        *field_value = serde_json::Value::Null;
                    } else if inner_map.is_empty() {
                        *field_value = serde_json::Value::Null;
                    }
                }

                // Recursively transform after unwrapping Option if needed.
                transform_options_for_extension(field_value);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                transform_options_for_extension(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::transform_options_for_extension;
    use super::{
        connect_pipe_fs, connect_pipe_ns, read_response_from_broker, SupervisorEndpointClient,
        SupervisorEndpointSpec,
    };
    use interprocess::local_socket::{
        traits::tokio::Listener as _, GenericFilePath, ListenerOptions, ToFsName,
    };
    use rzn_core::framing::write_frame;
    use serde_json::{json, Value};
    use tokio::io::duplex;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn transform_options_keeps_snake_case_keys() {
        let mut v = serde_json::json!({
            "action": "perform_task",
            "task_id": "plan-1",
            "task": {
                "steps": [],
            },
            "data": {
                "current_tab_id": { "Some": 123 },
                "maybe_null": { "None": null }
            }
        });

        transform_options_for_extension(&mut v);

        // Top-level must remain snake_case.
        assert_eq!(v.get("task_id").and_then(|v| v.as_str()), Some("plan-1"));
        assert!(v.get("taskId").is_none());

        // Nested keys must remain snake_case.
        let data = v.get("data").expect("data");
        assert_eq!(data.get("current_tab_id"), Some(&serde_json::json!(123)));
        assert!(data.get("currentTabId").is_none());
        assert_eq!(data.get("maybe_null"), Some(&serde_json::json!(null)));
    }

    #[tokio::test]
    async fn read_response_ignores_unrelated_messages_until_match() {
        let (mut client, mut server) = duplex(16 * 1024);

        // Unrelated out-of-band message (e.g., extension heartbeat).
        let ping = serde_json::json!({
            "cmd": "ping",
            "req_id": "ping-123",
            "payload": {}
        })
        .to_string();
        write_frame(&mut server, ping.as_bytes()).await.unwrap();

        // Matching broker response for our request id.
        let resp = serde_json::json!({
            "action": "task_result",
            "task_id": "plan-1",
            "success": true,
            "result": {
                "results": [
                    { "type": "page_source", "html": "<html/>" }
                ]
            },
            "html_content": "<html/>",
            "steps": [
                { "data": { "html_content": "<html/>" } }
            ]
        })
        .to_string();
        write_frame(&mut server, resp.as_bytes()).await.unwrap();

        let v = read_response_from_broker(&mut client, Some("plan-1"))
            .await
            .expect("expected matched response");

        assert!(v.get("cmd").is_none(), "should not return the ping message");
        assert!(
            v.get("results").is_some(),
            "expected unwrapped result to contain results"
        );
        assert!(
            v.get("html_content").is_some(),
            "expected html_content to be preserved into result"
        );
        assert!(
            v.get("steps").is_some(),
            "expected steps to be preserved into result"
        );
    }

    #[tokio::test]
    async fn pipe_socket_name_conversion_errors_are_returned() {
        let fs_error = connect_pipe_fs("/tmp/rzn-\0.sock")
            .await
            .expect_err("invalid filesystem socket path should return an error")
            .to_string();
        assert!(fs_error.contains("Invalid filesystem socket path"));

        let ns_error = connect_pipe_ns("rzn-\0.sock")
            .await
            .expect_err("invalid namespaced socket should return an error")
            .to_string();
        assert!(ns_error.contains("Invalid namespaced socket name"));
    }

    #[tokio::test]
    async fn supervisor_endpoint_client_reconnects_after_bad_frame_read() {
        let root = std::path::PathBuf::from(format!("/tmp/rzp-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&root).expect("test root");
        let socket = root.join("supervisor.sock");
        let token_path = root.join("token");
        std::fs::write(&token_path, "test-token").expect("token");
        let _ = std::fs::remove_file(&socket);

        let name = socket
            .to_string_lossy()
            .to_string()
            .to_fs_name::<GenericFilePath>()
            .expect("socket path");
        let listener = ListenerOptions::new()
            .name(name)
            .create_tokio()
            .expect("listener");
        let endpoint = SupervisorEndpointSpec {
            socket: socket.to_string_lossy().to_string(),
            token_path: token_path.to_string_lossy().to_string(),
        };

        let server = tokio::spawn(async move {
            for index in 0..2 {
                let mut stream = listener.accept().await.expect("accept");
                let _handshake = rzn_core::framing::read_required_frame(&mut stream)
                    .await
                    .expect("handshake frame");
                write_frame(
                    &mut stream,
                    &serde_json::to_vec(&json!({ "ok": true })).unwrap(),
                )
                .await
                .expect("handshake response");

                let request = rzn_core::framing::read_required_frame(&mut stream)
                    .await
                    .expect("request frame");
                let request: Value = serde_json::from_slice(&request).expect("request json");
                if index == 0 {
                    stream
                        .write_all(&0u32.to_le_bytes())
                        .await
                        .expect("bad frame header");
                    stream.flush().await.expect("flush bad frame");
                    continue;
                }

                let response = json!({
                    "jsonrpc": "2.0",
                    "id": request.get("id").cloned().unwrap_or(Value::Null),
                    "result": { "ready": true }
                });
                write_frame(&mut stream, &serde_json::to_vec(&response).unwrap())
                    .await
                    .expect("response frame");
            }
        });

        let mut client = SupervisorEndpointClient::connect(&endpoint)
            .await
            .expect("connect client");
        let error = client
            .call_tool("runtime.status", json!({}))
            .await
            .expect_err("bad frame should reset connection");
        assert!(error.to_string().contains("connection reset"));

        let response = client
            .call_tool("runtime.status", json!({}))
            .await
            .expect("second call uses reconnected stream");
        assert_eq!(response.get("ready"), Some(&json!(true)));

        server.await.expect("server task");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn static_command_context_is_threaded_into_payload_and_data() {
        let mut value = serde_json::json!({
            "cmd": "observe",
            "payload": {},
            "use_current_tab": true
        });

        super::inject_static_command_context(&mut value, "session-1", Some(42));

        assert_eq!(
            value.pointer("/payload/session_id"),
            Some(&serde_json::json!("session-1"))
        );
        assert_eq!(
            value.pointer("/payload/current_tab_id"),
            Some(&serde_json::json!(42))
        );
        assert_eq!(
            value.pointer("/payload/use_current_tab"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            value.pointer("/data/session_id"),
            Some(&serde_json::json!("session-1"))
        );
        assert_eq!(
            value.pointer("/data/current_tab_id"),
            Some(&serde_json::json!(42))
        );
        assert_eq!(
            value.pointer("/data/use_current_tab"),
            Some(&serde_json::json!(true))
        );
    }
}
const RZN_SOCK_PATH: &str = "rzn.sock";

/// Element representation from the extension's DOM snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementStub {
    #[serde(default)]
    pub id: Option<String>,
    pub tag: String,
    pub text: Option<String>,
    pub attributes: HashMap<String, String>,
    pub selector: String,
    pub spatial_info: Option<SpatialInfo>,
}

/// Spatial information for an element
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpatialInfo {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub area: i32,
    pub viewport_position: String, // "top", "middle", "bottom"
}

/// DOM snapshot from the extension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomSnapshot {
    pub elements: Vec<ElementStub>,
    pub hash: String,
    pub prompt: String,
    pub metadata: DomMetadata,
    pub delta: Option<DomDelta>,
}

/// Metadata about the DOM snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomMetadata {
    pub timestamp: u64,
    pub url: String,
    pub title: String,
    pub viewport: Viewport,
}

/// Viewport information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

/// DOM delta for incremental updates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomDelta {
    pub added: Vec<ElementStub>,
    pub removed: Vec<ElementStub>,
    pub modified: Vec<ElementStub>,
}

/// Transport type for broker communication
#[derive(Debug, Clone)]
pub enum Transport {
    Tcp,
    Pipe,
    Native,
}

/// Session information for maintaining state across tasks
#[derive(Debug, Clone)]
pub struct BrokerSession {
    pub session_id: String,
    pub current_tab_id: Option<u32>,
    pub current_url: Option<String>,
    pub last_heartbeat: Option<std::time::Instant>,
}

impl Default for BrokerSession {
    fn default() -> Self {
        Self::new()
    }
}

impl BrokerSession {
    pub fn new() -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            current_tab_id: None,
            current_url: None,
            last_heartbeat: None,
        }
    }
}

enum EitherStream {
    Tcp(TcpStream),
    Pipe(LocalSocketStream),
}

#[derive(Clone)]
struct SupervisorEndpointSpec {
    socket: String,
    token_path: String,
}

struct SupervisorEndpointClient {
    endpoint: SupervisorEndpointSpec,
    reader: Box<dyn AsyncRead + Unpin + Send>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
}

impl SupervisorEndpointClient {
    async fn connect(endpoint: &SupervisorEndpointSpec) -> PlanResult<Self> {
        let stream = tokio::time::timeout(
            Duration::from_millis(SUPERVISOR_ATTACH_TIMEOUT_MS),
            LocalSocketStream::connect(endpoint.socket.clone().to_fs_name::<GenericFilePath>()?),
        )
        .await
        .map_err(|_| {
            PlanError::BrokerError(format!(
                "Timed out connecting to supervisor endpoint {}",
                endpoint.socket
            ))
        })?
        .map_err(|e| {
            PlanError::BrokerError(format!(
                "Failed to connect to supervisor endpoint {}: {}",
                endpoint.socket, e
            ))
        })?;

        let (reader, writer) = tokio::io::split(stream);
        let mut client = Self {
            endpoint: endpoint.clone(),
            reader: Box::new(reader),
            writer: Box::new(writer),
        };
        client.handshake(Path::new(&endpoint.token_path)).await?;
        Ok(client)
    }

    async fn reconnect(&mut self) -> PlanResult<()> {
        let replacement = Self::connect(&self.endpoint).await?;
        *self = replacement;
        Ok(())
    }

    async fn handshake(&mut self, token_path: &Path) -> PlanResult<()> {
        let token = fs::read_to_string(token_path)
            .map_err(|e| PlanError::BrokerError(format!("Read supervisor token: {}", e)))?;
        let handshake = json!({
            "type": "rzn_local_handshake",
            "v": 1,
            "token": token.trim(),
            "client": {
                "name": "rzn-plan",
                "pid": std::process::id()
            }
        });
        let bytes = serde_json::to_vec(&handshake)?;
        write_frame(&mut self.writer, &bytes).await.map_err(|e| {
            PlanError::BrokerError(format!("Failed to send supervisor handshake: {}", e))
        })?;
        let response = tokio::time::timeout(
            Duration::from_millis(SUPERVISOR_ATTACH_TIMEOUT_MS),
            read_required_frame(&mut self.reader),
        )
        .await
        .map_err(|_| PlanError::BrokerError("Supervisor handshake timed out".to_string()))?
        .map_err(|e| PlanError::BrokerError(format!("Supervisor handshake failed: {}", e)))?;
        let value: Value = serde_json::from_slice(&response)?;
        if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok(())
        } else {
            Err(PlanError::BrokerError(format!(
                "Supervisor rejected handshake: {}",
                value
            )))
        }
    }

    async fn call_tool(&mut self, name: &str, arguments: Value) -> PlanResult<Value> {
        let id = format!("req-{}", Uuid::new_v4());
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": name,
            "params": arguments
        });
        let bytes = serde_json::to_vec(&request)?;
        write_frame(&mut self.writer, &bytes).await.map_err(|e| {
            PlanError::BrokerError(format!("Failed to write supervisor request: {}", e))
        })?;

        let response = match tokio::time::timeout(
            Duration::from_millis(SUPERVISOR_REQUEST_TIMEOUT_MS),
            read_matching_jsonrpc_frame(&mut self.reader, &id),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                let reconnect_result = self.reconnect().await;
                let message = match reconnect_result {
                    Ok(()) => format!("Supervisor request read failed; connection reset: {error}"),
                    Err(reconnect_error) => format!(
                        "Supervisor request read failed: {error}; reconnect failed: {reconnect_error}"
                    ),
                };
                return Err(PlanError::BrokerError(message));
            }
            Err(_) => {
                let reconnect_result = self.reconnect().await;
                let message = match reconnect_result {
                    Ok(()) => "Supervisor request timed out; connection reset".to_string(),
                    Err(error) => {
                        format!("Supervisor request timed out; reconnect failed: {}", error)
                    }
                };
                return Err(PlanError::BrokerError(message));
            }
        };

        if let Some(error) = response.get("error") {
            return Err(PlanError::BrokerError(format!(
                "Supervisor JSON-RPC error: {}",
                error
            )));
        }
        if let Some(structured) = response
            .pointer("/result/structuredContent")
            .or_else(|| response.pointer("/result/structured_content"))
        {
            return Ok(structured.clone());
        }
        if let Some(result) = response.get("result") {
            return Ok(result.clone());
        }
        Ok(response)
    }
}

impl AsyncRead for EitherStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut *self {
            EitherStream::Tcp(stream) => Pin::new(stream).poll_read(cx, buf),
            EitherStream::Pipe(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for EitherStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        match &mut *self {
            EitherStream::Tcp(stream) => Pin::new(stream).poll_write(cx, buf),
            EitherStream::Pipe(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        match &mut *self {
            EitherStream::Tcp(stream) => Pin::new(stream).poll_flush(cx),
            EitherStream::Pipe(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        match &mut *self {
            EitherStream::Tcp(stream) => Pin::new(stream).poll_shutdown(cx),
            EitherStream::Pipe(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

/// CDP attachment state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CdpState {
    Detached,
    Attaching,
    Attached,
}

/// Client for communicating with the rzn broker
pub struct BrokerClient {
    transport: Transport,
    connection: Option<EitherStream>,
    supervisor_client: Option<SupervisorEndpointClient>,
    pub session: BrokerSession,
    task_counter: std::sync::atomic::AtomicU64, // Optimization #9: monotonic counter
    current_dom_context: Option<DomContext>,    // Current DOM context for frame_id resolution
    element_cache: HashMap<String, DomElement>, // Cache elements by selector for frame_id lookup
    current_dom_snapshot: Option<DomSnapshot>,  // Current DOM snapshot from extension
    last_dom_hash: Option<String>,              // Last DOM hash for delta tracking

    // CDP-related state
    cdp_state: CdpState,
    resolved_elements: HashMap<String, ResolvedElement>, // Cache for resolved elements
}

impl BrokerClient {
    pub fn new(transport: Transport) -> Self {
        Self {
            transport,
            connection: None,
            supervisor_client: None,
            session: BrokerSession::new(),
            task_counter: std::sync::atomic::AtomicU64::new(1),
            current_dom_context: None,
            element_cache: HashMap::new(),
            current_dom_snapshot: None,
            last_dom_hash: None,
            cdp_state: CdpState::Detached,
            resolved_elements: HashMap::new(),
        }
    }

    /// Connect to the broker
    pub async fn connect(&mut self) -> PlanResult<()> {
        info!("Connecting to broker via {:?}", self.transport);

        match self.transport {
            Transport::Native => {
                let mut failures = Vec::new();
                for endpoint in supervisor_endpoint_specs() {
                    match SupervisorEndpointClient::connect(&endpoint).await {
                        Ok(mut client) => match wait_for_supervisor_ready(&mut client).await {
                            Ok(()) => {
                                info!(
                                    "Connected to browser supervisor endpoint: {}",
                                    endpoint.socket
                                );
                                self.supervisor_client = Some(client);
                                return Ok(());
                            }
                            Err(err) => {
                                failures.push(format!("{} not ready ({})", endpoint.socket, err))
                            }
                        },
                        Err(err) => {
                            failures.push(format!("{} ({})", endpoint.socket, err));
                        }
                    }
                }
                if supervisor_self_heal_enabled() {
                    match spawn_supervisor().await {
                        Ok(endpoint) => match SupervisorEndpointClient::connect(&endpoint).await {
                            Ok(mut client) => {
                                wait_for_supervisor_ready(&mut client).await?;
                                info!(
                                    "Spawned and connected browser supervisor: {}",
                                    endpoint.socket
                                );
                                self.supervisor_client = Some(client);
                                return Ok(());
                            }
                            Err(err) => {
                                failures.push(format!("{} after spawn ({})", endpoint.socket, err))
                            }
                        },
                        Err(err) => failures.push(format!("spawn browser supervisor ({})", err)),
                    }
                }
                return Err(PlanError::BrokerError(format!(
                    "Failed to connect to browser supervisor endpoint. Checked: {}",
                    if failures.is_empty() {
                        "<no supervisor socket/token found>".to_string()
                    } else {
                        failures.join("; ")
                    }
                )));
            }
            Transport::Tcp => {
                let mut retries = 5;
                let stream = loop {
                    let addr = format!("127.0.0.1:{}", BROKER_PORT);
                    match TcpStream::connect(&addr).await {
                        Ok(s) => break s,
                        Err(_e) if retries > 0 => {
                            retries -= 1;
                            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                            continue;
                        }
                        Err(e) => {
                            return Err(PlanError::BrokerError(format!(
                                "Failed to connect to broker: {}",
                                e
                            )))
                        }
                    }
                };
                self.connection = Some(EitherStream::Tcp(stream));
            }
            Transport::Pipe => {
                // Try both namespaced (e.g., "rzn.sock") and filesystem path (e.g., "/tmp/rzn.sock")
                // to match whatever the broker bound to.
                // Priority order: env override -> namespaced -> /tmp path.

                let env_sock = std::env::var("RZN_SOCK_PATH").ok();
                let candidates: Vec<(&'static str, Option<String>)> = vec![
                    ("env", env_sock.clone()),
                    ("ns", Some(RZN_SOCK_PATH.to_string())),
                    ("fs", Some("/tmp/rzn.sock".to_string())),
                ];

                let mut retries = 5;
                let stream = loop {
                    let mut last_err: Option<anyhow::Error> = None;
                    let mut connected: Option<LocalSocketStream> = None;

                    for (kind, val) in &candidates {
                        let Some(name) = val.as_ref() else { continue };
                        match *kind {
                            // If env path looks absolute, prefer filesystem connect; otherwise treat as namespaced
                            "env" => {
                                if name.starts_with('/') {
                                    info!("Attempting to connect to pipe (fs): {}", name);
                                    match connect_pipe_fs(name).await {
                                        Ok(s) => {
                                            info!("Connected to pipe (fs): {}", name);
                                            connected = Some(s);
                                            break;
                                        }
                                        Err(e) => {
                                            last_err = Some(anyhow::anyhow!(e));
                                        }
                                    }
                                } else {
                                    info!("Attempting to connect to pipe (ns): {}", name);
                                    match connect_pipe_ns(name).await {
                                        Ok(s) => {
                                            info!("Connected to pipe (ns): {}", name);
                                            connected = Some(s);
                                            break;
                                        }
                                        Err(e) => {
                                            last_err = Some(anyhow::anyhow!(e));
                                        }
                                    }
                                }
                            }
                            "ns" => {
                                let n = name;
                                info!("Attempting to connect to pipe (ns): {}", n);
                                match connect_pipe_ns(n).await {
                                    Ok(s) => {
                                        info!("Connected to pipe (ns): {}", n);
                                        connected = Some(s);
                                        break;
                                    }
                                    Err(e) => {
                                        last_err = Some(anyhow::anyhow!(e));
                                    }
                                }
                            }
                            _ => {
                                let n = name;
                                info!("Attempting to connect to pipe (fs): {}", n);
                                match connect_pipe_fs(n).await {
                                    Ok(s) => {
                                        info!("Connected to pipe (fs): {}", n);
                                        connected = Some(s);
                                        break;
                                    }
                                    Err(e) => {
                                        last_err = Some(anyhow::anyhow!(e));
                                    }
                                }
                            }
                        }
                    }

                    if let Some(s) = connected {
                        break s;
                    }

                    // If we reached here, none of the candidates connected
                    if retries == 0 {
                        let msg = format!(
                            "Failed to connect to pipe via candidates (env='{}', ns='{}', fs='/tmp/rzn.sock'): {}",
                            env_sock.clone().unwrap_or_default(),
                            RZN_SOCK_PATH,
                            last_err.map(|e| e.to_string()).unwrap_or_else(|| "unknown error".into())
                        );
                        return Err(PlanError::BrokerError(msg));
                    }

                    retries -= 1;
                    info!("Pipe connect failed; retries left: {}", retries);
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                };

                self.connection = Some(EitherStream::Pipe(stream));
            }
        }

        info!("Successfully connected to broker");
        Ok(())
    }

    fn is_connected(&self) -> bool {
        match self.transport {
            Transport::Native => self.supervisor_client.is_some(),
            Transport::Tcp | Transport::Pipe => self.connection.is_some(),
        }
    }

    /// Execute a single step through the broker
    pub async fn execute_step(&mut self, step: &Step) -> PlanResult<Value> {
        self.execute_step_standard(step).await
    }

    /// Execute a single step through the broker, requesting a compact response from the extension.
    ///
    /// This disables DOM snapshot forwarding in the extension's workflow executor to keep
    /// native-messaging payload sizes small and prevent disconnects on heavy pages.
    pub async fn execute_step_compact(&mut self, step: &Step) -> PlanResult<Value> {
        self.execute_step_standard_internal(step, false).await
    }

    // Removed: execute_step_with_robust_selectors method - simplified to use standard execution only

    /// Standard step execution without fallbacks
    pub async fn execute_step_standard(&mut self, step: &Step) -> PlanResult<Value> {
        self.execute_step_standard_internal(step, true).await
    }

    async fn execute_step_standard_internal(
        &mut self,
        step: &Step,
        include_dom_snapshot: bool,
    ) -> PlanResult<Value> {
        if !self.is_connected() {
            self.connect().await?;
        }

        // Heartbeat check before execution (optimization #5)
        // Commenting out for now as extension doesn't handle ping
        // self.ensure_connection_health().await?;

        let task_id = format!(
            "plan-{}",
            self.task_counter
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );

        // Clone and augment step with frame_id and shadow DOM info if available
        let mut augmented_step = step.clone();
        self.augment_step_with_context(&mut augmented_step).await?;

        // Create task with session information
        let task = rzn_core::dsl::Task {
            steps: vec![augmented_step.clone()],
            search_query: None,
        };

        // Debug: Log the step being sent
        debug!(
            "Sending step to extension: {}",
            serde_json::to_string_pretty(&augmented_step).unwrap_or_default()
        );

        let message = rzn_core::dsl::Message {
            action: ACTION_PERFORM_TASK.to_string(),
            task_id: task_id.clone(),
            task: Some(task),
            data: Some(json!({
                "session_id": self.session.session_id,
                "current_tab_id": self.session.current_tab_id,
                "include_dom_snapshot": include_dom_snapshot
            })),
        };

        debug!(
            "Sending task to broker: {}",
            serde_json::to_string_pretty(&message)
                .unwrap_or_else(|_| "Failed to serialize".to_string())
        );

        let response = self.send_message(message).await?;

        // Update session state from response
        self.update_session_from_response(&response);

        Ok(response)
    }

    /// Execute a batch of steps through the broker and return the full response
    pub async fn execute_steps(&mut self, steps: Vec<Step>) -> PlanResult<Value> {
        if !self.is_connected() {
            self.connect().await?;
        }

        // Augment each step with available context (frame_id/shadow hints)
        let mut augmented_steps: Vec<Step> = Vec::with_capacity(steps.len());
        for mut s in steps.into_iter() {
            // Best-effort augmentation; ignore errors to avoid blocking execution
            let _ = self.augment_step_with_context(&mut s).await;
            augmented_steps.push(s);
        }

        let task_id = format!(
            "plan-{}",
            self.task_counter
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );

        let task = rzn_core::dsl::Task {
            steps: augmented_steps,
            search_query: None,
        };

        let message = rzn_core::dsl::Message {
            action: ACTION_PERFORM_TASK.to_string(),
            task_id: task_id.clone(),
            task: Some(task),
            data: Some(json!({
                "session_id": self.session.session_id,
                "current_tab_id": self.session.current_tab_id
            })),
        };

        debug!(
            "Sending batch task to broker: {}",
            serde_json::to_string_pretty(&message)
                .unwrap_or_else(|_| "<serialize error>".to_string())
        );

        let response = self.send_message(message).await?;

        // Update session state from response (tab id, url, dom snapshot)
        self.update_session_from_response(&response);

        Ok(response)
    }

    /// Augment step with frame_id and shadow DOM information from current DOM context
    async fn augment_step_with_context(&mut self, step: &mut Step) -> PlanResult<()> {
        // Only use existing DOM context to avoid recursion
        // Don't try to refresh DOM context here as it would lead to infinite recursion

        // Extract selector from step kind
        let selector = match &step.kind {
            StepKind::ClickElement { selector, .. } => Some(selector.clone()),
            StepKind::FillInputField { selector, .. } => Some(selector.clone()),
            StepKind::WaitForElement { selector, .. } => Some(selector.clone()),
            StepKind::PressSpecialKey { selector, .. } => selector.clone(),
            _ => None,
        };

        if let Some(sel) = selector {
            // Look for element in cache or DOM context
            if let Some(element) = self.find_element_with_context(&sel) {
                // Augment step with frame_id if element has one
                if let Some(frame_id) = element.attributes.get("_frameId") {
                    self.set_frame_id_on_step(step, frame_id.clone());
                    debug!("Augmented step with frame_id: {}", frame_id);
                }

                // Check for shadow DOM
                if element.attributes.contains_key("_shadow") {
                    self.set_shadow_flag_on_step(step, true);
                    debug!("Augmented step with shadow DOM flag");
                }
            }
        }

        Ok(())
    }

    /// Find element in current DOM context by selector
    fn find_element_with_context(&self, selector: &str) -> Option<&DomElement> {
        // First check cache
        if let Some(element) = self.element_cache.get(selector) {
            return Some(element);
        }

        // Then check DOM context
        if let Some(context) = &self.current_dom_context {
            // Simple selector matching - in a real implementation this would be more sophisticated
            for element in &context.interactive_elements {
                // Check if any of the element's suggested selectors match
                if element.selector_suggestions.contains(&selector.to_string()) {
                    return Some(element);
                }

                // Basic matching for common patterns
                if selector.contains(&element.tag) {
                    if let Some(id) = &element.id {
                        if selector.contains(id) {
                            return Some(element);
                        }
                    }
                    for class in &element.classes {
                        if selector.contains(class) {
                            return Some(element);
                        }
                    }
                }
            }
        }

        None
    }

    /// Set frame_id on step based on step type
    fn set_frame_id_on_step(&self, step: &mut Step, frame_id: String) {
        match &mut step.kind {
            StepKind::ClickElement {
                frame_id: ref mut fid,
                ..
            } => *fid = Some(frame_id),
            StepKind::FillInputField {
                frame_id: ref mut fid,
                ..
            } => *fid = Some(frame_id),
            StepKind::WaitForElement {
                frame_id: ref mut fid,
                ..
            } => *fid = Some(frame_id),
            StepKind::PressSpecialKey {
                frame_id: ref mut fid,
                ..
            } => *fid = Some(frame_id),
            _ => {} // Other step types don't support frame_id
        }
    }

    /// Set shadow DOM flag on step (if supported by step type)
    fn set_shadow_flag_on_step(&self, step: &mut Step, _shadow: bool) {
        // For now, shadow DOM support is implemented in the extension
        // We could add a shadow field to step types in the future
        debug!("Shadow DOM flag noted for step: {}", step.name);
    }

    /// Refresh DOM context by getting current page HTML and processing it
    async fn refresh_dom_context(&mut self) -> PlanResult<()> {
        match self.get_current_dom().await {
            Ok(html) => {
                let url = self
                    .get_current_url()
                    .unwrap_or_else(|| "unknown".to_string());

                // Process DOM using our DOM processor
                let processor = crate::dom_processor::DomProcessor::with_defaults();
                match processor.extract_dom_context(&html, &url) {
                    Ok(context) => {
                        // Update element cache
                        self.element_cache.clear();
                        for element in &context.interactive_elements {
                            for selector in &element.selector_suggestions {
                                self.element_cache.insert(selector.clone(), element.clone());
                            }
                        }

                        self.current_dom_context = Some(context);
                        debug!(
                            "Refreshed DOM context with {} elements",
                            self.element_cache.len()
                        );
                        Ok(())
                    }
                    Err(e) => {
                        warn!("Failed to process DOM context: {:?}", e);
                        Err(PlanError::BrokerError(format!(
                            "DOM processing failed: {}",
                            e
                        )))
                    }
                }
            }
            Err(e) => {
                warn!("Failed to get current DOM: {:?}", e);
                Err(e)
            }
        }
    }

    // Removed: generate_fallback_steps method - robust selectors functionality removed

    // Removed: normalize_selector method - was only used by removed robust selectors functionality

    /// Execute a step and get DOM in a single task (maintains tab state)
    pub async fn execute_step_and_get_dom(&mut self, step: &Step) -> PlanResult<(Value, String)> {
        if !self.is_connected() {
            self.connect().await?;
        }

        let task_id = format!(
            "plan-{}",
            self.task_counter
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );

        // Create task with both the step and get_html, including session info
        let mut steps = vec![step.clone()];
        steps.push(Step {
            id: GET_HTML_STEP_ID.to_string(),
            name: GET_HTML_STEP_NAME.to_string(),
            kind: rzn_core::StepKind::GetPageSource,
        });

        let task = rzn_core::dsl::Task {
            steps,
            search_query: None,
        };

        let message = rzn_core::dsl::Message {
            action: ACTION_PERFORM_TASK.to_string(),
            task_id: task_id.clone(),
            task: Some(task),
            data: Some(json!({
                "session_id": self.session.session_id,
                "current_tab_id": self.session.current_tab_id
            })),
        };

        debug!(
            " Session state being sent: session_id={}, current_tab_id={:?}",
            self.session.session_id, self.session.current_tab_id
        );
        debug!(
            "Sending combined task to broker: {}",
            serde_json::to_string_pretty(&message)
                .unwrap_or_else(|_| "Failed to serialize".to_string())
        );

        let response = self.send_message(message).await?;

        // Update session state from response (this will also update dom_snapshot)
        self.update_session_from_response(&response);

        // Extract step result
        let step_result = response.clone();

        // First try to get DOM content from the new dom_snapshot format
        if let Some(dom_snapshot_value) = response.get("dom_snapshot") {
            match serde_json::from_value::<DomSnapshot>(dom_snapshot_value.clone()) {
                Ok(snapshot) => {
                    debug!(
                        "📸 Using DOM snapshot with {} elements, returning formatted prompt",
                        snapshot.elements.len()
                    );
                    return Ok((step_result, snapshot.prompt));
                }
                Err(e) => {
                    warn!(
                        "Failed to parse DOM snapshot, falling back to HTML extraction: {:?}",
                        e
                    );
                }
            }
        }

        // Check if we have a cached snapshot we can use
        if let Some(snapshot) = &self.current_dom_snapshot {
            debug!(
                "📸 Using cached DOM snapshot with {} elements",
                snapshot.elements.len()
            );
            return Ok((step_result, snapshot.prompt.clone()));
        }

        // Fallback: First try top-level html_content
        if let Some(html_content) = response.get("html_content") {
            if let Some(html_str) = html_content.as_str() {
                return Ok((step_result, html_str.to_string()));
            }
        }

        // Then try steps array
        if let Some(steps) = response.get("steps") {
            if let Some(steps_array) = steps.as_array() {
                // Look for get_html step result or dom_snapshot data
                for step_result_item in steps_array {
                    // Check for DOM snapshot in step result
                    if let Some(dom_snapshot_value) = step_result_item.get("dom_snapshot") {
                        match serde_json::from_value::<DomSnapshot>(dom_snapshot_value.clone()) {
                            Ok(snapshot) => {
                                debug!(
                                    "📸 Found DOM snapshot in step result with {} elements",
                                    snapshot.elements.len()
                                );
                                return Ok((step_result, snapshot.prompt));
                            }
                            Err(e) => {
                                warn!("Failed to parse DOM snapshot from step result: {:?}", e);
                            }
                        }
                    }

                    // Fallback to HTML content in step data
                    if let Some(data) = step_result_item.get("data") {
                        if let Some(html_content) = data.get("html_content") {
                            if let Some(html_str) = html_content.as_str() {
                                return Ok((step_result.clone(), html_str.to_string()));
                            }
                        }
                    }
                }
            }
        }

        // Finally, check if results are nested under result.results
        if let Some(results) = response.get("results") {
            if let Some(results_array) = results.as_array() {
                for result in results_array {
                    if let Some(result_type) = result.get("type") {
                        if result_type.as_str() == Some("page_source") {
                            if let Some(html) = result.get("html") {
                                if let Some(html_str) = html.as_str() {
                                    return Ok((step_result, html_str.to_string()));
                                }
                            }
                        }
                    }
                }
            }
        }

        Err(PlanError::BrokerError(
            "No DOM content found in response".to_string(),
        ))
    }

    /// Execute multiple steps and get DOM in a single task (maintains tab state)
    pub async fn execute_steps_and_get_dom(
        &mut self,
        steps_in: Vec<Step>,
    ) -> PlanResult<(Value, String)> {
        if !self.is_connected() {
            self.connect().await?;
        }

        let task_id = format!(
            "plan-{}",
            self.task_counter
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );

        // Clone and augment steps with context where applicable
        let mut steps: Vec<Step> = Vec::with_capacity(steps_in.len() + 1);
        for mut s in steps_in.into_iter() {
            let _ = self.augment_step_with_context(&mut s).await; // best-effort
            steps.push(s);
        }
        // Append page source capture for stable post-state
        steps.push(Step {
            id: GET_HTML_STEP_ID.to_string(),
            name: GET_HTML_STEP_NAME.to_string(),
            kind: rzn_core::StepKind::GetPageSource,
        });

        let task = rzn_core::dsl::Task {
            steps,
            search_query: None,
        };

        let message = rzn_core::dsl::Message {
            action: ACTION_PERFORM_TASK.to_string(),
            task_id: task_id.clone(),
            task: Some(task),
            data: Some(json!({
                "session_id": self.session.session_id,
                "current_tab_id": self.session.current_tab_id
            })),
        };

        debug!(
            "Sending multi-step task to broker: {}",
            serde_json::to_string_pretty(&message)
                .unwrap_or_else(|_| "<serialize error>".to_string())
        );

        let response = self.send_message(message).await?;
        self.update_session_from_response(&response);

        // Prefer DOM snapshot prompt, else fallbacks identical to single-step path
        if let Some(dom_snapshot_value) = response.get("dom_snapshot") {
            if let Ok(snapshot) = serde_json::from_value::<DomSnapshot>(dom_snapshot_value.clone())
            {
                debug!(
                    "📸 Using DOM snapshot (multi-step) with {} elements",
                    snapshot.elements.len()
                );
                return Ok((response.clone(), snapshot.prompt));
            }
        }

        if let Some(snapshot) = &self.current_dom_snapshot {
            debug!(
                "📸 Using cached DOM snapshot (multi-step) with {} elements",
                snapshot.elements.len()
            );
            return Ok((response.clone(), snapshot.prompt.clone()));
        }

        if let Some(html_content) = response.get("html_content").and_then(|v| v.as_str()) {
            return Ok((response.clone(), html_content.to_string()));
        }

        if let Some(steps) = response.get("steps").and_then(|v| v.as_array()) {
            for step_result_item in steps {
                if let Some(dom_snapshot_value) = step_result_item.get("dom_snapshot") {
                    if let Ok(snapshot) =
                        serde_json::from_value::<DomSnapshot>(dom_snapshot_value.clone())
                    {
                        debug!(
                            "📸 Found DOM snapshot in step result (multi-step) with {} elements",
                            snapshot.elements.len()
                        );
                        return Ok((response.clone(), snapshot.prompt));
                    }
                }
                if let Some(data) = step_result_item.get("data") {
                    if let Some(html_content) = data.get("html_content").and_then(|v| v.as_str()) {
                        return Ok((response.clone(), html_content.to_string()));
                    }
                }
            }
        }

        if let Some(results) = response.get("results").and_then(|v| v.as_array()) {
            for result in results {
                if result.get("type").and_then(|t| t.as_str()) == Some("page_source") {
                    if let Some(html_str) = result.get("html").and_then(|h| h.as_str()) {
                        return Ok((response.clone(), html_str.to_string()));
                    }
                }
            }
        }

        Err(PlanError::BrokerError(
            "No DOM content found in multi-step response".to_string(),
        ))
    }

    /// Get current DOM from the browser
    pub async fn get_current_dom(&mut self) -> PlanResult<String> {
        let get_html_step = Step {
            id: GET_HTML_STEP_ID.to_string(),
            name: GET_HTML_STEP_NAME.to_string(),
            kind: rzn_core::StepKind::GetPageSource,
        };

        let response = match self.execute_step(&get_html_step).await {
            Ok(resp) => resp,
            Err(e) => {
                let error_msg = e.to_string();
                // Handle chrome:// URL errors gracefully at the broker client level
                if error_msg.contains("Cannot access")
                    || error_msg.contains("chrome://")
                    || error_msg.contains("chrome-extension://")
                    || error_msg.contains("RESTRICTED_URL")
                    || error_msg.contains("system pages")
                {
                    warn!(
                        "🚫 Broker client: Cannot access restricted URL, returning placeholder DOM"
                    );
                    return Ok("<html><body>chrome://newtab/</body></html>".to_string());
                }
                return Err(e);
            }
        };

        // Extract DOM content from response - prioritize new DOM snapshot format

        // First try to get DOM content from the new dom_snapshot format
        if let Some(dom_snapshot_value) = response.get("dom_snapshot") {
            match serde_json::from_value::<DomSnapshot>(dom_snapshot_value.clone()) {
                Ok(snapshot) => {
                    debug!(
                        "📸 Using DOM snapshot with {} elements for get_current_dom",
                        snapshot.elements.len()
                    );
                    return Ok(snapshot.prompt);
                }
                Err(e) => {
                    warn!(
                        "Failed to parse DOM snapshot, falling back to HTML extraction: {:?}",
                        e
                    );
                }
            }
        }

        // Check if we have a cached snapshot we can use
        if let Some(snapshot) = &self.current_dom_snapshot {
            debug!(
                "📸 Using cached DOM snapshot with {} elements for get_current_dom",
                snapshot.elements.len()
            );
            return Ok(snapshot.prompt.clone());
        }

        // Fallback: First try top-level html_content
        if let Some(html_content) = response.get("html_content") {
            if let Some(html_str) = html_content.as_str() {
                return Ok(html_str.to_string());
            }
        }

        // Then try steps array
        if let Some(steps) = response.get("steps") {
            if let Some(steps_array) = steps.as_array() {
                for step_result in steps_array {
                    // Check for DOM snapshot in step result
                    if let Some(dom_snapshot_value) = step_result.get("dom_snapshot") {
                        match serde_json::from_value::<DomSnapshot>(dom_snapshot_value.clone()) {
                            Ok(snapshot) => {
                                debug!("📸 Found DOM snapshot in step result for get_current_dom with {} elements", snapshot.elements.len());
                                return Ok(snapshot.prompt);
                            }
                            Err(e) => {
                                warn!("Failed to parse DOM snapshot from step result: {:?}", e);
                            }
                        }
                    }

                    // Fallback to HTML content in step data
                    if let Some(data) = step_result.get("data") {
                        if let Some(html_content) = data.get("html_content") {
                            if let Some(html_str) = html_content.as_str() {
                                return Ok(html_str.to_string());
                            }
                        }
                    }
                }
            }
        }

        // Finally, check if results are nested (e.g., under results array)
        if let Some(results) = response.get("results") {
            if let Some(results_array) = results.as_array() {
                for result in results_array {
                    if let Some(result_type) = result.get("type") {
                        if result_type.as_str() == Some("page_source") {
                            if let Some(html) = result.get("html") {
                                if let Some(html_str) = html.as_str() {
                                    return Ok(html_str.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        Err(PlanError::BrokerError(
            "No DOM content found in response".to_string(),
        ))
    }

    /// Get current URL from the session
    pub fn get_current_url(&self) -> Option<String> {
        self.session.current_url.clone()
    }

    /// Get current DOM snapshot if available
    pub fn get_current_dom_snapshot(&self) -> Option<&DomSnapshot> {
        self.current_dom_snapshot.as_ref()
    }

    /// Update DOM snapshot
    pub fn update_dom_snapshot(&mut self, snapshot: Option<DomSnapshot>) {
        self.current_dom_snapshot = snapshot;
    }

    /// Get DOM snapshot from extension (content script bridge) via execute_static
    pub async fn get_dom_snapshot(&mut self) -> PlanResult<Value> {
        if !self.is_connected() {
            self.connect().await?;
        }

        let request_id = format!("snap-{}", Uuid::new_v4());
        let message = rzn_core::dsl::Message {
            action: "execute_static".to_string(),
            task_id: request_id.clone(),
            task: None,
            data: Some(json!({
                "cmd": "get_dom_snapshot",
                // Keep snapshots compact to reduce token usage and log volume
                "payload": { "options": { "maxElements": 120, "highlightElements": false } }
            })),
        };

        let response = self.send_message(message).await?;
        Ok(response)
    }

    /// Get lightweight DOM hash for stability checks
    pub async fn get_dom_hash(&mut self) -> PlanResult<String> {
        if !self.is_connected() {
            self.connect().await?;
        }

        let request_id = format!("hash-{}", Uuid::new_v4());
        let message = rzn_core::dsl::Message {
            action: "execute_static".to_string(),
            task_id: request_id.clone(),
            task: None,
            data: Some(json!({
                "cmd": "get_dom_hash",
                "payload": {}
            })),
        };

        let response = self.send_message(message).await?;
        if let Some(hash) = response.get("hash").and_then(|h| h.as_str()) {
            return Ok(hash.to_string());
        }
        // Some responses nest data; try alternative shapes
        if let Some(hash) = response
            .get("result")
            .and_then(|r| r.get("hash"))
            .and_then(|h| h.as_str())
        {
            return Ok(hash.to_string());
        }
        Err(crate::PlanError::BrokerError(
            "No DOM hash in response".to_string(),
        ))
    }

    /// Enumerate DOM candidates with robust selectors (top frame)
    pub async fn process_dom(&mut self, options: Option<Value>) -> PlanResult<Value> {
        if !self.is_connected() {
            self.connect().await?;
        }
        let request_id = format!("procdom-{}", Uuid::new_v4());
        let message = rzn_core::dsl::Message {
            action: "execute_static".to_string(),
            task_id: request_id.clone(),
            task: None,
            data: Some(json!({
                "cmd": "process_dom",
                "payload": { "options": options.unwrap_or(json!({})) }
            })),
        };
        let response = self.send_message(message).await?;
        Ok(response)
    }

    /// Auto list detection (container/item selectors + per-item xpaths)
    pub async fn detect_auto_list(&mut self, options: Option<Value>) -> PlanResult<Value> {
        if !self.is_connected() {
            self.connect().await?;
        }
        let request_id = format!("autolist-{}", Uuid::new_v4());
        let message = rzn_core::dsl::Message {
            action: "execute_static".to_string(),
            task_id: request_id.clone(),
            task: None,
            data: Some(json!({
                "cmd": "detect_auto_list",
                "payload": { "options": options.unwrap_or(json!({})) }
            })),
        };
        let response = self.send_message(message).await?;
        Ok(response)
    }

    /// Execute a validated extraction plan (deterministic, no arbitrary JS execution).
    /// The plan is validated inside the extension before running.
    pub async fn execute_extraction_plan(&mut self, plan: Value) -> PlanResult<Value> {
        if !self.is_connected() {
            self.connect().await?;
        }

        let request_id = format!("explan-{}", Uuid::new_v4());
        let message = rzn_core::dsl::Message {
            action: "execute_static".to_string(),
            task_id: request_id.clone(),
            task: None,
            data: Some(json!({
                "cmd": "execute_extraction_plan",
                "payload": { "plan": plan }
            })),
        };

        let response = self.send_message(message).await?;
        Ok(response)
    }

    /// Set per-domain feature flags in the extension (execute_static → set_flags)
    pub async fn set_flags(&mut self, overrides: Value) -> PlanResult<Value> {
        if !self.is_connected() {
            self.connect().await?;
        }

        let request_id = format!("flags-{}", Uuid::new_v4());
        let message = rzn_core::dsl::Message {
            action: "execute_static".to_string(),
            task_id: request_id.clone(),
            task: None,
            data: Some(json!({
                "cmd": "set_flags",
                "payload": { "overrides": overrides }
            })),
        };

        let response = self.send_message(message).await?;
        Ok(response)
    }

    /// Break-glass: explicitly enable CDP for the current tab/session (time-bounded).
    ///
    /// This is intentionally opt-in. By default, CDP is disabled in the extension to avoid
    /// chrome.debugger attach (infobar + detectability). Hosts must deliberately request it.
    pub async fn enable_debug(&mut self, mode: &str, ttl_ms: Option<u32>) -> PlanResult<Value> {
        if !self.is_connected() {
            self.connect().await?;
        }

        // Minimal policy gate: require explicit host opt-in.
        // A richer confirmer-based policy can be layered in the host app later.
        let allow = std::env::var("RZN_ALLOW_CDP")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if !allow {
            return Err(PlanError::PolicyBlocked(
                "CDP is disabled by policy (set RZN_ALLOW_CDP=1 to enable break-glass)".to_string(),
            ));
        }

        let request_id = format!("dbg-{}", Uuid::new_v4());
        let message = rzn_core::dsl::Message {
            action: "execute_static".to_string(),
            task_id: request_id.clone(),
            task: None,
            data: Some(json!({
                "cmd": "enable_debug",
                "payload": {
                    "mode": mode,
                    "ttl_ms": ttl_ms.unwrap_or(120_000)
                }
            })),
        };

        let response = self.send_message(message).await?;
        Ok(response)
    }

    /// Break-glass: explicitly disable CDP for the current tab/session.
    pub async fn disable_debug(&mut self) -> PlanResult<Value> {
        if !self.is_connected() {
            self.connect().await?;
        }

        let request_id = format!("dbg-off-{}", Uuid::new_v4());
        let message = rzn_core::dsl::Message {
            action: "execute_static".to_string(),
            task_id: request_id.clone(),
            task: None,
            data: Some(json!({
                "cmd": "disable_debug",
                "payload": {}
            })),
        };

        let response = self.send_message(message).await?;
        Ok(response)
    }

    /// Execute a raw extension step payload (bypasses typed StepKind).
    /// Useful for extension-only options like `force_legacy` or `extraction_type`.
    pub async fn execute_raw_step(&mut self, step_payload: Value) -> PlanResult<Value> {
        if !self.is_connected() {
            self.connect().await?;
        }

        let request_id = format!("raw-{}", Uuid::new_v4());
        let message = rzn_core::dsl::Message {
            action: "execute_static".to_string(),
            task_id: request_id.clone(),
            task: None,
            data: Some(json!({
                "cmd": "execute_step",
                "payload": { "step": step_payload }
            })),
        };

        let response = self.send_message(message).await?;
        Ok(response)
    }

    /// Observe page to discover selectors/items with minimal payload (no LLM)
    pub async fn observe(
        &mut self,
        instruction: &str,
        scope_selector: Option<&str>,
        max_items: Option<u32>,
    ) -> PlanResult<Value> {
        if !self.is_connected() {
            self.connect().await?;
        }

        let request_id = format!("obs-{}", Uuid::new_v4());
        let message = rzn_core::dsl::Message {
            action: "execute_static".to_string(),
            task_id: request_id.clone(),
            task: None,
            data: Some(json!({
                "cmd": "observe",
                "payload": {
                    "instruction": instruction,
                    "scope_selector": scope_selector,
                    "max_items": max_items.unwrap_or(10)
                }
            })),
        };

        let response = self.send_message(message).await?;
        Ok(response)
    }

    /// Get CDP-based DOM context (accessibility/unified snapshot) directly from background via execute_static
    pub async fn get_cdp_context(&mut self, options: Option<Value>) -> PlanResult<Value> {
        if !self.is_connected() {
            self.connect().await?;
        }

        let request_id = format!("cdpctx-{}", Uuid::new_v4());
        // Force CDP inspection for selector inventory (preferCDP=true)
        let mut merged_opts = options.unwrap_or(json!({}));
        if !merged_opts
            .get("preferCDP")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            merged_opts["preferCDP"] = json!(true);
        }

        let message = rzn_core::dsl::Message {
            action: "execute_static".to_string(),
            task_id: request_id.clone(),
            task: None,
            data: Some(json!({
                "cmd": "get_cdp_context",
                "payload": {
                    "options": merged_opts
                }
            })),
        };

        let response = self.send_message(message).await?;
        Ok(response)
    }

    /// Get simplified AX tree text + id->url map from background (top-frame by default)
    pub async fn get_ax_tree(&mut self, include_frames: bool, max_nodes: u32) -> PlanResult<Value> {
        if !self.is_connected() {
            self.connect().await?;
        }

        let request_id = format!("axtree-{}", Uuid::new_v4());
        let message = rzn_core::dsl::Message {
            action: "execute_static".to_string(),
            task_id: request_id.clone(),
            task: None,
            data: Some(json!({
                "cmd": "get_ax_tree",
                "payload": { "includeFrames": include_frames, "maxNodes": max_nodes }
            })),
        };

        let response = self.send_message(message).await?;
        Ok(response)
    }

    /// Get interactive elements via CDP Accessibility
    pub async fn get_interactive_elements(&mut self) -> PlanResult<Value> {
        if !self.is_connected() {
            self.connect().await?;
        }

        let request_id = format!("ax-{}", Uuid::new_v4());
        let message = rzn_core::dsl::Message {
            action: "execute_static".to_string(),
            task_id: request_id.clone(),
            task: None,
            data: Some(json!({
                "cmd": "get_interactive_elements",
                "payload": {}
            })),
        };

        let response = self.send_message(message).await?;
        Ok(response)
    }

    /// Execute a CDP action via background (click/type) with optional encodedId
    pub async fn cdp_action(&mut self, action_type: &str, payload: Value) -> PlanResult<Value> {
        if !self.is_connected() {
            self.connect().await?;
        }

        let request_id = format!("cdpact-{}", Uuid::new_v4());
        let message = rzn_core::dsl::Message {
            action: "execute_static".to_string(),
            task_id: request_id.clone(),
            task: None,
            data: Some(json!({
                "cmd": "cdp_action",
                "payload": {
                    "type": action_type,
                    // Merge caller payload fields (selector, encodedId, text, value)
                    // The background/ladder will pick what it needs
                    "selector": payload.get("selector"),
                    "encodedId": payload.get("encodedId"),
                    "text": payload.get("text"),
                    "value": payload.get("value")
                }
            })),
        };

        let response = self.send_message(message).await?;
        Ok(response)
    }

    /// Get current DOM hash if available
    pub fn get_current_dom_hash(&self) -> Option<&String> {
        self.last_dom_hash.as_ref()
    }

    /// Apply DOM delta to current snapshot
    pub fn apply_dom_delta(&mut self, delta: DomDelta) -> PlanResult<()> {
        if let Some(snapshot) = &mut self.current_dom_snapshot {
            // Create a hashmap for fast lookups
            let mut element_map: HashMap<String, ElementStub> = snapshot
                .elements
                .iter()
                .map(|e| (e.selector.clone(), e.clone()))
                .collect();

            // Remove elements
            for removed in &delta.removed {
                element_map.remove(&removed.selector);
                debug!(" Removed element: {}", removed.selector);
            }

            // Add new elements
            for added in &delta.added {
                element_map.insert(added.selector.clone(), added.clone());
                debug!("➕ Added element: {}", added.selector);
            }

            // Update modified elements
            for modified in &delta.modified {
                element_map.insert(modified.selector.clone(), modified.clone());
                debug!(" Modified element: {}", modified.selector);
            }

            // Rebuild the elements list
            snapshot.elements = element_map.into_values().collect();

            // Note: We don't update the prompt here as it would require the toPrompt function
            // The prompt will be regenerated on the next DOM request
            debug!(
                " Applied DOM delta: {} elements after changes",
                snapshot.elements.len()
            );

            // Update element cache after applying delta
            self.update_element_cache_from_snapshot();

            Ok(())
        } else {
            Err(PlanError::BrokerError(
                "No current DOM snapshot to apply delta to".to_string(),
            ))
        }
    }

    /// Process delta message if present in response
    fn process_delta_message(&mut self, response: &Value) -> PlanResult<()> {
        if let Some(delta_value) = response.get("dom_delta") {
            match serde_json::from_value::<DomDelta>(delta_value.clone()) {
                Ok(delta) => {
                    debug!(
                        " Processing DOM delta: {} added, {} removed, {} modified",
                        delta.added.len(),
                        delta.removed.len(),
                        delta.modified.len()
                    );
                    self.apply_dom_delta(delta)?;
                }
                Err(e) => {
                    warn!("Failed to parse DOM delta: {:?}", e);
                }
            }
        }
        Ok(())
    }

    /// Convert ElementStub to legacy DomElement format for compatibility
    fn element_stub_to_dom_element(&self, stub: &ElementStub) -> DomElement {
        DomElement {
            tag: stub.tag.clone(),
            id: stub.attributes.get("id").cloned(),
            classes: stub
                .attributes
                .get("class")
                .map(|c| c.split_whitespace().map(|s| s.to_string()).collect())
                .unwrap_or_default(),
            attributes: stub.attributes.clone(),
            text_content: stub.text.clone().unwrap_or_default(),
            selector_suggestions: vec![stub.selector.clone()],
            frame_id: stub.attributes.get("_frameId").cloned(),
        }
    }

    /// Update element cache from current DOM snapshot
    fn update_element_cache_from_snapshot(&mut self) {
        if let Some(snapshot) = &self.current_dom_snapshot {
            self.element_cache.clear();
            for element_stub in &snapshot.elements {
                let dom_element = self.element_stub_to_dom_element(element_stub);
                self.element_cache
                    .insert(element_stub.selector.clone(), dom_element);

                // Also add by ID and class selectors for better lookup
                if let Some(id) = &element_stub.attributes.get("id") {
                    let id_selector = format!("#{}", id);
                    self.element_cache
                        .insert(id_selector, self.element_stub_to_dom_element(element_stub));
                }
            }
            debug!(
                " Updated element cache with {} elements from DOM snapshot",
                self.element_cache.len()
            );
        }
    }

    /// Disconnect from broker
    pub async fn disconnect(&mut self) -> PlanResult<()> {
        if let Some(stream) = &mut self.connection {
            let _ = stream.shutdown().await;
        }
        self.connection = None;
        self.supervisor_client = None;
        info!("Disconnected from broker");
        Ok(())
    }

    /// Update session state from broker response
    fn update_session_from_response(&mut self, response: &Value) {
        // Debug log the full response
        debug!(
            "[SEARCH] Full broker response: {}",
            serde_json::to_string_pretty(response)
                .unwrap_or_else(|_| "Failed to serialize".to_string())
        );

        // Extract tab ID from response if available
        if let Some(tab_id) = response.get("current_tab_id") {
            if let Some(tab_id_num) = tab_id.as_u64() {
                self.session.current_tab_id = Some(tab_id_num as u32);
                debug!("[OK] Updated session tab ID to: {}", tab_id_num);
            } else {
                debug!(
                    "[WARNING] Found current_tab_id but couldn't parse as u64: {:?}",
                    tab_id
                );
            }
        } else {
            debug!("[WARNING] No current_tab_id found in response");
        }

        // Extract current URL from response if available (top-level)
        if let Some(url) = response.get("current_url") {
            if let Some(url_str) = url.as_str() {
                self.session.current_url = Some(url_str.to_string());
                debug!("Updated session URL to: {}", url_str);
            }
        }

        // Extract DOM snapshot and hash if available
        if let Some(dom_snapshot_value) = response.get("dom_snapshot") {
            match serde_json::from_value::<DomSnapshot>(dom_snapshot_value.clone()) {
                Ok(snapshot) => {
                    debug!(
                        "📸 Updated DOM snapshot: {} elements, hash: {}",
                        snapshot.elements.len(),
                        snapshot.hash
                    );
                    self.last_dom_hash = Some(snapshot.hash.clone());
                    self.current_dom_snapshot = Some(snapshot);
                    self.update_element_cache_from_snapshot();
                }
                Err(e) => {
                    warn!("Failed to parse DOM snapshot: {:?}", e);
                }
            }
        }

        // Extract DOM hash separately if available (for cases where only hash is sent)
        if let Some(dom_hash) = response.get("dom_hash") {
            if let Some(hash_str) = dom_hash.as_str() {
                if self.last_dom_hash.as_deref() != Some(hash_str) {
                    debug!(" Updated DOM hash: {}", hash_str);
                    self.last_dom_hash = Some(hash_str.to_string());
                }
            }
        }

        // Process DOM delta if present
        if let Err(e) = self.process_delta_message(response) {
            warn!("Failed to process DOM delta: {:?}", e);
        }

        // Also check in steps array for navigation results (legacy support)
        if let Some(steps) = response.get("steps") {
            if let Some(steps_array) = steps.as_array() {
                for step_result in steps_array {
                    if let Some(step_type) = step_result.get("type") {
                        if step_type.as_str() == Some("navigate") {
                            if let Some(tab_id) = step_result.get("tab_id") {
                                if let Some(tab_id_num) = tab_id.as_u64() {
                                    self.session.current_tab_id = Some(tab_id_num as u32);
                                    debug!(
                                        "Updated session tab ID from navigate step: {}",
                                        tab_id_num
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Check connection health and reconnect if needed (optimization #5)
    async fn ensure_connection_health(&mut self) -> PlanResult<()> {
        const HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);
        const HEARTBEAT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

        let should_heartbeat = match self.session.last_heartbeat {
            Some(last) => last.elapsed() > HEARTBEAT_INTERVAL,
            None => true,
        };

        if should_heartbeat {
            let ping_result = tokio::time::timeout(HEARTBEAT_TIMEOUT, self.send_ping()).await;

            match ping_result {
                Ok(Ok(_)) => {
                    self.session.last_heartbeat = Some(std::time::Instant::now());
                    debug!("Heartbeat successful");
                }
                Ok(Err(_)) | Err(_) => {
                    warn!("Heartbeat failed, reconnecting...");
                    self.connection = None;
                    self.connect().await?;
                }
            }
        }

        Ok(())
    }

    /// Send a ping to check connection health
    async fn send_ping(&mut self) -> PlanResult<Value> {
        let ping_message = rzn_core::dsl::Message {
            action: ACTION_PING.to_string(),
            task_id: TASK_ID_PING.to_string(),
            task: None,
            data: None,
        };

        match &mut self.connection {
            Some(stream) => {
                let bytes =
                    serde_json::to_vec(&ping_message).map_err(PlanError::SerializationError)?;
                write_frame(stream, &bytes)
                    .await
                    .map_err(|e| PlanError::BrokerError(format!("Ping failed: {}", e)))?;

                // Simple success response for ping
                Ok(json!({"success": true, "type": "pong"}))
            }
            None => Err(PlanError::BrokerError("No connection for ping".to_string())),
        }
    }

    /// Send a message to the broker and get response
    pub async fn send_message(&mut self, message: rzn_core::dsl::Message) -> PlanResult<Value> {
        if matches!(self.transport, Transport::Native) {
            return self.send_message_via_supervisor(message).await;
        }

        //  FIX: Transform Option<T> fields before sending to broker/extension
        // Convert Rust Option serialization to plain JSON values
        let mut msg_value =
            serde_json::to_value(&message).map_err(PlanError::SerializationError)?;

        // Transform BEFORE serializing to bytes to avoid nested mutations
        transform_options_for_extension(&mut msg_value);

        let json_str = serde_json::to_string(&msg_value).map_err(PlanError::SerializationError)?;

        //  CRITICAL FIX: Compress large messages to prevent broker crashes
        let (final_message, was_compressed) = maybe_compress_message(&json_str)?;
        let bytes = final_message.as_bytes();

        if was_compressed {
            info!(
                " Sending compressed message to broker: {} bytes (original: {} bytes)",
                bytes.len(),
                json_str.len()
            );
        } else {
            debug!("Sending message to broker ({} bytes)", bytes.len());
        }

        // Try to send with automatic reconnection on failure
        // General retries; may be extended if the extension channel is down
        let mut retries = 4;
        let mut last_error = String::new();

        while retries > 0 {
            info!(
                "Attempting to send message to broker (retries left: {})",
                retries
            );

            match &mut self.connection {
                Some(stream) => {
                    info!("Connection exists, writing message bytes...");
                    match write_frame(stream, bytes).await {
                        Ok(_) => {
                            info!("Message sent successfully, waiting for response...");
                            // Successfully sent, now try to read response
                            match read_response_from_broker(stream, Some(&message.task_id)).await {
                                Ok(response) => {
                                    info!("Received response from broker successfully");
                                    return Ok(response);
                                }
                                Err(e) => {
                                    last_error =
                                        format!("Failed to read response from broker: {}", e);
                                    warn!("{}", last_error);
                                    self.connection = None; // Mark connection as dead
                                                            // Special backoff for extension channel closure to allow SW to reattach
                                    let is_ext_closed =
                                        last_error.contains("Extension channel closed");
                                    if is_ext_closed {
                                        info!("Detected extension channel closed. Waiting 1.2s before retry...");
                                        tokio::time::sleep(tokio::time::Duration::from_millis(
                                            1200,
                                        ))
                                        .await;
                                    }
                                    retries -= 1;

                                    if retries > 0 {
                                        info!("Attempting to reconnect to broker...");
                                        if let Err(conn_err) = self.connect().await {
                                            error!("Reconnection failed: {}", conn_err);
                                            last_error =
                                                format!("Reconnection failed: {}", conn_err);
                                        } else {
                                            info!("Successfully reconnected to broker");
                                            if is_ext_closed {
                                                // Extra small delay to give extension time to set up its channel
                                                tokio::time::sleep(
                                                    tokio::time::Duration::from_millis(500),
                                                )
                                                .await;
                                            }
                                            continue; // Retry the whole operation
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            last_error = format!("Failed to write to broker: {}", e);
                            warn!("{}", last_error);
                            self.connection = None; // Mark connection as dead
                            retries -= 1;

                            if retries > 0 {
                                info!("Attempting to reconnect to broker...");
                                if let Err(conn_err) = self.connect().await {
                                    error!("Reconnection failed: {}", conn_err);
                                    last_error = format!("Reconnection failed: {}", conn_err);
                                } else {
                                    info!("Successfully reconnected to broker");
                                    continue; // Retry sending
                                }
                            }
                        }
                    }
                }
                None => {
                    info!("No connection exists, attempting to connect...");
                    // Try to connect if not connected
                    if let Err(e) = self.connect().await {
                        last_error =
                            format!("Not connected to broker and failed to connect: {}", e);
                        return Err(PlanError::BrokerError(last_error));
                    }
                    info!("Connected successfully, retrying send...");
                    continue; // Retry sending after connection
                }
            }
        }

        Err(PlanError::BrokerError(format!(
            "Failed to communicate with broker after retries. Last error: {}",
            last_error
        )))
    }

    async fn send_message_via_supervisor(
        &mut self,
        message: rzn_core::dsl::Message,
    ) -> PlanResult<Value> {
        if self.supervisor_client.is_none() {
            self.connect().await?;
        }

        let data = message.data.clone().unwrap_or_else(|| json!({}));
        let session_id = data
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
            .unwrap_or_else(|| self.session.session_id.clone());

        if message.action != ACTION_PERFORM_TASK {
            return self
                .send_static_message_via_supervisor(message, data, session_id)
                .await;
        }

        let task = message
            .task
            .ok_or_else(|| PlanError::BrokerError("perform_task missing task".to_string()))?;
        let mut step_results = Vec::new();
        let mut final_result = json!({});
        let mut success = true;

        for step in task.steps {
            let mut step_value = serde_json::to_value(&step)?;
            transform_options_for_extension(&mut step_value);
            let mut args = json!({
                "session_id": session_id,
                "step": step_value
            });
            if let Some(use_current_tab) = data.get("use_current_tab").and_then(|v| v.as_bool()) {
                args["use_current_tab"] = Value::Bool(use_current_tab);
            }
            if let Some(use_active_tab) = data.get("use_active_tab").and_then(|v| v.as_bool()) {
                args["use_active_tab"] = Value::Bool(use_active_tab);
            }

            let structured = self
                .supervisor_client
                .as_mut()
                .ok_or_else(|| {
                    PlanError::BrokerError("Supervisor endpoint is not connected".to_string())
                })?
                .call_tool("browser.execute_step", args)
                .await?;
            let normalized = normalize_supervisor_tool_response(structured);
            if !supervisor_response_success(&normalized) {
                success = false;
            }
            final_result = normalized.clone();
            step_results.push(normalized);
            if !success {
                break;
            }
        }

        let mut response = if final_result.is_object() {
            final_result
        } else {
            json!({ "result": final_result })
        };
        if let Some(obj) = response.as_object_mut() {
            obj.insert("task_id".to_string(), Value::String(message.task_id));
            obj.insert("success".to_string(), Value::Bool(success));
            obj.insert("steps".to_string(), Value::Array(step_results));
        }
        Ok(response)
    }

    async fn send_static_message_via_supervisor(
        &mut self,
        message: rzn_core::dsl::Message,
        mut data: Value,
        session_id: String,
    ) -> PlanResult<Value> {
        let cmd = data.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
        let payload = data.get("payload").cloned().unwrap_or_else(|| json!({}));
        let mut args = payload;

        if cmd == "execute_step" {
            if args.get("step").is_none() {
                args = json!({ "step": args });
            }
            args["session_id"] = Value::String(session_id);
            let structured = self
                .supervisor_client
                .as_mut()
                .ok_or_else(|| {
                    PlanError::BrokerError("Supervisor endpoint is not connected".to_string())
                })?
                .call_tool("browser.execute_step", args)
                .await?;
            let mut response = normalize_supervisor_tool_response(structured);
            if let Some(obj) = response.as_object_mut() {
                obj.insert("task_id".to_string(), Value::String(message.task_id));
            }
            return Ok(response);
        }

        inject_static_command_context(&mut data, &session_id, self.session.current_tab_id);

        let structured = self
            .supervisor_client
            .as_mut()
            .ok_or_else(|| {
                PlanError::BrokerError("Supervisor endpoint is not connected".to_string())
            })?
            .call_tool("browser.static_command", data)
            .await?;
        let mut response = structured;
        if let Some(obj) = response.as_object_mut() {
            obj.entry("task_id".to_string())
                .or_insert_with(|| Value::String(message.task_id));
        }
        Ok(response)
    }
}

fn inject_static_command_context(data: &mut Value, session_id: &str, current_tab_id: Option<u32>) {
    let Some(obj) = data.as_object_mut() else {
        return;
    };
    let use_current_tab = obj.get("use_current_tab").cloned();
    let use_active_tab = obj.get("use_active_tab").cloned();

    let payload = obj
        .entry("payload".to_string())
        .or_insert_with(|| json!({}));
    if !payload.is_object() {
        *payload = json!({});
    }
    if let Some(payload_obj) = payload.as_object_mut() {
        payload_obj
            .entry("session_id".to_string())
            .or_insert_with(|| Value::String(session_id.to_string()));
        if let Some(current_tab_id) = current_tab_id {
            payload_obj
                .entry("current_tab_id".to_string())
                .or_insert_with(|| Value::from(current_tab_id));
        }
        if let Some(value) = use_current_tab.clone() {
            payload_obj
                .entry("use_current_tab".to_string())
                .or_insert(value);
        }
        if let Some(value) = use_active_tab.clone() {
            payload_obj
                .entry("use_active_tab".to_string())
                .or_insert(value);
        }
    }

    let forward_data = obj.entry("data".to_string()).or_insert_with(|| json!({}));
    if !forward_data.is_object() {
        *forward_data = json!({});
    }
    if let Some(forward_data_obj) = forward_data.as_object_mut() {
        forward_data_obj
            .entry("session_id".to_string())
            .or_insert_with(|| Value::String(session_id.to_string()));
        if let Some(current_tab_id) = current_tab_id {
            forward_data_obj
                .entry("current_tab_id".to_string())
                .or_insert_with(|| Value::from(current_tab_id));
        }
        if let Some(value) = use_current_tab {
            forward_data_obj
                .entry("use_current_tab".to_string())
                .or_insert(value);
        }
        if let Some(value) = use_active_tab {
            forward_data_obj
                .entry("use_active_tab".to_string())
                .or_insert(value);
        }
    }
}

async fn read_matching_jsonrpc_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    expected_id: &str,
) -> PlanResult<Value> {
    loop {
        let bytes = read_required_frame(reader).await.map_err(|e| {
            PlanError::BrokerError(format!("Failed to read supervisor frame: {}", e))
        })?;
        let value: Value = serde_json::from_slice(&bytes)?;
        if value.get("id").and_then(|v| v.as_str()) == Some(expected_id) {
            return Ok(value);
        }
        debug!(
            "Ignoring supervisor JSON-RPC frame for id {:?}; waiting for {}",
            value.get("id"),
            expected_id
        );
    }
}

fn normalize_supervisor_tool_response(structured: Value) -> Value {
    let mut normalized = structured
        .get("result")
        .cloned()
        .filter(|v| !matches!(v, Value::Null))
        .unwrap_or_else(|| structured.clone());

    if !normalized.is_object() {
        normalized = json!({ "result": normalized });
    }

    if let (Some(target), Some(source)) = (normalized.as_object_mut(), structured.as_object()) {
        for key in ["ok", "success", "error", "error_msg", "session_id"] {
            if !target.contains_key(key) {
                if let Some(value) = source.get(key) {
                    target.insert(key.to_string(), value.clone());
                }
            }
        }
    }

    normalized
}

async fn wait_for_supervisor_ready(client: &mut SupervisorEndpointClient) -> PlanResult<()> {
    let wait_ms = std::env::var("RZN_WAIT_NATIVE_HOST_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(5_000);
    let deadline = std::time::Instant::now() + Duration::from_millis(wait_ms);
    let mut last_health = json!({});

    loop {
        match client.call_tool("runtime.ensure_ready", json!({})).await {
            Ok(health) => {
                let ready = health
                    .get("ready")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if ready {
                    return Ok(());
                }
                last_health = health;
            }
            Err(err) => {
                last_health = json!({ "error": err.to_string() });
            }
        }

        if std::time::Instant::now() >= deadline {
            return Err(PlanError::BrokerError(format!(
                "Timed out waiting for native host bridge; last health={}",
                last_health
            )));
        }

        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn supervisor_response_success(value: &Value) -> bool {
    value
        .get("success")
        .and_then(|v| v.as_bool())
        .or_else(|| value.get("ok").and_then(|v| v.as_bool()))
        .or_else(|| value.pointer("/result/success").and_then(|v| v.as_bool()))
        .or_else(|| value.pointer("/result/ok").and_then(|v| v.as_bool()))
        .unwrap_or_else(|| value.get("error").is_none() && value.get("error_msg").is_none())
}

fn supervisor_endpoint_specs() -> Vec<SupervisorEndpointSpec> {
    let mut specs = Vec::new();

    if let (Some(socket), Some(token_path)) = (
        first_env_path(SUPERVISOR_SOCKET_ENV_KEYS),
        first_env_path(SUPERVISOR_TOKEN_ENV_KEYS),
    ) {
        specs.push(SupervisorEndpointSpec {
            socket: socket.to_string_lossy().to_string(),
            token_path: token_path.to_string_lossy().to_string(),
        });
    }

    for app_base in supervisor_app_base_candidates() {
        specs.push(supervisor_spec_for_app_base(&app_base));
    }

    let mut deduped = Vec::new();
    for spec in specs {
        if !deduped
            .iter()
            .any(|existing: &SupervisorEndpointSpec| existing.socket == spec.socket)
        {
            deduped.push(spec);
        }
    }

    deduped
        .into_iter()
        .filter(|spec| Path::new(&spec.socket).exists() && Path::new(&spec.token_path).exists())
        .collect()
}

fn supervisor_self_heal_enabled() -> bool {
    !std::env::var("RZN_DISABLE_SUPERVISOR_SELF_HEAL")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn supervisor_spec_for_app_base(app_base: &Path) -> SupervisorEndpointSpec {
    let (socket, token_path) = supervisor_paths_for_app_base(app_base);
    SupervisorEndpointSpec {
        socket: socket.to_string_lossy().to_string(),
        token_path: token_path.to_string_lossy().to_string(),
    }
}

fn supervisor_app_base_candidates() -> Vec<PathBuf> {
    candidate_app_bases()
}

fn default_supervisor_app_base() -> PathBuf {
    default_app_base_dir()
}

fn resolve_rzn_browser_command() -> Option<PathBuf> {
    if let Some(value) = env_trimmed("RZN_BROWSER_CMD") {
        let path = PathBuf::from(value);
        if path.exists() {
            return Some(path);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if exe
            .file_stem()
            .and_then(|value| value.to_str())
            .map(|value| value == "rzn-browser")
            .unwrap_or(false)
        {
            return Some(exe);
        }
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(if cfg!(windows) {
                "rzn-browser.exe"
            } else {
                "rzn-browser"
            });
            if sibling.exists() {
                return Some(sibling);
            }
        }
    }

    for candidate in [
        PathBuf::from("./target/debug/rzn-browser"),
        PathBuf::from("./target/release/rzn-browser"),
    ] {
        if candidate.exists() {
            return Some(candidate);
        }
    }

    supervisor_app_base_candidates()
        .into_iter()
        .map(|base| {
            base.join("bin").join(if cfg!(windows) {
                "rzn-browser.exe"
            } else {
                "rzn-browser"
            })
        })
        .find(|path| path.exists())
}

async fn spawn_supervisor() -> PlanResult<SupervisorEndpointSpec> {
    let browser = resolve_rzn_browser_command()
        .ok_or_else(|| PlanError::BrokerError("Could not find rzn-browser".to_string()))?;
    let app_base = first_env_path(APP_BASE_ENV_KEYS)
        .or_else(|| infer_app_base_from_executable(&browser))
        .unwrap_or_else(default_supervisor_app_base);
    let endpoint = supervisor_spec_for_app_base(&app_base);

    info!(
        "Spawning browser supervisor for planner transport: {} APP_BASE={}",
        browser.display(),
        app_base.display()
    );
    std::process::Command::new(&browser)
        .arg("supervisor")
        .arg("serve")
        .arg("--app-base")
        .arg(&app_base)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            PlanError::BrokerError(format!(
                "Spawn browser supervisor {}: {}",
                browser.display(),
                e
            ))
        })?;

    wait_for_spawned_supervisor(&endpoint).await?;
    Ok(endpoint)
}

async fn wait_for_spawned_supervisor(endpoint: &SupervisorEndpointSpec) -> PlanResult<()> {
    let deadline = std::time::Instant::now() + Duration::from_millis(5_000);
    loop {
        if Path::new(&endpoint.socket).exists() && Path::new(&endpoint.token_path).exists() {
            if let Ok(mut client) = SupervisorEndpointClient::connect(endpoint).await {
                if client.call_tool("runtime.status", json!({})).await.is_ok() {
                    return Ok(());
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(PlanError::BrokerError(format!(
                "Timed out waiting for spawned browser supervisor {}",
                endpoint.socket
            )));
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

fn env_trimmed(key: &str) -> Option<String> {
    shared_env_trimmed(key)
}

async fn connect_pipe_fs(name: &str) -> anyhow::Result<LocalSocketStream> {
    let socket_name = name
        .to_string()
        .to_fs_name::<GenericFilePath>()
        .map_err(|e| {
            anyhow::anyhow!(
                "Invalid filesystem socket path `{}`: {}",
                name.escape_debug(),
                e
            )
        })?;
    LocalSocketStream::connect(socket_name)
        .await
        .map_err(anyhow::Error::from)
}

async fn connect_pipe_ns(name: &str) -> anyhow::Result<LocalSocketStream> {
    let socket_name = name
        .to_string()
        .to_ns_name::<GenericNamespaced>()
        .map_err(|e| {
            anyhow::anyhow!(
                "Invalid namespaced socket name `{}`: {}",
                name.escape_debug(),
                e
            )
        })?;
    LocalSocketStream::connect(socket_name)
        .await
        .map_err(anyhow::Error::from)
}

/// Read response from broker
async fn read_response_from_broker<R: AsyncRead + Unpin>(
    reader: &mut R,
    expected_id: Option<&str>,
) -> PlanResult<Value> {
    loop {
        info!("Reading response from broker...");
        let buffer = read_required_frame(reader)
            .await
            .map_err(|e| PlanError::BrokerError(format!("Failed to read message frame: {}", e)))?;
        info!("Message length: {} bytes", buffer.len());

        // Convert bytes to string for potential decompression
        let raw_response = String::from_utf8(buffer).map_err(|e| {
            PlanError::BrokerError(format!("Failed to decode response as UTF-8: {}", e))
        })?;

        //  CRITICAL FIX: Decompress response if needed
        let decompressed_response = maybe_decompress_message(&raw_response)?;

        let response: Value = serde_json::from_str(&decompressed_response).map_err(|e| {
            PlanError::BrokerError(format!("Failed to parse broker response: {}", e))
        })?;

        // Some broker paths may forward unrelated out-of-band messages (e.g. extension heartbeat pings)
        // while a request is in-flight. Only accept responses that match our task_id/req_id.
        //
        // Compatibility note: older/buggy extension handlers may omit correlation IDs (req_id/task_id).
        // In that case, treat the next "response-like" message as the reply to the in-flight request,
        // otherwise we can hang forever while only receiving heartbeats.
        if let Some(expected) = expected_id {
            let task_id = response.get("task_id").and_then(|v| v.as_str());
            let req_id = response.get("req_id").and_then(|v| v.as_str());

            let looks_like_ping = response
                .get("action")
                .and_then(|v| v.as_str())
                .map(|a| a.eq_ignore_ascii_case("ping"))
                .unwrap_or(false);

            if task_id.is_none() && req_id.is_none() {
                if looks_like_ping {
                    continue;
                }
                let looks_like_response = response.get("success").is_some()
                    || response.get("result").is_some()
                    || response.get("error").is_some()
                    || response.get("error_msg").is_some()
                    || response.get("error_code").is_some();
                if !looks_like_response {
                    debug!(
                        "Ignoring broker message without ids that doesn't look like a response (expected={})",
                        expected
                    );
                    continue;
                }
                debug!(
                    "Broker response missing correlation ids; assuming it matches expected request {}",
                    expected
                );
            } else if task_id != Some(expected) && req_id != Some(expected) {
                debug!(
                    "Ignoring broker message not matching expected id (expected={}, task_id={:?}, req_id={:?})",
                    expected, task_id, req_id
                );
                continue;
            }
        }

        debug!(
            "Received response from broker: {}",
            serde_json::to_string_pretty(&response).unwrap_or_default()
        );

        // NOTE: Do not treat `success: false` as a transport error here.
        // The orchestrator consumes structured error responses (and may attempt healing).
        // Returning an Err would cause send_message() to mark the connection dead and retry,
        // which can spam duplicate requests and keep tabs perpetually "loading".

        // Return the result field if present, but preserve session info and other top-level fields
        if let Some(result) = response.get("result") {
            let mut result_with_session = result.clone();

            // If result isn't an object, return the full response to avoid losing metadata.
            if !result_with_session.is_object() {
                return Ok(response);
            }

            // Preserve success and error fields so orchestrator can make decisions.
            if let Some(success) = response.get("success") {
                if let Some(result_obj) = result_with_session.as_object_mut() {
                    result_obj.insert("success".to_string(), success.clone());
                }
            }
            if let Some(error_code) = response.get("error_code") {
                if let Some(result_obj) = result_with_session.as_object_mut() {
                    result_obj.insert("error_code".to_string(), error_code.clone());
                }
            }
            // Orchestrator historically looks for "error". Extension often sends "error_msg".
            let error_value = response
                .get("error")
                .cloned()
                .or_else(|| response.get("error_msg").cloned());
            if let Some(error_value) = error_value {
                if let Some(result_obj) = result_with_session.as_object_mut() {
                    result_obj.insert("error".to_string(), error_value);
                }
            }
            if let Some(task_id) = response.get("task_id") {
                if let Some(result_obj) = result_with_session.as_object_mut() {
                    result_obj.insert("task_id".to_string(), task_id.clone());
                }
            }
            if let Some(req_id) = response.get("req_id") {
                if let Some(result_obj) = result_with_session.as_object_mut() {
                    result_obj.insert("req_id".to_string(), req_id.clone());
                }
            }

            // Preserve session information from top-level response
            if let Some(tab_id) = response.get("current_tab_id") {
                if let Some(result_obj) = result_with_session.as_object_mut() {
                    result_obj.insert("current_tab_id".to_string(), tab_id.clone());
                }
            }

            if let Some(url) = response.get("current_url") {
                if let Some(result_obj) = result_with_session.as_object_mut() {
                    result_obj.insert("current_url".to_string(), url.clone());
                }
            }

            // IMPORTANT: Also preserve html_content and steps from top-level
            if let Some(html_content) = response.get("html_content") {
                if let Some(result_obj) = result_with_session.as_object_mut() {
                    result_obj.insert("html_content".to_string(), html_content.clone());
                }
            }

            if let Some(steps) = response.get("steps") {
                if let Some(result_obj) = result_with_session.as_object_mut() {
                    result_obj.insert("steps".to_string(), steps.clone());
                }
            }

            // Capabilities are useful to higher-level clients (SDK/desktop) even when `result` is returned.
            if let Some(caps) = response.get("capabilities") {
                if let Some(result_obj) = result_with_session.as_object_mut() {
                    result_obj.insert("capabilities".to_string(), caps.clone());
                }
            }

            return Ok(result_with_session);
        }

        // Best-effort normalize error field for downstream consumers.
        if response.get("error").is_none() {
            if let Some(err_msg) = response.get("error_msg").cloned() {
                if let Some(obj) = response.as_object() {
                    let mut new_obj = obj.clone();
                    new_obj.insert("error".to_string(), err_msg);
                    return Ok(serde_json::Value::Object(new_obj));
                }
            }
        }

        return Ok(response);
    }
}

///  CRITICAL FIX: Compress large payloads to prevent broker crashes
fn compress_payload(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    encoder.finish()
}

/// Decompress payload
fn decompress_payload(compressed_data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let mut decoder = GzDecoder::new(compressed_data);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;
    Ok(decompressed)
}

/// Check if a message should be compressed and compress if needed
fn maybe_compress_message(message: &str) -> Result<(String, bool), PlanError> {
    let message_bytes = message.as_bytes();
    let should_compress = message_bytes.len() > COMPRESSION_THRESHOLD;

    if should_compress {
        info!(
            " Compressing large message: {} bytes -> ",
            message_bytes.len()
        );

        // Compress the message
        let compressed = compress_payload(message_bytes)
            .map_err(|e| PlanError::BrokerError(format!("Compression failed: {}", e)))?;

        // Encode as base64
        let encoded = general_purpose::STANDARD.encode(&compressed);

        info!(
            " Compression complete: {} bytes ({}% reduction)",
            encoded.len(),
            ((message_bytes.len() - compressed.len()) * 100) / message_bytes.len()
        );

        // Wrap in compression envelope
        let compressed_message = json!({
            "compressed": true,
            "data": encoded
        });

        Ok((compressed_message.to_string(), true))
    } else {
        Ok((message.to_string(), false))
    }
}

/// Decompress a message if it's compressed
fn maybe_decompress_message(message: &str) -> Result<String, PlanError> {
    // Try to parse as JSON to check if it's compressed
    if let Ok(parsed) = serde_json::from_str::<Value>(message) {
        if let Some(compressed) = parsed.get("compressed") {
            if compressed.as_bool() == Some(true) {
                if let Some(data) = parsed.get("data").and_then(|d| d.as_str()) {
                    info!(" Decompressing received message");

                    // Decode base64
                    let compressed_bytes = general_purpose::STANDARD.decode(data).map_err(|e| {
                        PlanError::BrokerError(format!("Base64 decode failed: {}", e))
                    })?;

                    // Decompress
                    let decompressed = decompress_payload(&compressed_bytes).map_err(|e| {
                        PlanError::BrokerError(format!("Decompression failed: {}", e))
                    })?;

                    let decompressed_str = String::from_utf8(decompressed).map_err(|e| {
                        PlanError::BrokerError(format!("UTF-8 decode failed: {}", e))
                    })?;

                    info!(" Decompression complete: {} bytes", decompressed_str.len());

                    return Ok(decompressed_str);
                }
            }
        }
    }

    // Not compressed or failed to parse, return as-is
    Ok(message.to_string())
}

impl BrokerClient {
    /// Send a JSON message to the broker and get response
    pub async fn send_json_message(&mut self, mut message: Value) -> PlanResult<Value> {
        // Transform Option<T> fields before sending
        transform_options_for_extension(&mut message);

        // Ensure connection exists
        if !self.is_connected() {
            return Err(PlanError::BrokerError(
                "Not connected to broker".to_string(),
            ));
        }

        let socket = self.connection.as_mut().unwrap();

        // Serialize and send
        let message_str = serde_json::to_string(&message).map_err(PlanError::SerializationError)?;

        // Compress if needed
        let (final_message, _was_compressed) = maybe_compress_message(&message_str)?;

        let message_bytes = format!("{}\n", final_message).into_bytes();

        socket
            .write_all(&message_bytes)
            .await
            .map_err(|e| PlanError::BrokerError(format!("Failed to send: {}", e)))?;

        // Read response
        let mut buffer = vec![0u8; MAX_FRAME_SIZE];
        let mut total_read = 0;

        loop {
            match socket.read(&mut buffer[total_read..]).await {
                Ok(0) => {
                    if total_read == 0 {
                        return Err(PlanError::BrokerError(
                            "Connection closed by broker".to_string(),
                        ));
                    }
                    break;
                }
                Ok(n) => {
                    total_read += n;
                    if buffer[total_read - 1] == b'\n' {
                        break;
                    }
                    if total_read >= MAX_FRAME_SIZE {
                        return Err(PlanError::BrokerError("Response too large".to_string()));
                    }
                }
                Err(e) => return Err(PlanError::BrokerError(format!("Failed to read: {}", e))),
            }
        }

        let response_str = String::from_utf8_lossy(&buffer[..total_read]);
        let response_str = response_str.trim();

        // Decompress if needed
        let final_response = maybe_decompress_message(response_str)?;

        serde_json::from_str(&final_response)
            .map_err(|e| PlanError::BrokerError(format!("Invalid JSON response: {}", e)))
    }

    /// Get the active tab information
    pub async fn get_active_tab(&mut self) -> PlanResult<Value> {
        // Use the existing send_message format that broker understands
        let message = rzn_core::dsl::Message {
            action: "get_active_tab".to_string(),
            task_id: format!("tab_{}", Uuid::new_v4()),
            task: None,
            data: None,
        };

        debug!("Sending get_active_tab message");
        self.send_message(message).await
    }

    /// Send a message to a specific tab's content script
    pub async fn send_to_content_script(
        &mut self,
        tab_id: i32,
        message: Value,
    ) -> PlanResult<Value> {
        // Use the existing send_message format that broker understands
        let wrapped_message = rzn_core::dsl::Message {
            action: "send_to_tab".to_string(),
            task_id: format!("msg_{}", Uuid::new_v4()),
            task: None,
            data: Some(json!({
                "tab_id": tab_id,
                "message": message
            })),
        };

        self.send_message(wrapped_message).await
    }

    // New TargetSpec and CDP support methods

    /// Execute a step with TargetSpec targeting
    pub async fn execute_step_with_target(
        &mut self,
        step: &Step,
        target: &TargetSpec,
    ) -> PlanResult<ResultEnvelope> {
        if !self.is_connected() {
            self.connect().await?;
        }

        let task_id = format!(
            "target-{}",
            self.task_counter
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );

        // Create enhanced step with TargetSpec
        let mut enhanced_step = step.clone();
        self.apply_target_spec_to_step(&mut enhanced_step, target);

        let task = rzn_core::dsl::Task {
            steps: vec![enhanced_step],
            search_query: None,
        };

        let message = rzn_core::dsl::Message {
            action: ACTION_PERFORM_TASK.to_string(),
            task_id: task_id.clone(),
            task: Some(task),
            data: Some(json!({
                "session_id": self.session.session_id,
                "current_tab_id": self.session.current_tab_id,
                "use_target_spec": true,
                "target_spec": target
            })),
        };

        debug!("Executing step with TargetSpec: {:?}", target);
        let response = self.send_message(message).await?;

        self.update_session_from_response(&response);

        // Parse response into ResultEnvelope
        self.parse_result_envelope(&response)
    }

    /// Resolve TargetSpec to stable element reference
    pub async fn resolve_target(&mut self, target: &TargetSpec) -> PlanResult<ResolvedElement> {
        // Check cache first
        if let Some(encoded_id) = &target.encoded_id {
            if let Some(cached_element) = self.resolved_elements.get(encoded_id) {
                if cached_element.is_cache_valid(30000) {
                    // 30 second cache
                    debug!("Using cached resolved element: {}", encoded_id);
                    return Ok(cached_element.clone());
                }
            }
        }

        // Request element resolution from extension
        let message = rzn_core::dsl::Message {
            action: "resolve_element".to_string(),
            task_id: format!("resolve-{}", Uuid::new_v4()),
            task: None,
            data: Some(json!({
                "target_spec": target,
                "session_id": self.session.session_id,
                "current_tab_id": self.session.current_tab_id
            })),
        };

        debug!("Resolving target: {:?}", target);
        let response = self.send_message(message).await?;

        // Parse response into ResolvedElement
        let resolved = self.parse_resolved_element(&response, target)?;

        // Cache the resolved element
        self.resolved_elements
            .insert(resolved.encoded_id.clone(), resolved.clone());

        Ok(resolved)
    }

    /// Attach CDP for Pro mode capabilities
    pub async fn attach_cdp(&mut self) -> PlanResult<()> {
        if self.cdp_state == CdpState::Attached {
            debug!("CDP already attached");
            return Ok(());
        }

        self.cdp_state = CdpState::Attaching;
        debug!("Enabling debug (CDP) for Pro mode");
        let response = self.enable_debug("rescue", Some(120_000)).await?;

        if response
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            self.cdp_state = CdpState::Attached;
            info!("CDP attached successfully");
            Ok(())
        } else {
            self.cdp_state = CdpState::Detached;
            let error = response
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("Unknown error");
            Err(PlanError::BrokerError(format!(
                "CDP attachment failed: {}",
                error
            )))
        }
    }

    /// Detach CDP (return to Light mode)
    pub async fn detach_cdp(&mut self) -> PlanResult<()> {
        if self.cdp_state == CdpState::Detached {
            debug!("CDP already detached");
            return Ok(());
        }
        debug!("Disabling debug (CDP)");
        let _response = self.disable_debug().await?;

        self.cdp_state = CdpState::Detached;
        info!("CDP detached");
        Ok(())
    }

    /// Check if CDP is available for Pro mode operations
    pub fn is_pro_mode_available(&self) -> bool {
        self.cdp_state == CdpState::Attached
    }

    // Helper methods

    /// Apply TargetSpec to a step
    fn apply_target_spec_to_step(&self, step: &mut Step, target: &TargetSpec) {
        // Add target spec to step data (extension will handle it)
        match &mut step.kind {
            StepKind::ClickElement {
                selector, frame_id, ..
            } => {
                if let Some(css) = &target.css {
                    *selector = css.clone();
                }
                if let Some(frame_ordinal) = target.frame_ordinal {
                    *frame_id = Some(frame_ordinal.to_string());
                }
            }
            StepKind::FillInputField {
                selector, frame_id, ..
            } => {
                if let Some(css) = &target.css {
                    *selector = css.clone();
                }
                if let Some(frame_ordinal) = target.frame_ordinal {
                    *frame_id = Some(frame_ordinal.to_string());
                }
            }
            StepKind::WaitForElement {
                selector, frame_id, ..
            } => {
                if let Some(css) = &target.css {
                    *selector = css.clone();
                }
                if let Some(frame_ordinal) = target.frame_ordinal {
                    *frame_id = Some(frame_ordinal.to_string());
                }
            }
            _ => {
                // Other step types don't use selectors directly
                debug!(
                    "Step type doesn't support direct TargetSpec application: {:?}",
                    step.kind
                );
            }
        }
    }

    /// Parse broker response into ResultEnvelope
    fn parse_result_envelope(&self, response: &Value) -> PlanResult<ResultEnvelope> {
        let success = response
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let rung_used = response
            .get("rung_used")
            .and_then(|v| v.as_u64())
            .and_then(|r| InputRung::from_u8(r as u8))
            .unwrap_or(InputRung::Dom);
        let escalated = response
            .get("escalated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let execution_time_ms = response
            .get("execution_time_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let error = response
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Parse resolved element if present
        let resolved_element = if let Some(element_data) = response.get("resolved_element") {
            self.parse_resolved_element_from_value(element_data).ok()
        } else {
            None
        };

        if success {
            Ok(ResultEnvelope {
                result: response.clone(),
                rung_used,
                escalated,
                success: true,
                error: None,
                execution_time_ms,
                resolved_element,
            })
        } else {
            Ok(ResultEnvelope {
                result: response.clone(),
                rung_used,
                escalated,
                success: false,
                error,
                execution_time_ms,
                resolved_element,
            })
        }
    }

    /// Parse ResolvedElement from response
    fn parse_resolved_element(
        &self,
        response: &Value,
        original_target: &TargetSpec,
    ) -> PlanResult<ResolvedElement> {
        let element_data = response
            .get("resolved_element")
            .ok_or_else(|| PlanError::BrokerError("No resolved_element in response".to_string()))?;

        self.parse_resolved_element_from_value(element_data)
            .map(|mut element| {
                // Ensure original target spec is preserved
                element.target_spec = original_target.clone();
                element
            })
    }

    /// Parse ResolvedElement from JSON value
    fn parse_resolved_element_from_value(&self, value: &Value) -> PlanResult<ResolvedElement> {
        let encoded_id = value
            .get("encoded_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PlanError::BrokerError("Missing encoded_id".to_string()))?
            .to_string();

        let frame_ordinal = value
            .get("frame_ordinal")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| PlanError::BrokerError("Missing frame_ordinal".to_string()))?
            as u32;

        let backend_node_id = value
            .get("backend_node_id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| PlanError::BrokerError("Missing backend_node_id".to_string()))?;

        let bounds_data = value
            .get("bounds")
            .ok_or_else(|| PlanError::BrokerError("Missing bounds".to_string()))?;

        let bounds = ElementBounds::new(
            bounds_data.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0),
            bounds_data.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0),
            bounds_data
                .get("width")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            bounds_data
                .get("height")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
        );

        let is_cross_origin = value
            .get("is_cross_origin")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Create a placeholder target spec (will be overwritten by caller if needed)
        let target_spec = TargetSpec::from_encoded_id(encoded_id.clone());

        Ok(ResolvedElement {
            encoded_id,
            frame_ordinal,
            backend_node_id,
            bounds,
            is_cross_origin,
            target_spec,
            resolved_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        })
    }

    /// Clear resolved element cache
    pub fn clear_resolved_cache(&mut self) {
        self.resolved_elements.clear();
        debug!("Cleared resolved element cache");
    }
}
