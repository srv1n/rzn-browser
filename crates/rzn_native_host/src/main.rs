//! Native Messaging host that forwards browser extension messages to the local runtime.
//!
//! Target architecture: Chrome owns this process, so this binary stays a thin
//! extension-to-runtime bridge. The supervisor `rzn.local.v1` IPC path is
//! represented here, but until that endpoint exists we keep the legacy
//! worker-owned browser bridge as the compatibility adapter.
//!
//! Chrome/Edge native messaging framing:
//!   [4-byte little-endian length][UTF-8 JSON bytes]
//!
//! Usage:
//!   rzn-native-host [--socket <path>] [--token <path>]
//!
//! Env overrides:
//!   RZN_APP_BASE_DIR
//!   RZN_SUPERVISOR_APP_BASE, RZN_NATIVE_APP_BASE, RZN_APP_BASE, APP_BASE
//!   RZN_LOCAL_RUNTIME_SOCKET_PATH, RZN_LOCAL_RUNTIME_TOKEN_PATH
//!   RZN_SUPERVISOR_SOCKET_PATH, RZN_SUPERVISOR_TOKEN_PATH
//!   RZN_BROWSER_BRIDGE_SOCKET_PATH, RZN_BROWSER_BRIDGE_TOKEN_PATH

use anyhow::{anyhow, Context, Result};
use interprocess::local_socket::{
    tokio::Stream as LocalSocketStream, traits::tokio::Stream as _, GenericFilePath, ToFsName,
};
use serde_json::json;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::{timeout, Duration};
use tracing::{info, warn};
use uuid::Uuid;

mod cloud;

use rzn_broker_endpoint::{
    broker_endpoint_path, endpoint_pid_is_live, prune_stale_broker_endpoint, read_broker_endpoint,
};

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024; // 16 MiB
const EXTENSION_CALL_TIMEOUT_MS: u64 = 20000;
const LOCAL_RUNTIME_PROTOCOL: &str = "rzn.local.v1";
const LEGACY_BROWSER_BRIDGE_PROTOCOL: &str = "rzn-browser-bridge/1";
const LEGACY_BROWSER_SESSION_METHOD: &str = "browser.session";
const SUPERVISOR_EXTENSION_CALL_METHOD: &str = "native_host.extension_call";
const SUPERVISOR_SOCKET_FILENAME: &str = "rzn-supervisor.sock";
const SUPERVISOR_TOKEN_FILENAME: &str = "rzn-supervisor-token-v1";
const SUPERVISOR_SOCKET_ENV_KEYS: &[&str] = &[
    "RZN_LOCAL_RUNTIME_SOCKET_PATH",
    "RZN_SUPERVISOR_SOCKET_PATH",
];
const SUPERVISOR_TOKEN_ENV_KEYS: &[&str] =
    &["RZN_LOCAL_RUNTIME_TOKEN_PATH", "RZN_SUPERVISOR_TOKEN_PATH"];
const APP_BASE_ENV_KEYS: &[&str] = &[
    "RZN_APP_BASE_DIR",
    "RZN_SUPERVISOR_APP_BASE",
    "RZN_NATIVE_APP_BASE",
    "RZN_APP_BASE",
    "APP_BASE",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RuntimeBridgeKind {
    SupervisorLocalV1,
    LegacyBrowserBridgeV1,
}

impl RuntimeBridgeKind {
    fn label(self) -> &'static str {
        match self {
            Self::SupervisorLocalV1 => "supervisor_local_v1",
            Self::LegacyBrowserBridgeV1 => "legacy_browser_bridge_v1",
        }
    }

    fn protocol(self) -> &'static str {
        match self {
            Self::SupervisorLocalV1 => LOCAL_RUNTIME_PROTOCOL,
            Self::LegacyBrowserBridgeV1 => LEGACY_BROWSER_BRIDGE_PROTOCOL,
        }
    }
}

#[derive(Clone, Debug)]
struct UpstreamEndpoint {
    kind: RuntimeBridgeKind,
    socket_path: PathBuf,
    token_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct UpstreamKey {
    kind: RuntimeBridgeKind,
    socket_path: PathBuf,
    token_path: PathBuf,
}

impl From<&UpstreamEndpoint> for UpstreamKey {
    fn from(value: &UpstreamEndpoint) -> Self {
        Self {
            kind: value.kind,
            socket_path: value.socket_path.clone(),
            token_path: value.token_path.clone(),
        }
    }
}

fn parse_args() -> (Option<String>, Option<String>) {
    let mut socket = None;
    let mut token = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => socket = args.next(),
            "--token" => token = args.next(),
            _ => {}
        }
    }
    (socket, token)
}

fn env_trimmed(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn infer_app_base_from_exe() -> Option<PathBuf> {
    // When installed as a plugin bundle, the native host lives under:
    //   {APP_BASE}/plugins/<plugin_id>/<version or current>/bin/.../rzn-native-host
    //
    // The browser launches us without any env vars, so we infer APP_BASE from
    // our own executable path to avoid dev/release mismatches.
    let exe = std::env::current_exe().ok()?;
    for ancestor in exe.ancestors() {
        if ancestor.file_name().and_then(|s| s.to_str()) == Some("plugins") {
            return ancestor.parent().map(|p| p.to_path_buf());
        }
    }
    None
}

fn explicit_bridge_token_path(arg: Option<String>) -> Option<PathBuf> {
    if let Some(arg) = arg
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(PathBuf::from(arg));
    }
    if let Ok(v) = std::env::var("RZN_BROWSER_BRIDGE_TOKEN_PATH") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    None
}

fn explicit_bridge_socket_path(arg: Option<String>) -> Option<PathBuf> {
    if let Some(arg) = arg
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(PathBuf::from(arg));
    }
    if let Ok(v) = std::env::var("RZN_BROWSER_BRIDGE_SOCKET_PATH") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    None
}

fn explicit_env_path(keys: &[&str]) -> Option<PathBuf> {
    for key in keys {
        if let Some(value) = env_trimmed(key) {
            return Some(PathBuf::from(value));
        }
    }
    None
}

fn candidate_app_bases() -> Vec<PathBuf> {
    if let Some(base) = explicit_env_path(APP_BASE_ENV_KEYS) {
        return vec![base];
    }
    let Some(data) = dirs::data_dir() else {
        return vec![];
    };
    let mut bases = Vec::new();
    if let Some(base) = infer_app_base_from_exe() {
        bases.push(base);
    }
    // Prefer debug first (dev server), then release (bundled app).
    bases.push(data.join("rzn_debug"));
    bases.push(data.join("rzn"));
    // Standalone CLI native-run uses its own app base so it can coexist with the desktop app.
    bases.push(data.join("rzn-browser"));

    // Dedup while preserving preference order.
    let mut seen = std::collections::HashSet::new();
    bases.retain(|b| seen.insert(b.clone()));
    bases
}

#[derive(Clone, Debug)]
struct EndpointPaths {
    browser_bridge: Option<(PathBuf, PathBuf)>,
}

#[derive(Clone, Debug)]
struct BaseCandidate {
    base: PathBuf,
    paths: Option<EndpointPaths>,
    endpoint_mtime: Option<std::time::SystemTime>,
}

fn supervisor_paths_for_base(base: &Path) -> (PathBuf, PathBuf) {
    (
        base.join("run").join(SUPERVISOR_SOCKET_FILENAME),
        base.join("secure").join(SUPERVISOR_TOKEN_FILENAME),
    )
}

fn endpoint_paths_for_base(base: &Path) -> Option<EndpointPaths> {
    let endpoint = read_broker_endpoint(base)?;
    let mut paths = EndpointPaths {
        browser_bridge: None,
    };
    if let Some(bridge) = endpoint.browser_bridge {
        let socket_path = PathBuf::from(&bridge.socket);
        let token_path = PathBuf::from(&bridge.token_path);
        let pid_live = bridge.pid.map(endpoint_pid_is_live).unwrap_or(true);
        if socket_path.exists() && token_path.exists() && pid_live {
            paths.browser_bridge = Some((socket_path, token_path));
        }
    }
    if paths.browser_bridge.is_none() {
        None
    } else {
        Some(paths)
    }
}

fn discover_base_candidates() -> Vec<BaseCandidate> {
    let mut out = Vec::new();
    for base in candidate_app_bases() {
        let _ = prune_stale_broker_endpoint(&base);
        let endpoint_path = broker_endpoint_path(&base);
        let endpoint_mtime = std::fs::metadata(&endpoint_path)
            .ok()
            .and_then(|meta| meta.modified().ok());
        let paths = endpoint_paths_for_base(&base);
        out.push(BaseCandidate {
            base,
            paths,
            endpoint_mtime,
        });
    }

    // Prefer the freshest endpoint file first. This avoids stale dev/desktop app
    // state hijacking a newer `native-run` worker that just published its bridge.
    out.sort_by(|a, b| b.endpoint_mtime.cmp(&a.endpoint_mtime));
    out
}

fn candidate_endpoints_from_bases(
    bases: &[BaseCandidate],
    socket_override: Option<PathBuf>,
    token_override: Option<PathBuf>,
    supervisor_socket_override: Option<PathBuf>,
    supervisor_token_override: Option<PathBuf>,
) -> Vec<UpstreamEndpoint> {
    let mut bases = bases.to_vec();
    bases.sort_by(|a, b| b.endpoint_mtime.cmp(&a.endpoint_mtime));

    let mut out = Vec::new();
    let mut seen = HashSet::new();

    let mut push_unique = |endpoint: UpstreamEndpoint| {
        let key = (
            endpoint.kind,
            endpoint.socket_path.clone(),
            endpoint.token_path.clone(),
        );
        if seen.insert(key) {
            out.push(endpoint);
        }
    };

    if let (Some(sock), Some(tok)) = (
        supervisor_socket_override.as_ref(),
        supervisor_token_override.as_ref(),
    ) {
        push_unique(UpstreamEndpoint {
            kind: RuntimeBridgeKind::SupervisorLocalV1,
            socket_path: sock.clone(),
            token_path: tok.clone(),
        });
    }

    if let (Some(sock), Some(tok)) = (socket_override.as_ref(), token_override.as_ref()) {
        push_unique(UpstreamEndpoint {
            kind: RuntimeBridgeKind::LegacyBrowserBridgeV1,
            socket_path: sock.clone(),
            token_path: tok.clone(),
        });
    }

    for candidate in &bases {
        let (supervisor_socket, supervisor_token) = supervisor_paths_for_base(&candidate.base);
        if supervisor_socket.exists() && supervisor_token.exists() {
            push_unique(UpstreamEndpoint {
                kind: RuntimeBridgeKind::SupervisorLocalV1,
                socket_path: supervisor_socket,
                token_path: supervisor_token,
            });
        }

        if let Some(paths) = candidate.paths.as_ref() {
            if let Some((sock, tok)) = paths.browser_bridge.as_ref() {
                push_unique(UpstreamEndpoint {
                    kind: RuntimeBridgeKind::LegacyBrowserBridgeV1,
                    socket_path: socket_override.clone().unwrap_or_else(|| sock.clone()),
                    token_path: token_override.clone().unwrap_or_else(|| tok.clone()),
                });
            }
        }
    }

    out
}

fn candidate_endpoints(
    socket_arg: Option<String>,
    token_arg: Option<String>,
) -> Vec<UpstreamEndpoint> {
    let socket_override = explicit_bridge_socket_path(socket_arg);
    let token_override = explicit_bridge_token_path(token_arg);
    let supervisor_socket_override = explicit_env_path(SUPERVISOR_SOCKET_ENV_KEYS);
    let supervisor_token_override = explicit_env_path(SUPERVISOR_TOKEN_ENV_KEYS);

    let bases = discover_base_candidates();
    candidate_endpoints_from_bases(
        &bases,
        socket_override,
        token_override,
        supervisor_socket_override,
        supervisor_token_override,
    )
}

async fn connect_upstream_runtime(endpoint: &UpstreamEndpoint) -> Result<LocalSocketStream> {
    match endpoint.kind {
        RuntimeBridgeKind::SupervisorLocalV1 => {
            connect_supervisor_runtime(&endpoint.socket_path, &endpoint.token_path).await
        }
        RuntimeBridgeKind::LegacyBrowserBridgeV1 => {
            connect_legacy_browser_bridge(&endpoint.socket_path, &endpoint.token_path).await
        }
    }
}

async fn connect_legacy_browser_bridge(
    socket_path: &Path,
    token_path: &Path,
) -> Result<LocalSocketStream> {
    let token = tokio::fs::read_to_string(token_path)
        .await
        .with_context(|| format!("read bridge token {:?}", token_path))?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(anyhow!("Bridge token is empty at {:?}", token_path));
    }

    let name = socket_path
        .to_path_buf()
        .to_fs_name::<GenericFilePath>()
        .map_err(|e| anyhow!("Invalid socket path {:?}: {}", socket_path, e))?;

    let mut stream = LocalSocketStream::connect(name)
        .await
        .with_context(|| format!("connect bridge socket {:?}", socket_path))?;

    let handshake = json!({
        "type": "rzn_browser_bridge_handshake",
        "v": 1,
        "token": token,
        "client": {
            "name": "rzn-native-host",
            "kind": "native_host",
            "pid": std::process::id(),
            "version": env!("CARGO_PKG_VERSION")
        }
    });
    write_frame(&mut stream, serde_json::to_vec(&handshake)?.as_slice()).await?;
    let resp = read_frame(&mut stream).await?;
    let Some(resp) = resp else {
        return Err(anyhow!("Bridge closed during handshake"));
    };
    let v: serde_json::Value = serde_json::from_slice(&resp)?;
    if v.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(anyhow!("Bridge handshake failed: {}", v));
    }
    Ok(stream)
}

async fn connect_supervisor_runtime(
    socket_path: &Path,
    token_path: &Path,
) -> Result<LocalSocketStream> {
    let token = tokio::fs::read_to_string(token_path)
        .await
        .with_context(|| format!("read supervisor token {:?}", token_path))?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(anyhow!("Supervisor token is empty at {:?}", token_path));
    }

    let name = socket_path
        .to_path_buf()
        .to_fs_name::<GenericFilePath>()
        .map_err(|e| anyhow!("Invalid supervisor socket path {:?}: {}", socket_path, e))?;

    let mut stream = LocalSocketStream::connect(name)
        .await
        .with_context(|| format!("connect supervisor socket {:?}", socket_path))?;

    let request_id = format!("native-host-hello-{}", Uuid::new_v4());
    let handshake = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "runtime.hello",
        "params": {
            "version": LOCAL_RUNTIME_PROTOCOL,
            "token": token,
            "role": "native_host_bridge",
            "client": native_host_client_metadata(RuntimeBridgeKind::SupervisorLocalV1),
            "capabilities": {
                "chrome_native_messaging": true,
                "extension_rpc": true,
                "accepts_extension_call_method": SUPERVISOR_EXTENSION_CALL_METHOD,
                "legacy_cloud_actor_owner": false,
                "cloud_actor_owner": "supervisor"
            },
            "cloud": cloud::native_host_cloud_bridge_status()
        }
    });
    write_frame(&mut stream, serde_json::to_vec(&handshake)?.as_slice()).await?;
    let resp = read_frame(&mut stream).await?;
    let Some(resp) = resp else {
        return Err(anyhow!("Supervisor closed during runtime.hello"));
    };
    let v: serde_json::Value = serde_json::from_slice(&resp)?;
    let ok = v.get("ok").and_then(|v| v.as_bool()) == Some(true)
        || v.pointer("/result/ok").and_then(|v| v.as_bool()) == Some(true);
    if !ok {
        return Err(anyhow!("Supervisor runtime.hello failed: {}", v));
    }
    Ok(stream)
}

async fn connect_supervisor_client(
    socket_path: &Path,
    token_path: &Path,
) -> Result<LocalSocketStream> {
    let token = tokio::fs::read_to_string(token_path)
        .await
        .with_context(|| format!("read supervisor token {:?}", token_path))?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(anyhow!("Supervisor token is empty at {:?}", token_path));
    }

    let name = socket_path
        .to_path_buf()
        .to_fs_name::<GenericFilePath>()
        .map_err(|e| anyhow!("Invalid supervisor socket path {:?}: {}", socket_path, e))?;

    let mut stream = LocalSocketStream::connect(name)
        .await
        .with_context(|| format!("connect supervisor client socket {:?}", socket_path))?;

    let handshake = json!({
        "type": "rzn_local_handshake",
        "v": 1,
        "token": token,
        "client": native_host_client_metadata(RuntimeBridgeKind::SupervisorLocalV1)
    });
    write_frame(&mut stream, serde_json::to_vec(&handshake)?.as_slice()).await?;
    let resp = read_frame(&mut stream).await?;
    let Some(resp) = resp else {
        return Err(anyhow!("Supervisor closed during client handshake"));
    };
    let v: Value = serde_json::from_slice(&resp)?;
    if v.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(anyhow!("Supervisor client handshake failed: {}", v));
    }
    Ok(stream)
}

async fn call_supervisor_client(
    socket_arg: Option<String>,
    token_arg: Option<String>,
    method: &str,
    params: Value,
) -> Result<Value> {
    let endpoints = candidate_endpoints(socket_arg, token_arg);
    let mut last_error: Option<anyhow::Error> = None;
    for endpoint in endpoints
        .into_iter()
        .filter(|endpoint| endpoint.kind == RuntimeBridgeKind::SupervisorLocalV1)
    {
        match connect_supervisor_client(&endpoint.socket_path, &endpoint.token_path).await {
            Ok(mut stream) => {
                let id = format!("native-host-control-{}", Uuid::new_v4());
                let request = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": method,
                    "params": params
                });
                write_frame(&mut stream, serde_json::to_vec(&request)?.as_slice()).await?;
                let response = read_frame(&mut stream).await?;
                let Some(response) = response else {
                    return Err(anyhow!("Supervisor closed during {}", method));
                };
                let value: Value = serde_json::from_slice(&response)?;
                if let Some(error) = value.get("error") {
                    return Err(anyhow!("Supervisor {} failed: {}", method, error));
                }
                return Ok(value.get("result").cloned().unwrap_or(value));
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("No supervisor endpoint available")))
}

fn native_host_client_metadata(kind: RuntimeBridgeKind) -> Value {
    json!({
        "name": "rzn-native-host",
        "kind": "native_host",
        "pid": std::process::id(),
        "version": env!("CARGO_PKG_VERSION"),
        "bridge_kind": kind.label(),
        "protocol": kind.protocol()
    })
}

async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    if let Err(e) = reader.read_exact(&mut len_buf).await {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            return Ok(None);
        }
        return Err(e).context("read length");
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(anyhow!("Frame too large: {} bytes", len));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await.context("read frame")?;
    Ok(Some(buf))
}

async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, payload: &[u8]) -> Result<()> {
    let len = payload.len();
    if len > MAX_FRAME_BYTES {
        return Err(anyhow!("Frame too large: {} bytes", len));
    }
    let len_buf = (len as u32).to_le_bytes();
    writer.write_all(&len_buf).await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

async fn read_native_message<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    if let Err(e) = reader.read_exact(&mut len_buf).await {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            return Ok(None);
        }
        return Err(e).context("read native length");
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(anyhow!("Native message too large: {} bytes", len));
    }
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .context("read native payload")?;
    Ok(Some(buf))
}

async fn write_native_message<W: AsyncWrite + Unpin>(writer: &mut W, payload: &[u8]) -> Result<()> {
    let len = payload.len();
    if len > MAX_FRAME_BYTES {
        return Err(anyhow!("Native message too large: {} bytes", len));
    }
    let len_buf = (len as u32).to_le_bytes();
    writer.write_all(&len_buf).await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

fn id_key(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn extension_response_id(value: &Value) -> Option<String> {
    value
        .get("req_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            value
                .get("task_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .or_else(|| value.get("id").and_then(id_key))
}

fn is_cmd_envelope(val: &Value) -> bool {
    val.get("cmd").is_some()
}

fn cmd_field(val: &Value, key: &str) -> Option<String> {
    val.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn jsonrpc_error(id: impl Into<String>, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.into(),
        "error": { "code": code, "message": message.into() }
    })
}

#[derive(Clone, Debug)]
struct ExtensionCallRequest {
    upstream_request_id: String,
    cmd: String,
    payload: Value,
    data: Option<Value>,
    original_req_id: String,
    timeout_ms: u64,
}

fn parse_extension_call_request(value: &Value) -> Option<Result<ExtensionCallRequest, Value>> {
    let method = value.get("method").and_then(|v| v.as_str())?;
    if method != LEGACY_BROWSER_SESSION_METHOD && method != SUPERVISOR_EXTENSION_CALL_METHOD {
        return None;
    }

    let upstream_request_id = value
        .get("id")
        .and_then(id_key)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let params = value.get("params").cloned().unwrap_or_else(|| json!({}));

    let cmd = params
        .get("cmd")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if cmd.is_empty() {
        return Some(Err(jsonrpc_error(
            upstream_request_id,
            -32602,
            "cmd is required",
        )));
    }

    let payload = params.get("payload").cloned().unwrap_or_else(|| json!({}));
    let data = params.get("data").cloned();
    let original_req_id = params
        .get("req_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| upstream_request_id.clone());
    let timeout_ms = params
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .or_else(|| params.get("timeoutMs").and_then(|v| v.as_u64()))
        .unwrap_or(EXTENSION_CALL_TIMEOUT_MS);

    Some(Ok(ExtensionCallRequest {
        upstream_request_id,
        cmd,
        payload,
        data,
        original_req_id,
        timeout_ms,
    }))
}

fn rewrite_response_correlation(value: &mut Value, wire_req_id: &str, original_req_id: &str) {
    if let Some(req_id) = value.get_mut("req_id") {
        if req_id.as_str() == Some(wire_req_id) {
            *req_id = Value::String(original_req_id.to_string());
        }
    }
    if let Some(task_id) = value.get_mut("task_id") {
        if task_id.as_str() == Some(wire_req_id) {
            *task_id = Value::String(original_req_id.to_string());
        }
    }
}

fn cloud_supervisor_method(cmd: &str) -> Option<&'static str> {
    match cmd {
        "cloud_get_status" => Some("cloud.status"),
        "cloud_set_config" => Some("cloud.set_config"),
        "cloud_clear_config" => Some("cloud.clear_config"),
        _ => None,
    }
}

fn build_native_control_response(
    request: &Value,
    cmd: &str,
    success: bool,
    result: Value,
    error: Option<String>,
) -> Value {
    let req_id = request
        .get("req_id")
        .and_then(|value| value.as_str())
        .unwrap_or("native-host-control");
    let mut response = json!({
        "cmd": format!("{}_response", cmd),
        "req_id": req_id,
        "success": success,
        "result": result
    });
    if let Some(error) = error {
        response["error"] = Value::String(error.clone());
        response["error_msg"] = Value::String(error);
    }
    response
}

async fn forward_supervisor_cloud_control_command(
    request: &Value,
    socket_arg: Option<String>,
    token_arg: Option<String>,
) -> Option<Result<Value>> {
    let cmd = request.get("cmd").and_then(|value| value.as_str())?;
    let method = cloud_supervisor_method(cmd)?;
    let params = request.get("payload").cloned().unwrap_or_else(|| json!({}));
    Some(
        call_supervisor_client(socket_arg, token_arg, method, params)
            .await
            .map(|result| build_native_control_response(request, cmd, true, result, None)),
    )
}

fn legacy_native_host_cloud_enabled_from_value(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

fn legacy_native_host_cloud_enabled() -> bool {
    legacy_native_host_cloud_enabled_from_value(
        std::env::var("RZN_NATIVE_HOST_LEGACY_CLOUD")
            .ok()
            .as_deref(),
    )
}

fn handle_native_control_command(value: &Value) -> Option<Value> {
    let cmd = cmd_field(value, "cmd")?;
    let req_id = value
        .get("req_id")
        .and_then(|v| v.as_str())
        .unwrap_or("native-host-control");

    match cmd.as_str() {
        "ping" => {
            let source = value
                .pointer("/payload/source")
                .and_then(|v| v.as_str())
                .unwrap_or("native_host_control");

            Some(json!({
                "cmd": "ping_response",
                "req_id": req_id,
                "success": true,
                "result": {
                    "pong": true,
                    "source": source,
                    "native_host_pid": std::process::id()
                }
            }))
        }
        "runtime_bridge_get_status" => Some(json!({
            "cmd": "runtime_bridge_get_status_response",
            "req_id": req_id,
            "success": true,
            "result": {
                "role": "extension_to_runtime_bridge",
                "native_host_pid": std::process::id(),
                "chrome_native_messaging": true,
                "supervisor": {
                    "protocol": LOCAL_RUNTIME_PROTOCOL,
                    "extension_call_method": SUPERVISOR_EXTENSION_CALL_METHOD,
                    "socket_env": SUPERVISOR_SOCKET_ENV_KEYS,
                    "token_env": SUPERVISOR_TOKEN_ENV_KEYS,
                    "available": true,
                    "status": "preferred_when_socket_and_token_exist"
                },
                "legacy_browser_bridge": {
                    "protocol": LEGACY_BROWSER_BRIDGE_PROTOCOL,
                    "method": LEGACY_BROWSER_SESSION_METHOD,
                    "available": true,
                    "status": "compatibility_adapter"
                },
                "cloud": cloud::native_host_cloud_bridge_status()
            }
        })),
        _ => None,
    }
}

async fn run_upstream_connection(
    endpoint: UpstreamEndpoint,
    stream: LocalSocketStream,
    native_tx: mpsc::UnboundedSender<Vec<u8>>,
    native_pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    active_bridges: Arc<Mutex<HashSet<UpstreamKey>>>,
) {
    let key = UpstreamKey::from(&endpoint);
    let (mut upstream_reader, upstream_writer) = stream.split();
    let (upstream_tx, mut upstream_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    active_bridges.lock().await.insert(key.clone());

    let writer_task = tokio::spawn(async move {
        let mut writer = upstream_writer;
        while let Some(bytes) = upstream_rx.recv().await {
            if write_frame(&mut writer, &bytes).await.is_err() {
                break;
            }
        }
    });

    loop {
        let frame = match read_frame(&mut upstream_reader).await {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(e) => {
                warn!(
                    "Runtime bridge read error kind={}: {}",
                    endpoint.kind.label(),
                    e
                );
                break;
            }
        };
        if frame.is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_slice(&frame) {
            Ok(v) => v,
            Err(e) => {
                warn!("Upstream message parse error: {}", e);
                continue;
            }
        };

        let Some(extension_call) = parse_extension_call_request(&msg) else {
            continue;
        };
        let extension_call = match extension_call {
            Ok(extension_call) => extension_call,
            Err(resp) => {
                if let Ok(bytes) = serde_json::to_vec(&resp) {
                    let _ = upstream_tx.send(bytes);
                }
                continue;
            }
        };
        let wire_req_id = format!("native-host-{}", Uuid::new_v4());

        let (tx, rx) = oneshot::channel::<Value>();
        {
            let mut guard = native_pending.lock().await;
            guard.insert(wire_req_id.clone(), tx);
        }

        let native_tx_session = native_tx.clone();
        let upstream_tx_session = upstream_tx.clone();
        let pending_session = native_pending.clone();
        tokio::spawn(async move {
            let ExtensionCallRequest {
                upstream_request_id,
                cmd,
                payload,
                data,
                original_req_id,
                timeout_ms,
            } = extension_call;
            let mut out = json!({
                "cmd": cmd,
                "req_id": wire_req_id,
                "payload": payload
            });
            if let Some(data) = data {
                out["data"] = data;
            }
            let bytes = match serde_json::to_vec(&out) {
                Ok(bytes) => bytes,
                Err(e) => {
                    let resp = jsonrpc_error(
                        upstream_request_id,
                        -32700,
                        format!("serialize error: {}", e),
                    );
                    if let Ok(bytes) = serde_json::to_vec(&resp) {
                        let _ = upstream_tx_session.send(bytes);
                    }
                    return;
                }
            };

            if native_tx_session.send(bytes).is_err() {
                let mut guard = pending_session.lock().await;
                guard.remove(&wire_req_id);
                let resp = jsonrpc_error(upstream_request_id, -32000, "extension disconnected");
                if let Ok(bytes) = serde_json::to_vec(&resp) {
                    let _ = upstream_tx_session.send(bytes);
                }
                return;
            }

            let response = match timeout(Duration::from_millis(timeout_ms), rx).await {
                Ok(Ok(mut v)) => {
                    rewrite_response_correlation(&mut v, &wire_req_id, &original_req_id);
                    v
                }
                Ok(Err(_)) => {
                    let mut guard = pending_session.lock().await;
                    guard.remove(&wire_req_id);
                    let resp = jsonrpc_error(
                        upstream_request_id,
                        -32002,
                        "extension response channel closed",
                    );
                    if let Ok(bytes) = serde_json::to_vec(&resp) {
                        let _ = upstream_tx_session.send(bytes);
                    }
                    return;
                }
                Err(_) => {
                    let mut guard = pending_session.lock().await;
                    guard.remove(&wire_req_id);
                    let resp = jsonrpc_error(
                        upstream_request_id,
                        -32003,
                        format!("Extension timeout after {}ms", timeout_ms),
                    );
                    if let Ok(bytes) = serde_json::to_vec(&resp) {
                        let _ = upstream_tx_session.send(bytes);
                    }
                    return;
                }
            };

            let resp = json!({
                "jsonrpc": "2.0",
                "id": upstream_request_id,
                "result": response
            });
            if let Ok(bytes) = serde_json::to_vec(&resp) {
                let _ = upstream_tx_session.send(bytes);
            }
        });
    }

    drop(upstream_tx);
    let _ = writer_task.await;
    active_bridges.lock().await.remove(&key);
}

async fn endpoint_manager_loop(
    socket_arg: Option<String>,
    token_arg: Option<String>,
    native_tx: mpsc::UnboundedSender<Vec<u8>>,
    native_pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    active_bridges: Arc<Mutex<HashSet<UpstreamKey>>>,
) {
    loop {
        let endpoints = candidate_endpoints(socket_arg.clone(), token_arg.clone());

        for endpoint in &endpoints {
            if !endpoint.token_path.exists() {
                continue;
            }

            let key = UpstreamKey::from(endpoint);
            let already_connected = active_bridges.lock().await.contains(&key);
            if already_connected {
                continue;
            }

            match connect_upstream_runtime(endpoint).await {
                Ok(stream) => {
                    info!(
                        "Connected to runtime bridge kind={} socket={:?} token_path={:?}",
                        endpoint.kind.label(),
                        endpoint.socket_path,
                        endpoint.token_path
                    );
                    active_bridges.lock().await.insert(key);
                    tokio::spawn(run_upstream_connection(
                        endpoint.clone(),
                        stream,
                        native_tx.clone(),
                        native_pending.clone(),
                        active_bridges.clone(),
                    ));
                }
                Err(e) => {
                    warn!(
                        "Failed to connect runtime bridge kind={} socket={:?} token_path={:?}: {}",
                        endpoint.kind.label(),
                        endpoint.socket_path,
                        endpoint.token_path,
                        e
                    );
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge_paths(label: &str) -> EndpointPaths {
        EndpointPaths {
            browser_bridge: Some((
                PathBuf::from(format!("/tmp/{label}-bridge.sock")),
                PathBuf::from(format!("/tmp/{label}-bridge.token")),
            )),
        }
    }

    fn base_candidate(label: &str, paths: Option<EndpointPaths>, mtime_secs: u64) -> BaseCandidate {
        BaseCandidate {
            base: PathBuf::from(format!("/tmp/rzn-native-host-test-{label}")),
            paths,
            endpoint_mtime: Some(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime_secs),
            ),
        }
    }

    #[test]
    fn candidate_endpoints_prefers_freshest_bridge() {
        let newer = base_candidate("debug", Some(bridge_paths("debug")), 20);
        let older = base_candidate("cli", Some(bridge_paths("cli")), 10);

        let endpoints = candidate_endpoints_from_bases(&[newer, older], None, None, None, None);

        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].kind, RuntimeBridgeKind::LegacyBrowserBridgeV1);
        assert_eq!(
            endpoints[0].socket_path,
            PathBuf::from("/tmp/debug-bridge.sock")
        );
        assert_eq!(
            endpoints[1].socket_path,
            PathBuf::from("/tmp/cli-bridge.sock")
        );
    }

    #[test]
    fn candidate_endpoints_accepts_explicit_override() {
        let base = base_candidate("runtime", Some(bridge_paths("runtime")), 5);

        let endpoints = candidate_endpoints_from_bases(
            &[base],
            Some(PathBuf::from("/tmp/explicit.sock")),
            Some(PathBuf::from("/tmp/explicit.token")),
            None,
            None,
        );

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].kind, RuntimeBridgeKind::LegacyBrowserBridgeV1);
        assert_eq!(
            endpoints[0].socket_path,
            PathBuf::from("/tmp/explicit.sock")
        );
        assert_eq!(
            endpoints[0].token_path,
            PathBuf::from("/tmp/explicit.token")
        );
    }

    #[test]
    fn candidate_endpoints_prefers_supervisor_override() {
        let base = base_candidate("legacy", Some(bridge_paths("legacy")), 5);

        let endpoints = candidate_endpoints_from_bases(
            &[base],
            None,
            None,
            Some(PathBuf::from("/tmp/supervisor.sock")),
            Some(PathBuf::from("/tmp/supervisor.token")),
        );

        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].kind, RuntimeBridgeKind::SupervisorLocalV1);
        assert_eq!(
            endpoints[0].socket_path,
            PathBuf::from("/tmp/supervisor.sock")
        );
        assert_eq!(endpoints[1].kind, RuntimeBridgeKind::LegacyBrowserBridgeV1);
    }

    #[test]
    fn candidate_endpoints_discovers_supervisor_from_app_base() {
        let base_dir =
            std::env::temp_dir().join(format!("rzn-native-host-supervisor-{}", Uuid::new_v4()));
        let (socket_path, token_path) = supervisor_paths_for_base(&base_dir);
        std::fs::create_dir_all(socket_path.parent().expect("socket parent")).unwrap();
        std::fs::create_dir_all(token_path.parent().expect("token parent")).unwrap();
        std::fs::write(&socket_path, "").unwrap();
        std::fs::write(&token_path, "token\n").unwrap();

        let base = BaseCandidate {
            base: base_dir.clone(),
            paths: Some(bridge_paths("legacy-for-supervisor-base")),
            endpoint_mtime: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(5)),
        };

        let endpoints = candidate_endpoints_from_bases(&[base], None, None, None, None);

        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].kind, RuntimeBridgeKind::SupervisorLocalV1);
        assert_eq!(endpoints[0].socket_path, socket_path);
        assert_eq!(endpoints[0].token_path, token_path);
        assert_eq!(endpoints[1].kind, RuntimeBridgeKind::LegacyBrowserBridgeV1);

        let _ = std::fs::remove_dir_all(base_dir);
    }

    #[test]
    fn parses_legacy_browser_session_extension_call() {
        let request = parse_extension_call_request(&json!({
            "jsonrpc": "2.0",
            "id": "upstream-1",
            "method": "browser.session",
            "params": {
                "cmd": "execute_step",
                "req_id": "extension-1",
                "timeout_ms": 1234,
                "payload": { "session_id": "session-1" }
            }
        }))
        .expect("browser.session is a bridge method")
        .expect("request should be valid");

        assert_eq!(request.upstream_request_id, "upstream-1");
        assert_eq!(request.cmd, "execute_step");
        assert_eq!(request.original_req_id, "extension-1");
        assert_eq!(request.timeout_ms, 1234);
        assert_eq!(
            request.payload.get("session_id").and_then(|v| v.as_str()),
            Some("session-1")
        );
        assert!(request.data.is_none());
    }

    #[test]
    fn parses_supervisor_extension_call_method() {
        let request = parse_extension_call_request(&json!({
            "jsonrpc": "2.0",
            "id": "runtime-1",
            "method": "native_host.extension_call",
            "params": {
                "cmd": "get_dom_snapshot",
                "payload": { "session_id": "session-1" },
                "data": { "include_dom_snapshot": true }
            }
        }))
        .expect("native_host.extension_call is a bridge method")
        .expect("request should be valid");

        assert_eq!(request.upstream_request_id, "runtime-1");
        assert_eq!(request.cmd, "get_dom_snapshot");
        assert_eq!(request.original_req_id, "runtime-1");
        assert_eq!(
            request
                .data
                .as_ref()
                .and_then(|v| v.get("include_dom_snapshot"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn supervisor_extension_call_requires_cmd() {
        let error = parse_extension_call_request(&json!({
            "jsonrpc": "2.0",
            "id": "runtime-1",
            "method": "native_host.extension_call",
            "params": {}
        }))
        .expect("native_host.extension_call is a bridge method")
        .expect_err("missing cmd should be rejected");

        assert_eq!(
            error.pointer("/error/code").and_then(|v| v.as_i64()),
            Some(-32602)
        );
        assert_eq!(error.get("id").and_then(|v| v.as_str()), Some("runtime-1"));
    }

    #[test]
    fn native_control_ping_returns_ping_response() {
        let response = handle_native_control_command(&json!({
            "cmd": "ping",
            "req_id": "heartbeat-1",
            "payload": { "source": "extension_keepalive" }
        }))
        .expect("ping should be handled locally");

        assert_eq!(
            response.get("cmd").and_then(|v| v.as_str()),
            Some("ping_response")
        );
        assert_eq!(
            response.get("req_id").and_then(|v| v.as_str()),
            Some("heartbeat-1")
        );
        assert_eq!(
            response.get("success").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            response.pointer("/result/source").and_then(|v| v.as_str()),
            Some("extension_keepalive")
        );
    }

    #[test]
    fn native_control_unknown_command_is_not_handled() {
        assert!(handle_native_control_command(&json!({
            "cmd": "unknown",
            "req_id": "x"
        }))
        .is_none());
    }

    #[test]
    fn native_control_runtime_bridge_status_describes_supervisor_contract() {
        let response = handle_native_control_command(&json!({
            "cmd": "runtime_bridge_get_status",
            "req_id": "status-1"
        }))
        .expect("runtime bridge status should be handled locally");

        assert_eq!(
            response.get("cmd").and_then(|v| v.as_str()),
            Some("runtime_bridge_get_status_response")
        );
        assert_eq!(
            response.pointer("/result/role").and_then(|v| v.as_str()),
            Some("extension_to_runtime_bridge")
        );
        assert_eq!(
            response
                .pointer("/result/supervisor/status")
                .and_then(|v| v.as_str()),
            Some("preferred_when_socket_and_token_exist")
        );
        assert_eq!(
            response
                .pointer("/result/legacy_browser_bridge/status")
                .and_then(|v| v.as_str()),
            Some("compatibility_adapter")
        );
        assert_eq!(
            response
                .pointer("/result/cloud/runtime_owner")
                .and_then(|v| v.as_str()),
            Some("supervisor")
        );
        assert_eq!(
            response
                .pointer("/result/cloud/native_host_dispatch")
                .and_then(|v| v.as_str()),
            Some("disabled")
        );
    }

    #[test]
    fn app_base_env_keys_match_supervisor_aliases() {
        for key in [
            "RZN_APP_BASE_DIR",
            "RZN_SUPERVISOR_APP_BASE",
            "RZN_NATIVE_APP_BASE",
            "RZN_APP_BASE",
            "APP_BASE",
        ] {
            assert!(APP_BASE_ENV_KEYS.contains(&key));
        }
    }

    #[test]
    fn legacy_native_host_cloud_requires_explicit_truthy_flag() {
        assert!(!legacy_native_host_cloud_enabled_from_value(None));
        assert!(!legacy_native_host_cloud_enabled_from_value(Some("")));
        assert!(!legacy_native_host_cloud_enabled_from_value(Some("0")));
        assert!(!legacy_native_host_cloud_enabled_from_value(Some("false")));
        assert!(legacy_native_host_cloud_enabled_from_value(Some("1")));
        assert!(legacy_native_host_cloud_enabled_from_value(Some("true")));
        assert!(legacy_native_host_cloud_enabled_from_value(Some("YES")));
        assert!(legacy_native_host_cloud_enabled_from_value(Some(" on ")));
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let (socket_arg, token_arg) = parse_args();

    let (native_tx, mut native_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let native_pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let active_bridges: Arc<Mutex<HashSet<UpstreamKey>>> = Arc::new(Mutex::new(HashSet::new()));

    let native_writer_task = tokio::spawn(async move {
        let mut stdout = io::BufWriter::new(io::stdout());
        while let Some(bytes) = native_rx.recv().await {
            if write_native_message(&mut stdout, &bytes).await.is_err() {
                break;
            }
        }
    });

    let endpoint_manager = tokio::spawn(endpoint_manager_loop(
        socket_arg.clone(),
        token_arg.clone(),
        native_tx.clone(),
        native_pending.clone(),
        active_bridges.clone(),
    ));

    let native_pending_native = native_pending.clone();
    let native_tx_native = native_tx.clone();
    let socket_arg_for_control = socket_arg.clone();
    let token_arg_for_control = token_arg.clone();
    let native_reader_task = tokio::spawn(async move {
        let mut stdin = io::stdin();
        loop {
            let msg = match read_native_message(&mut stdin).await {
                Ok(Some(msg)) => msg,
                Ok(None) => break,
                Err(e) => {
                    warn!("Native read error: {}", e);
                    break;
                }
            };
            if msg.is_empty() {
                continue;
            }
            let v: Value = match serde_json::from_slice(&msg) {
                Ok(v) => v,
                Err(e) => {
                    warn!("Native message parse error: {}", e);
                    continue;
                }
            };

            if let Some(response) = forward_supervisor_cloud_control_command(
                &v,
                socket_arg_for_control.clone(),
                token_arg_for_control.clone(),
            )
            .await
            {
                match response {
                    Ok(response) => {
                        if let Ok(bytes) = serde_json::to_vec(&response) {
                            let _ = native_tx_native.send(bytes);
                        }
                        continue;
                    }
                    Err(error) if !legacy_native_host_cloud_enabled() => {
                        let cmd = cmd_field(&v, "cmd").unwrap_or_else(|| "cloud".to_string());
                        let response = build_native_control_response(
                            &v,
                            &cmd,
                            false,
                            json!({}),
                            Some(error.to_string()),
                        );
                        if let Ok(bytes) = serde_json::to_vec(&response) {
                            let _ = native_tx_native.send(bytes);
                        }
                        continue;
                    }
                    Err(error) => {
                        warn!(
                            "Supervisor cloud control failed; using legacy native-host cloud fallback: {}",
                            error
                        );
                    }
                }
            }

            if legacy_native_host_cloud_enabled() {
                if let Some(response) = cloud::handle_local_control_command(&v).await {
                    if let Ok(bytes) = serde_json::to_vec(&response) {
                        let _ = native_tx_native.send(bytes);
                    }
                    continue;
                }
            } else if cloud_supervisor_method(&cmd_field(&v, "cmd").unwrap_or_default()).is_some() {
                continue;
            }

            // Extension session response handling
            if v.get("success").is_some() {
                if let Some(req_id) = extension_response_id(&v) {
                    let tx = {
                        let mut guard = native_pending_native.lock().await;
                        guard.remove(&req_id)
                    };
                    if let Some(tx) = tx {
                        let _ = tx.send(v.clone());
                        continue;
                    }
                }
            }
            if is_cmd_envelope(&v) {
                if let Some(response) = handle_native_control_command(&v) {
                    if let Ok(bytes) = serde_json::to_vec(&response) {
                        let _ = native_tx_native.send(bytes);
                    }
                    continue;
                }
                warn!(
                    "Ignoring unsupported native host control cmd: {}",
                    cmd_field(&v, "cmd").unwrap_or_else(|| "<missing>".to_string())
                );
                continue;
            }
            warn!("Ignoring unexpected native host message from extension");
        }
    });

    info!("rzn-native-host running");

    let _ = native_reader_task.await;
    endpoint_manager.abort();
    native_writer_task.abort();
    Ok(())
}
