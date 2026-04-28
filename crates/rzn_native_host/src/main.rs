//! Native Messaging host that forwards browser extension messages to the worker-owned
//! browser bridge.
//!
//! Chrome/Edge native messaging framing:
//!   [4-byte little-endian length][UTF-8 JSON bytes]
//!
//! Usage:
//!   rzn-native-host [--socket <path>] [--token <path>]
//!
//! Env overrides:
//!   RZN_APP_BASE_DIR, RZN_BROWSER_BRIDGE_SOCKET_PATH, RZN_BROWSER_BRIDGE_TOKEN_PATH

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

use rzn_broker_endpoint::{broker_endpoint_path, read_broker_endpoint};

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024; // 16 MiB
const EXTENSION_CALL_TIMEOUT_MS: u64 = 20000;

#[derive(Clone, Debug)]
struct UpstreamEndpoint {
    socket_path: PathBuf,
    token_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct UpstreamKey {
    socket_path: PathBuf,
    token_path: PathBuf,
}

impl From<&UpstreamEndpoint> for UpstreamKey {
    fn from(value: &UpstreamEndpoint) -> Self {
        Self {
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

fn candidate_app_bases() -> Vec<PathBuf> {
    if let Some(base) = env_trimmed("RZN_APP_BASE_DIR") {
        return vec![PathBuf::from(base)];
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
    paths: Option<EndpointPaths>,
    endpoint_mtime: Option<std::time::SystemTime>,
}

fn endpoint_paths_for_base(base: &Path) -> Option<EndpointPaths> {
    let endpoint = read_broker_endpoint(base)?;
    let mut paths = EndpointPaths {
        browser_bridge: None,
    };
    if let Some(bridge) = endpoint.browser_bridge {
        paths.browser_bridge = Some((
            PathBuf::from(bridge.socket),
            PathBuf::from(bridge.token_path),
        ));
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
        let endpoint_path = broker_endpoint_path(&base);
        let endpoint_mtime = std::fs::metadata(&endpoint_path)
            .ok()
            .and_then(|meta| meta.modified().ok());
        out.push(BaseCandidate {
            paths: endpoint_paths_for_base(&base),
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
) -> Vec<UpstreamEndpoint> {
    let mut bases = bases.to_vec();
    bases.sort_by(|a, b| b.endpoint_mtime.cmp(&a.endpoint_mtime));

    let mut out = Vec::new();
    let mut seen = HashSet::new();

    let mut push_unique = |endpoint: UpstreamEndpoint| {
        let key = (endpoint.socket_path.clone(), endpoint.token_path.clone());
        if seen.insert(key) {
            out.push(endpoint);
        }
    };

    if let (Some(sock), Some(tok)) = (socket_override.as_ref(), token_override.as_ref()) {
        push_unique(UpstreamEndpoint {
            socket_path: sock.clone(),
            token_path: tok.clone(),
        });
    }

    for candidate in &bases {
        if let Some(paths) = candidate.paths.as_ref() {
            if let Some((sock, tok)) = paths.browser_bridge.as_ref() {
                push_unique(UpstreamEndpoint {
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

    let bases = discover_base_candidates();
    candidate_endpoints_from_bases(&bases, socket_override, token_override)
}

async fn connect_browser_bridge(
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

fn handle_native_control_command(value: &Value) -> Option<Value> {
    let cmd = cmd_field(value, "cmd")?;
    if cmd != "ping" {
        return None;
    }

    let req_id = value
        .get("req_id")
        .and_then(|v| v.as_str())
        .unwrap_or("ping");
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
                warn!("Browser bridge read error: {}", e);
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

        if msg.get("method").and_then(|v| v.as_str()) != Some("browser.session") {
            continue;
        }

        let upstream_request_id = msg
            .get("id")
            .and_then(id_key)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));

        let cmd = params
            .get("cmd")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if cmd.is_empty() {
            let resp = json!({
                "jsonrpc": "2.0",
                "id": upstream_request_id,
                "error": { "code": -32602, "message": "cmd is required" }
            });
            if let Ok(bytes) = serde_json::to_vec(&resp) {
                let _ = upstream_tx.send(bytes);
            }
            continue;
        }

        let payload = params.get("payload").cloned().unwrap_or_else(|| json!({}));
        let original_req_id = params
            .get("req_id")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| upstream_request_id.clone());
        let wire_req_id = format!("native-host-{}", Uuid::new_v4());
        let timeout_ms = params
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .or_else(|| params.get("timeoutMs").and_then(|v| v.as_u64()))
            .unwrap_or(EXTENSION_CALL_TIMEOUT_MS);

        let (tx, rx) = oneshot::channel::<Value>();
        {
            let mut guard = native_pending.lock().await;
            guard.insert(wire_req_id.clone(), tx);
        }

        let native_tx_session = native_tx.clone();
        let upstream_tx_session = upstream_tx.clone();
        let pending_session = native_pending.clone();
        tokio::spawn(async move {
            let out = json!({
                "cmd": cmd,
                "req_id": wire_req_id,
                "payload": payload
            });
            let bytes = match serde_json::to_vec(&out) {
                Ok(bytes) => bytes,
                Err(e) => {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": upstream_request_id,
                        "error": { "code": -32700, "message": format!("serialize error: {}", e) }
                    });
                    if let Ok(bytes) = serde_json::to_vec(&resp) {
                        let _ = upstream_tx_session.send(bytes);
                    }
                    return;
                }
            };

            if native_tx_session.send(bytes).is_err() {
                let mut guard = pending_session.lock().await;
                guard.remove(&wire_req_id);
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": upstream_request_id,
                    "error": { "code": -32000, "message": "extension disconnected" }
                });
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
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": upstream_request_id,
                        "error": { "code": -32002, "message": "extension response channel closed" }
                    });
                    if let Ok(bytes) = serde_json::to_vec(&resp) {
                        let _ = upstream_tx_session.send(bytes);
                    }
                    return;
                }
                Err(_) => {
                    let mut guard = pending_session.lock().await;
                    guard.remove(&wire_req_id);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": upstream_request_id,
                        "error": { "code": -32003, "message": format!("Extension timeout after {}ms", timeout_ms) }
                    });
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

            match connect_browser_bridge(&endpoint.socket_path, &endpoint.token_path).await {
                Ok(stream) => {
                    info!(
                        "Connected to browser bridge socket={:?} token_path={:?}",
                        endpoint.socket_path, endpoint.token_path
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
                        "Failed to connect browser bridge socket={:?} token_path={:?}: {}",
                        endpoint.socket_path, endpoint.token_path, e
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

    #[test]
    fn candidate_endpoints_prefers_freshest_bridge() {
        let newer = BaseCandidate {
            paths: Some(bridge_paths("debug")),
            endpoint_mtime: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(20)),
        };
        let older = BaseCandidate {
            paths: Some(bridge_paths("cli")),
            endpoint_mtime: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(10)),
        };

        let endpoints = candidate_endpoints_from_bases(&[newer, older], None, None);

        assert_eq!(endpoints.len(), 2);
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
        let base = BaseCandidate {
            paths: Some(bridge_paths("runtime")),
            endpoint_mtime: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(5)),
        };

        let endpoints = candidate_endpoints_from_bases(
            &[base],
            Some(PathBuf::from("/tmp/explicit.sock")),
            Some(PathBuf::from("/tmp/explicit.token")),
        );

        assert_eq!(endpoints.len(), 1);
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

    cloud::maybe_spawn_cloud_actor(native_tx.clone(), native_pending.clone());

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

            if let Some(response) = cloud::handle_local_control_command(&v).await {
                if let Ok(bytes) = serde_json::to_vec(&response) {
                    let _ = native_tx_native.send(bytes);
                }
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
