//! Native Messaging host that forwards browser extension messages to the local runtime.
//!
//! Chrome owns this process, so this binary stays a thin extension-to-supervisor
//! bridge over the supervisor `rzn.local.v1` IPC path.
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

use anyhow::{anyhow, Context, Result};
use interprocess::local_socket::{
    tokio::Stream as LocalSocketStream, traits::tokio::Stream as _, GenericFilePath, ToFsName,
};
use rzn_core::framing::{read_frame, write_frame, MAX_FRAME_SIZE};
use rzn_core::runtime_paths::{
    candidate_app_bases as shared_candidate_app_bases, first_env_path,
    infer_app_base_from_executable, supervisor_paths_for_app_base, APP_BASE_ENV_KEYS,
    SUPERVISOR_SOCKET_ENV_KEYS, SUPERVISOR_TOKEN_ENV_KEYS,
};
use rzn_core::secure_files::{cleanup_secure_artifacts, secure_dir, write_secret_file};
use serde_json::json;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::{timeout, Duration};
use tracing::{info, warn};
use uuid::Uuid;

mod cloud;

const CHROME_TO_NATIVE_HOST_MAX_BYTES: usize = 64 * 1024 * 1024; // Chrome protocol limit.
const NATIVE_HOST_TO_CHROME_MAX_BYTES: usize = 1024 * 1024; // Chrome protocol limit.
const EXTENSION_CALL_TIMEOUT_MS: u64 = 20000;
const LOCAL_RUNTIME_PROTOCOL: &str = "rzn.local.v1";
const SUPERVISOR_EXTENSION_CALL_METHOD: &str = "native_host.extension_call";
const SUPERVISOR_SHUTDOWN_METHOD: &str = "native_host.shutdown";
const STDOUT_HEARTBEAT_INTERVAL_MS: u64 = 20_000;
const STDOUT_HEARTBEAT_CMD: &str = "native_host_heartbeat";
const EXTENSION_TIMEOUT_SHUTDOWN_GRACE_MS: u64 = 1_000;
const NATIVE_READER_EXIT_UPSTREAM_FLUSH_GRACE_MS: u64 = 1_000;
const OVERSIZE_ARTIFACT_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const OVERSIZE_ARTIFACT_MAX_FILES: usize = 50;
static NATIVE_HOST_BOOT_ID: OnceLock<String> = OnceLock::new();

fn native_host_boot_id() -> &'static str {
    NATIVE_HOST_BOOT_ID
        .get_or_init(|| format!("native-host-{}", Uuid::new_v4()))
        .as_str()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RuntimeBridgeKind {
    SupervisorLocalV1,
}

impl RuntimeBridgeKind {
    fn label(self) -> &'static str {
        match self {
            Self::SupervisorLocalV1 => "supervisor_local_v1",
        }
    }

    fn protocol(self) -> &'static str {
        match self {
            Self::SupervisorLocalV1 => LOCAL_RUNTIME_PROTOCOL,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct NativeHostLaunchContext {
    socket: Option<String>,
    token: Option<String>,
    caller_origin: Option<String>,
    parent_window: Option<String>,
    extra_args: Vec<String>,
}

impl NativeHostLaunchContext {
    fn from_env_args() -> Self {
        parse_launch_context(std::env::args().skip(1))
    }
}

fn parse_launch_context<I, S>(args: I) -> NativeHostLaunchContext
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut context = NativeHostLaunchContext::default();
    let mut args = args.into_iter().map(Into::into);
    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--socket=") {
            context.socket = Some(value.to_string());
            continue;
        }
        if arg == "--socket" {
            match args.next() {
                Some(value) => context.socket = Some(value),
                None => context.extra_args.push(arg),
            }
            continue;
        }

        if let Some(value) = arg.strip_prefix("--token=") {
            context.token = Some(value.to_string());
            continue;
        }
        if arg == "--token" {
            match args.next() {
                Some(value) => context.token = Some(value),
                None => context.extra_args.push(arg),
            }
            continue;
        }

        if let Some(value) = arg.strip_prefix("--parent-window=") {
            context.parent_window = Some(value.to_string());
            continue;
        }
        if arg == "--parent-window" {
            match args.next() {
                Some(value) => context.parent_window = Some(value),
                None => context.extra_args.push(arg),
            }
            continue;
        }

        if is_valid_caller_origin(&arg) {
            if context.caller_origin.is_none() {
                context.caller_origin = Some(arg);
            } else {
                context.extra_args.push(arg);
            }
            continue;
        }

        context.extra_args.push(arg);
    }
    context
}

fn is_valid_caller_origin(value: &str) -> bool {
    let Some(extension_id) = value.strip_prefix("chrome-extension://") else {
        return false;
    };
    let Some(extension_id) = extension_id.strip_suffix('/') else {
        return false;
    };
    extension_id.len() == 32 && extension_id.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

fn explicit_supervisor_token_path(arg: Option<String>) -> Option<PathBuf> {
    if let Some(arg) = arg
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(PathBuf::from(arg));
    }
    explicit_env_path(SUPERVISOR_TOKEN_ENV_KEYS)
}

fn explicit_supervisor_socket_path(arg: Option<String>) -> Option<PathBuf> {
    if let Some(arg) = arg
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(PathBuf::from(arg));
    }
    explicit_env_path(SUPERVISOR_SOCKET_ENV_KEYS)
}

fn explicit_env_path(keys: &[&str]) -> Option<PathBuf> {
    first_env_path(keys)
}

fn candidate_app_bases() -> Vec<PathBuf> {
    if let Some(base) = explicit_env_path(APP_BASE_ENV_KEYS) {
        return vec![base];
    }

    let mut bases = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(base) = infer_app_base_from_executable(&exe) {
            bases.push(base);
        }
    }
    for base in shared_candidate_app_bases() {
        if !bases.iter().any(|existing| existing == &base) {
            bases.push(base);
        }
    }
    bases
}

#[derive(Clone, Debug)]
struct BaseCandidate {
    base: PathBuf,
}

fn supervisor_paths_for_base(base: &Path) -> (PathBuf, PathBuf) {
    supervisor_paths_for_app_base(base)
}

fn discover_base_candidates() -> Vec<BaseCandidate> {
    let mut out = Vec::new();
    for base in candidate_app_bases() {
        out.push(BaseCandidate { base });
    }
    out
}

fn candidate_endpoints_from_bases(
    bases: &[BaseCandidate],
    supervisor_socket_override: Option<PathBuf>,
    supervisor_token_override: Option<PathBuf>,
) -> Vec<UpstreamEndpoint> {
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

    for candidate in bases {
        let (supervisor_socket, supervisor_token) = supervisor_paths_for_base(&candidate.base);
        if supervisor_socket.exists() && supervisor_token.exists() {
            push_unique(UpstreamEndpoint {
                kind: RuntimeBridgeKind::SupervisorLocalV1,
                socket_path: supervisor_socket,
                token_path: supervisor_token,
            });
        }
    }

    out
}

fn candidate_endpoints(
    socket_arg: Option<String>,
    token_arg: Option<String>,
) -> Vec<UpstreamEndpoint> {
    let supervisor_socket_override = explicit_supervisor_socket_path(socket_arg);
    let supervisor_token_override = explicit_supervisor_token_path(token_arg);

    let bases = discover_base_candidates();
    candidate_endpoints_from_bases(
        &bases,
        supervisor_socket_override,
        supervisor_token_override,
    )
}

async fn connect_upstream_runtime(
    endpoint: &UpstreamEndpoint,
    launch_context: &NativeHostLaunchContext,
) -> Result<LocalSocketStream> {
    match endpoint.kind {
        RuntimeBridgeKind::SupervisorLocalV1 => {
            connect_supervisor_runtime(&endpoint.socket_path, &endpoint.token_path, launch_context)
                .await
        }
    }
}

async fn connect_supervisor_runtime(
    socket_path: &Path,
    token_path: &Path,
    launch_context: &NativeHostLaunchContext,
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
    let mut params = json!({
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
    });
    merge_native_host_launch_metadata(&mut params, launch_context);

    let handshake = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "runtime.hello",
        "params": params
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

fn merge_native_host_launch_metadata(params: &mut Value, launch_context: &NativeHostLaunchContext) {
    let Some(params) = params.as_object_mut() else {
        return;
    };
    params.insert(
        "native_host_pid".to_string(),
        Value::Number(std::process::id().into()),
    );
    params.insert(
        "native_host_boot_id".to_string(),
        Value::String(native_host_boot_id().to_string()),
    );
    if let Some(caller_origin) = launch_context.caller_origin.as_ref() {
        params.insert(
            "caller_origin".to_string(),
            Value::String(caller_origin.clone()),
        );
    }
    params.insert(
        "launch".to_string(),
        json!({
            "parent_window": launch_context.parent_window,
            "has_socket_override": launch_context.socket.as_ref().is_some_and(|value| !value.trim().is_empty()),
            "has_token_override": launch_context.token.as_ref().is_some_and(|value| !value.trim().is_empty()),
            "extra_arg_count": launch_context.extra_args.len(),
        }),
    );
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
        "boot_id": native_host_boot_id(),
        "version": env!("CARGO_PKG_VERSION"),
        "bridge_kind": kind.label(),
        "protocol": kind.protocol()
    })
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
    if len > CHROME_TO_NATIVE_HOST_MAX_BYTES {
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
    if len > NATIVE_HOST_TO_CHROME_MAX_BYTES {
        return Err(anyhow!(
            "Native host-to-Chrome message too large: {} bytes",
            len
        ));
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

fn extension_protocol_error_response(value: &Value, req_id: &str, message: &str) -> Value {
    let mut response = json!({
        "req_id": req_id,
        "success": false,
        "error_code": "EXTENSION_PROTOCOL_ERROR",
        "error": message,
        "error_msg": message,
        "raw_response": value,
    });
    if let Some(task_id) = value.get("task_id").cloned() {
        response["task_id"] = task_id;
    }
    if let Some(lease_id) = value.get("lease_id").cloned() {
        response["lease_id"] = lease_id;
    }
    response
}

fn pending_extension_response(value: &Value) -> Option<(String, Value)> {
    let req_id = extension_response_id(value)?;
    if value.get("success").and_then(Value::as_bool).is_some() {
        Some((req_id, value.clone()))
    } else {
        Some((
            req_id.clone(),
            extension_protocol_error_response(
                value,
                &req_id,
                "extension response matched pending request but did not include boolean success",
            ),
        ))
    }
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

fn jsonrpc_result(id: impl Into<String>, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.into(),
        "result": result
    })
}

fn native_host_stdout_heartbeat(seq: u64) -> Value {
    json!({
        "cmd": STDOUT_HEARTBEAT_CMD,
        "payload": {
            "source": "rzn-native-host",
            "native_host_pid": std::process::id(),
            "native_host_boot_id": native_host_boot_id(),
            "interval_ms": STDOUT_HEARTBEAT_INTERVAL_MS,
            "seq": seq
        }
    })
}

fn extension_timeout_shutdown_reason(cmd: &str, timeout_ms: u64) -> String {
    format!(
        "extension call '{}' timed out after {}ms; restarting native-host/native-port epoch",
        cmd, timeout_ms
    )
}

async fn write_oversize_response_artifact(
    cmd: &str,
    original_req_id: &str,
    response: &Value,
    encoded_len: usize,
) -> Result<Value> {
    let dir = secure_dir("native-host-artifacts")?;
    let _ = cleanup_secure_artifacts(&dir, OVERSIZE_ARTIFACT_MAX_AGE, OVERSIZE_ARTIFACT_MAX_FILES);
    let artifact_id = Uuid::new_v4().to_string();
    let path = dir.join(format!("{}.json", artifact_id));
    let bytes = serde_json::to_vec(response)?;
    write_secret_file(&path, &bytes)?;
    Ok(json!({
        "success": true,
        "result": {
            "type": "native_host_oversize_response_artifact",
            "artifact_id": artifact_id,
            "path": path.to_string_lossy(),
            "content_type": "application/json",
            "bytes": bytes.len(),
            "encoded_response_bytes": encoded_len,
            "cmd": cmd,
            "req_id": original_req_id
        },
        "rzn_artifact_ref": {
            "artifact_id": artifact_id,
            "path": path.to_string_lossy(),
            "content_type": "application/json",
            "bytes": bytes.len()
        },
        "warning": "extension response exceeded supervisor local frame cap and was written to an artifact"
    }))
}

fn native_stdout_write_failed_shutdown_reason(error: &dyn std::fmt::Display) -> String {
    format!(
        "native-host stdout write failed: {}; restarting native-host/native-port epoch",
        error
    )
}

async fn drain_native_pending_with_disconnect_error(
    native_pending: &Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    reason: &str,
) -> usize {
    let pending = {
        let mut guard = native_pending.lock().await;
        std::mem::take(&mut *guard)
    };
    let count = pending.len();
    for (req_id, tx) in pending {
        let _ = tx.send(json!({
            "cmd": "native_host_disconnected",
            "req_id": req_id,
            "success": false,
            "error_code": "NATIVE_HOST_DISCONNECTED",
            "error": "native_host_disconnected",
            "error_msg": reason,
            "result": {
                "reason": reason,
                "native_host_pid": std::process::id()
            }
        }));
    }
    count
}

fn parse_shutdown_request(value: &Value) -> Option<(String, String)> {
    let method = value.get("method").and_then(|v| v.as_str())?;
    if method != SUPERVISOR_SHUTDOWN_METHOD {
        return None;
    }

    let id = value
        .get("id")
        .and_then(id_key)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let reason = value
        .pointer("/params/reason")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("supervisor requested native-host restart")
        .to_string();
    Some((id, reason))
}

#[derive(Clone, Debug)]
struct ExtensionCallRequest {
    upstream_request_id: String,
    cmd: String,
    payload: Value,
    data: Option<Value>,
    original_req_id: String,
    timeout_ms: u64,
    supervisor_boot_id: Option<String>,
    supervisor_bridge_id: Option<String>,
    supervisor_bridge_epoch: Option<u64>,
}

fn parse_extension_call_request(value: &Value) -> Option<Result<ExtensionCallRequest, Value>> {
    let method = value.get("method").and_then(|v| v.as_str())?;
    if method != SUPERVISOR_EXTENSION_CALL_METHOD {
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
    let supervisor_boot_id = params
        .get("supervisor_boot_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let supervisor_bridge_id = params
        .get("supervisor_bridge_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let supervisor_bridge_epoch = params
        .get("supervisor_bridge_epoch")
        .and_then(Value::as_u64);

    Some(Ok(ExtensionCallRequest {
        upstream_request_id,
        cmd,
        payload,
        data,
        original_req_id,
        timeout_ms,
        supervisor_boot_id,
        supervisor_bridge_id,
        supervisor_bridge_epoch,
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
    let payload = request.get("payload").cloned().unwrap_or_else(|| json!({}));
    let method = if cmd == "supervisor_rpc" {
        payload.get("method").and_then(Value::as_str)?.to_string()
    } else {
        cloud_supervisor_method(cmd)?.to_string()
    };
    let params = if cmd == "supervisor_rpc" {
        payload.get("params").cloned().unwrap_or_else(|| json!({}))
    } else {
        payload
    };
    let (method, params) = if cmd == "supervisor_rpc" {
        (
            "native_host.rpc".to_string(),
            json!({"method": method, "params": params}),
        )
    } else {
        (method, params)
    };
    Some(
        call_supervisor_client(socket_arg, token_arg, &method, params)
            .await
            .map(|result| build_native_control_response(request, cmd, true, result, None)),
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
                    "native_host_pid": std::process::id(),
                    "native_host_boot_id": native_host_boot_id()
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
                "native_host_boot_id": native_host_boot_id(),
                "chrome_native_messaging": true,
                "supervisor": {
                    "protocol": LOCAL_RUNTIME_PROTOCOL,
                    "extension_call_method": SUPERVISOR_EXTENSION_CALL_METHOD,
                    "socket_env": SUPERVISOR_SOCKET_ENV_KEYS,
                    "token_env": SUPERVISOR_TOKEN_ENV_KEYS,
                    "available": true,
                    "status": "preferred_when_socket_and_token_exist"
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
    shutdown_tx: mpsc::UnboundedSender<String>,
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

    let mut shutdown_reason: Option<String> = None;
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

        if let Some((id, reason)) = parse_shutdown_request(&msg) {
            let response = jsonrpc_result(
                id,
                json!({
                    "ok": true,
                    "shutdown": true,
                    "reason": reason,
                    "native_host_pid": std::process::id()
                }),
            );
            if let Ok(bytes) = serde_json::to_vec(&response) {
                let _ = upstream_tx.send(bytes);
            }
            shutdown_reason = Some(reason);
            break;
        }

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
        let shutdown_tx_session = shutdown_tx.clone();
        tokio::spawn(async move {
            let ExtensionCallRequest {
                upstream_request_id,
                cmd,
                payload,
                data,
                original_req_id,
                timeout_ms,
                supervisor_boot_id,
                supervisor_bridge_id,
                supervisor_bridge_epoch,
            } = extension_call;
            let mut out = json!({
                "cmd": cmd,
                "req_id": wire_req_id,
                "payload": payload,
                "timeout_ms": timeout_ms,
                "rzn_bridge": {
                    "supervisor_boot_id": supervisor_boot_id,
                    "supervisor_bridge_id": supervisor_bridge_id,
                    "supervisor_bridge_epoch": supervisor_bridge_epoch,
                    "native_host_boot_id": native_host_boot_id(),
                    "native_host_pid": std::process::id()
                }
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
            if bytes.len() > NATIVE_HOST_TO_CHROME_MAX_BYTES {
                let mut guard = pending_session.lock().await;
                guard.remove(&wire_req_id);
                let resp = jsonrpc_error(
                    upstream_request_id,
                    -32004,
                    format!(
                        "Extension request exceeds Chrome native messaging host-to-browser cap: {} bytes",
                        bytes.len()
                    ),
                );
                if let Ok(bytes) = serde_json::to_vec(&resp) {
                    let _ = upstream_tx_session.send(bytes);
                }
                return;
            }

            if native_tx_session.send(bytes).is_err() {
                let mut guard = pending_session.lock().await;
                guard.remove(&wire_req_id);
                let resp = jsonrpc_error(upstream_request_id, -32000, "extension disconnected");
                if let Ok(bytes) = serde_json::to_vec(&resp) {
                    let _ = upstream_tx_session.send(bytes);
                }
                let _ = shutdown_tx_session.send(
                    "native-host stdout channel closed before extension call; restarting native-host/native-port epoch"
                        .to_string(),
                );
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
                    let reason = "extension response channel closed; restarting native-host/native-port epoch"
                        .to_string();
                    tokio::time::sleep(Duration::from_millis(EXTENSION_TIMEOUT_SHUTDOWN_GRACE_MS))
                        .await;
                    let _ = shutdown_tx_session.send(reason);
                    return;
                }
                Err(_) => {
                    let mut guard = pending_session.lock().await;
                    guard.remove(&wire_req_id);
                    let reason = extension_timeout_shutdown_reason(&cmd, timeout_ms);
                    let resp = jsonrpc_error(
                        upstream_request_id,
                        -32003,
                        format!("Extension timeout after {}ms", timeout_ms),
                    );
                    if let Ok(bytes) = serde_json::to_vec(&resp) {
                        let _ = upstream_tx_session.send(bytes);
                    }
                    tokio::time::sleep(Duration::from_millis(EXTENSION_TIMEOUT_SHUTDOWN_GRACE_MS))
                        .await;
                    let _ = shutdown_tx_session.send(reason);
                    return;
                }
            };

            let resp = json!({
                "jsonrpc": "2.0",
                "id": upstream_request_id,
                "result": response
            });
            match serde_json::to_vec(&resp) {
                Ok(bytes) if bytes.len() <= MAX_FRAME_SIZE => {
                    let _ = upstream_tx_session.send(bytes);
                }
                Ok(bytes) => {
                    let artifact = write_oversize_response_artifact(
                        &cmd,
                        &original_req_id,
                        &response,
                        bytes.len(),
                    )
                    .await;
                    let fallback = match artifact {
                        Ok(artifact) => jsonrpc_result(upstream_request_id, artifact),
                        Err(error) => jsonrpc_error(
                            upstream_request_id,
                            -32005,
                            format!("Extension response exceeded local frame cap and artifact write failed: {}", error),
                        ),
                    };
                    if let Ok(bytes) = serde_json::to_vec(&fallback) {
                        let _ = upstream_tx_session.send(bytes);
                    }
                }
                Err(error) => {
                    let resp = jsonrpc_error(
                        upstream_request_id,
                        -32700,
                        format!("serialize response error: {}", error),
                    );
                    if let Ok(bytes) = serde_json::to_vec(&resp) {
                        let _ = upstream_tx_session.send(bytes);
                    }
                }
            }
        });
    }

    drop(upstream_tx);
    let _ = writer_task.await;
    active_bridges.lock().await.remove(&key);
    if let Some(reason) = shutdown_reason {
        let _ = shutdown_tx.send(reason);
    }
}

async fn endpoint_manager_loop(
    socket_arg: Option<String>,
    token_arg: Option<String>,
    launch_context: NativeHostLaunchContext,
    native_tx: mpsc::UnboundedSender<Vec<u8>>,
    native_pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    active_bridges: Arc<Mutex<HashSet<UpstreamKey>>>,
    shutdown_tx: mpsc::UnboundedSender<String>,
) {
    loop {
        let endpoints = candidate_endpoints(socket_arg.clone(), token_arg.clone());

        for endpoint in &endpoints {
            if !active_bridges.lock().await.is_empty() {
                break;
            }
            if !endpoint.token_path.exists() {
                continue;
            }

            let key = UpstreamKey::from(endpoint);
            let already_connected = active_bridges.lock().await.contains(&key);
            if already_connected {
                continue;
            }

            match connect_upstream_runtime(endpoint, &launch_context).await {
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
                        shutdown_tx.clone(),
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

async fn run_self_test(launch_context: NativeHostLaunchContext) -> Result<()> {
    let endpoints =
        candidate_endpoints(launch_context.socket.clone(), launch_context.token.clone());
    let payload = json!({
        "ok": true,
        "mode": "self_test",
        "stdout": "native-messaging-framed-only",
        "candidate_endpoint_count": endpoints.len(),
        "candidate_endpoints": endpoints
            .iter()
            .map(|endpoint| json!({
                "kind": endpoint.kind.label(),
                "protocol": endpoint.kind.protocol(),
                "socket_path": endpoint.socket_path,
                "socket_exists": endpoint.socket_path.exists(),
                "token_path": endpoint.token_path,
                "token_exists": endpoint.token_path.exists(),
            }))
            .collect::<Vec<_>>(),
        "caller_origin": launch_context.caller_origin,
        "parent_window_present": launch_context.parent_window.is_some(),
        "native_host_boot_id": native_host_boot_id(),
    });
    eprintln!("{}", serde_json::to_string(&payload)?);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let launch_context = NativeHostLaunchContext::from_env_args();
    if launch_context
        .extra_args
        .iter()
        .any(|arg| arg == "--self-test")
    {
        return run_self_test(launch_context).await;
    }
    if !launch_context.extra_args.is_empty() {
        info!(
            extra_arg_count = launch_context.extra_args.len(),
            "native-host launch contained extra args"
        );
    }
    let socket_arg = launch_context.socket.clone();
    let token_arg = launch_context.token.clone();

    match secure_dir("native-host-artifacts") {
        Ok(dir) => {
            if let Err(error) = cleanup_secure_artifacts(
                &dir,
                OVERSIZE_ARTIFACT_MAX_AGE,
                OVERSIZE_ARTIFACT_MAX_FILES,
            ) {
                warn!("Native-host artifact cleanup failed: {}", error);
            }
        }
        Err(error) => warn!("Native-host artifact directory setup failed: {}", error),
    }

    let (native_tx, mut native_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let native_pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let active_bridges: Arc<Mutex<HashSet<UpstreamKey>>> = Arc::new(Mutex::new(HashSet::new()));
    let (shutdown_tx, mut shutdown_rx) = mpsc::unbounded_channel::<String>();

    let shutdown_tx_writer = shutdown_tx.clone();
    let native_writer_task = tokio::spawn(async move {
        let mut stdout = io::BufWriter::new(io::stdout());
        while let Some(bytes) = native_rx.recv().await {
            if let Err(error) = write_native_message(&mut stdout, &bytes).await {
                let _ = shutdown_tx_writer.send(native_stdout_write_failed_shutdown_reason(&error));
                break;
            }
        }
    });

    let native_tx_heartbeat = native_tx.clone();
    let native_stdout_heartbeat_task = tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(Duration::from_millis(STDOUT_HEARTBEAT_INTERVAL_MS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        let mut seq = 0_u64;
        loop {
            interval.tick().await;
            seq = seq.saturating_add(1);
            let heartbeat = native_host_stdout_heartbeat(seq);
            let Ok(bytes) = serde_json::to_vec(&heartbeat) else {
                continue;
            };
            if native_tx_heartbeat.send(bytes).is_err() {
                break;
            }
        }
    });

    let endpoint_manager = tokio::spawn(endpoint_manager_loop(
        socket_arg.clone(),
        token_arg.clone(),
        launch_context.clone(),
        native_tx.clone(),
        native_pending.clone(),
        active_bridges.clone(),
        shutdown_tx.clone(),
    ));

    let native_pending_native = native_pending.clone();
    let native_tx_native = native_tx.clone();
    let socket_arg_for_control = socket_arg.clone();
    let token_arg_for_control = token_arg.clone();
    let mut native_reader_task = tokio::spawn(async move {
        let mut stdin = io::stdin();
        loop {
            let msg = match read_native_message(&mut stdin).await {
                Ok(Some(msg)) => msg,
                Ok(None) => return "native messaging stdin closed by browser".to_string(),
                Err(e) => {
                    warn!("Native read error: {}", e);
                    return format!("native messaging stdin read error: {}", e);
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
                    Err(error) => {
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
                }
            }

            if cloud_supervisor_method(&cmd_field(&v, "cmd").unwrap_or_default()).is_some() {
                continue;
            }

            // Extension session response handling. Correlate by request id first; a malformed
            // response should complete the pending call with a protocol error, not sit until timeout.
            if let Some((req_id, response)) = pending_extension_response(&v) {
                let tx = {
                    let mut guard = native_pending_native.lock().await;
                    guard.remove(&req_id)
                };
                if let Some(tx) = tx {
                    let _ = tx.send(response);
                    continue;
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

    tokio::select! {
        result = &mut native_reader_task => {
            let reason = result.unwrap_or_else(|error| format!("native reader task ended: {}", error));
            let drained = drain_native_pending_with_disconnect_error(&native_pending, &reason).await;
            if drained > 0 {
                tokio::time::sleep(Duration::from_millis(NATIVE_READER_EXIT_UPSTREAM_FLUSH_GRACE_MS)).await;
            }
        }
        reason = shutdown_rx.recv() => {
            info!(
                reason = reason.unwrap_or_else(|| "supervisor requested native-host restart".to_string()),
                "Shutting down native host"
            );
            native_reader_task.abort();
        }
    }
    endpoint_manager.abort();
    native_stdout_heartbeat_task.abort();
    native_writer_task.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::fs;

    fn base_candidate(label: &str) -> BaseCandidate {
        BaseCandidate {
            base: PathBuf::from(format!("/tmp/rzn-native-host-test-{label}")),
        }
    }

    #[test]
    fn candidate_endpoints_ignores_bases_without_supervisor_files() {
        let debug = base_candidate("debug");
        let cli = base_candidate("cli");

        let endpoints = candidate_endpoints_from_bases(&[debug, cli], None, None);

        assert!(endpoints.is_empty());
    }

    #[test]
    fn candidate_endpoints_accepts_explicit_supervisor_override() {
        let base = base_candidate("runtime");

        let endpoints = candidate_endpoints_from_bases(
            &[base],
            Some(PathBuf::from("/tmp/supervisor.sock")),
            Some(PathBuf::from("/tmp/supervisor.token")),
        );

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].kind, RuntimeBridgeKind::SupervisorLocalV1);
        assert_eq!(
            endpoints[0].socket_path,
            PathBuf::from("/tmp/supervisor.sock")
        );
        assert_eq!(
            endpoints[0].token_path,
            PathBuf::from("/tmp/supervisor.token"),
        );
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
        };

        let endpoints = candidate_endpoints_from_bases(&[base], None, None);

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].kind, RuntimeBridgeKind::SupervisorLocalV1);
        assert_eq!(endpoints[0].socket_path, socket_path);
        assert_eq!(endpoints[0].token_path, token_path);

        let _ = std::fs::remove_dir_all(base_dir);
    }

    #[test]
    fn parses_chrome_style_launch_context() {
        let context = parse_launch_context([
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
            "--parent-window=0",
            "--socket",
            "/tmp/a",
            "--token",
            "/tmp/b",
        ]);

        assert_eq!(
            context.caller_origin.as_deref(),
            Some("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/")
        );
        assert_eq!(context.parent_window.as_deref(), Some("0"));
        assert_eq!(context.socket.as_deref(), Some("/tmp/a"));
        assert_eq!(context.token.as_deref(), Some("/tmp/b"));
        assert!(context.extra_args.is_empty());
    }

    #[test]
    fn parses_equals_style_launch_context() {
        let context = parse_launch_context([
            "--socket=/tmp/a",
            "--token=/tmp/b",
            "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/",
        ]);

        assert_eq!(
            context.caller_origin.as_deref(),
            Some("chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/")
        );
        assert_eq!(context.socket.as_deref(), Some("/tmp/a"));
        assert_eq!(context.token.as_deref(), Some("/tmp/b"));
        assert_eq!(context.parent_window, None);
        assert!(context.extra_args.is_empty());
    }

    #[test]
    fn launch_context_allows_missing_caller_origin() {
        let context = parse_launch_context(["--socket", "/tmp/a", "--token=/tmp/b"]);

        assert_eq!(context.caller_origin, None);
        assert_eq!(context.socket.as_deref(), Some("/tmp/a"));
        assert_eq!(context.token.as_deref(), Some("/tmp/b"));
        assert!(context.extra_args.is_empty());
    }

    #[test]
    fn launch_context_keeps_first_valid_caller_origin() {
        let context = parse_launch_context([
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
            "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/",
            "--unknown",
        ]);

        assert_eq!(
            context.caller_origin.as_deref(),
            Some("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/")
        );
        assert_eq!(
            context.extra_args,
            vec![
                "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/".to_string(),
                "--unknown".to_string(),
            ]
        );
    }

    #[test]
    fn launch_context_preserves_invalid_extension_like_args_as_extra() {
        let context = parse_launch_context([
            "chrome-extension://missing-trailing-slash",
            "chrome-extension://qrstqrstqrstqrstqrstqrstqrstqrst/",
        ]);

        assert_eq!(context.caller_origin, None);
        assert_eq!(
            context.extra_args,
            vec![
                "chrome-extension://missing-trailing-slash".to_string(),
                "chrome-extension://qrstqrstqrstqrstqrstqrstqrstqrst/".to_string(),
            ]
        );
    }

    #[test]
    fn launch_metadata_contains_provenance_without_token_value() {
        let context = parse_launch_context([
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
            "--parent-window",
            "0",
            "--socket",
            "/tmp/socket",
            "--token",
            "/tmp/secret-token-path",
            "--diagnostic-extra",
        ]);
        let mut params = json!({
            "version": LOCAL_RUNTIME_PROTOCOL,
            "token": "supervisor-secret",
            "role": "native_host_bridge"
        });

        merge_native_host_launch_metadata(&mut params, &context);

        assert_eq!(
            params.get("caller_origin").and_then(Value::as_str),
            Some("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/")
        );
        assert_eq!(
            params.get("native_host_boot_id").and_then(Value::as_str),
            Some(native_host_boot_id())
        );
        assert_eq!(
            params
                .pointer("/launch/parent_window")
                .and_then(Value::as_str),
            Some("0")
        );
        assert_eq!(
            params
                .pointer("/launch/has_socket_override")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            params
                .pointer("/launch/has_token_override")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            params
                .pointer("/launch/extra_arg_count")
                .and_then(Value::as_u64),
            Some(1)
        );
        let encoded = serde_json::to_string(&params).unwrap();
        assert!(!encoded.contains("/tmp/secret-token-path"));
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
                "data": { "include_dom_snapshot": true },
                "supervisor_boot_id": "supervisor-test",
                "supervisor_bridge_id": "bridge-test",
                "supervisor_bridge_epoch": 42
            }
        }))
        .expect("native_host.extension_call is a bridge method")
        .expect("request should be valid");

        assert_eq!(request.upstream_request_id, "runtime-1");
        assert_eq!(request.cmd, "get_dom_snapshot");
        assert_eq!(request.original_req_id, "runtime-1");
        assert_eq!(
            request.supervisor_boot_id.as_deref(),
            Some("supervisor-test")
        );
        assert_eq!(request.supervisor_bridge_id.as_deref(), Some("bridge-test"));
        assert_eq!(request.supervisor_bridge_epoch, Some(42));
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
    fn parses_supervisor_shutdown_request() {
        let (id, reason) = parse_shutdown_request(&json!({
            "jsonrpc": "2.0",
            "id": "shutdown-1",
            "method": "native_host.shutdown",
            "params": {
                "reason": "zombie bridge"
            }
        }))
        .expect("native_host.shutdown should be recognized");

        assert_eq!(id, "shutdown-1");
        assert_eq!(reason, "zombie bridge");
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
    fn native_host_stdout_heartbeat_is_unsolicited_noop() {
        let heartbeat = native_host_stdout_heartbeat(7);

        assert_eq!(
            heartbeat.get("cmd").and_then(|v| v.as_str()),
            Some(STDOUT_HEARTBEAT_CMD)
        );
        assert!(heartbeat.get("req_id").is_none());
        assert_eq!(
            heartbeat
                .pointer("/payload/interval_ms")
                .and_then(|v| v.as_u64()),
            Some(STDOUT_HEARTBEAT_INTERVAL_MS)
        );
        assert_eq!(
            heartbeat.pointer("/payload/seq").and_then(|v| v.as_u64()),
            Some(7)
        );
        assert!(heartbeat
            .pointer("/payload/native_host_boot_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .starts_with("native-host-"));
    }

    #[test]
    fn pending_extension_response_accepts_well_formed_response() {
        let (_, response) = pending_extension_response(&json!({
            "req_id": "wire-1",
            "success": true,
            "result": { "ok": true }
        }))
        .expect("req_id should correlate");

        assert_eq!(
            response.get("req_id").and_then(Value::as_str),
            Some("wire-1")
        );
        assert_eq!(response.get("success").and_then(Value::as_bool), Some(true));
        assert_eq!(
            response.pointer("/result/ok").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn pending_extension_response_turns_malformed_match_into_protocol_error() {
        let (req_id, response) = pending_extension_response(&json!({
            "req_id": "wire-2",
            "lease_id": "lease-2",
            "result": { "ok": true }
        }))
        .expect("req_id should correlate before validating envelope shape");

        assert_eq!(req_id, "wire-2");
        assert_eq!(
            response.get("req_id").and_then(Value::as_str),
            Some("wire-2")
        );
        assert_eq!(
            response.get("lease_id").and_then(Value::as_str),
            Some("lease-2")
        );
        assert_eq!(
            response.get("success").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            response.get("error_code").and_then(Value::as_str),
            Some("EXTENSION_PROTOCOL_ERROR")
        );
    }

    #[test]
    fn extension_timeout_shutdown_reason_names_epoch_restart() {
        let reason = extension_timeout_shutdown_reason("execute_step", 40_000);

        assert!(reason.contains("execute_step"));
        assert!(reason.contains("40000ms"));
        assert!(reason.contains("native-host/native-port epoch"));
    }

    #[test]
    fn native_stdout_write_failed_shutdown_reason_names_epoch_restart() {
        let error = io::Error::new(io::ErrorKind::BrokenPipe, "pipe closed");
        let reason = native_stdout_write_failed_shutdown_reason(&error);

        assert!(reason.contains("stdout write failed"));
        assert!(reason.contains("pipe closed"));
        assert!(reason.contains("native-host/native-port epoch"));
    }

    #[tokio::test]
    async fn drain_native_pending_with_disconnect_error_resolves_waiters() {
        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = oneshot::channel();
        pending.lock().await.insert("wire-1".to_string(), tx);

        let drained =
            drain_native_pending_with_disconnect_error(&pending, "native messaging stdin closed")
                .await;
        let response = rx
            .await
            .expect("pending waiter should receive synthetic failure");

        assert_eq!(drained, 1);
        assert!(pending.lock().await.is_empty());
        assert_eq!(
            response.get("req_id").and_then(Value::as_str),
            Some("wire-1")
        );
        assert_eq!(
            response.get("error_code").and_then(Value::as_str),
            Some("NATIVE_HOST_DISCONNECTED")
        );
        assert_eq!(
            response.get("success").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[tokio::test]
    async fn write_native_message_rejects_host_to_chrome_frames_above_one_mib() {
        let payload = vec![b'x'; NATIVE_HOST_TO_CHROME_MAX_BYTES + 1];
        let mut sink = io::sink();
        let err = write_native_message(&mut sink, &payload)
            .await
            .expect_err("host-to-Chrome frames above 1 MiB must be rejected");

        assert!(err.to_string().contains("host-to-Chrome"));
    }

    #[tokio::test]
    async fn read_native_message_allows_chrome_to_host_limit_separately() {
        let oversized_len = (NATIVE_HOST_TO_CHROME_MAX_BYTES + 1) as u32;
        let mut frame = Vec::new();
        frame.extend_from_slice(&oversized_len.to_le_bytes());
        frame.resize(4 + oversized_len as usize, b'x');
        let mut input = frame.as_slice();

        let message = read_native_message(&mut input)
            .await
            .expect("Chrome-to-host frames use the larger protocol cap")
            .expect("frame is present");

        assert_eq!(message.len(), oversized_len as usize);
    }

    #[tokio::test]
    async fn oversize_response_artifact_is_small_reference() {
        let app_base =
            std::env::temp_dir().join(format!("rzn-native-host-artifact-test-{}", Uuid::new_v4()));
        std::env::set_var("RZN_APP_BASE_DIR", &app_base);

        let large = json!({
            "success": true,
            "result": {
                "blob": "x".repeat(4096)
            }
        });

        let artifact = write_oversize_response_artifact("execute_step", "req-1", &large, 4096)
            .await
            .expect("artifact writes");

        assert_eq!(
            artifact.pointer("/result/type").and_then(Value::as_str),
            Some("native_host_oversize_response_artifact")
        );
        let path = artifact
            .pointer("/result/path")
            .and_then(Value::as_str)
            .expect("artifact path");
        let path = std::path::Path::new(path);
        assert!(path.exists());
        assert!(path.starts_with(app_base.join("secure").join("native-host-artifacts")));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir_mode = std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            let file_mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(dir_mode, 0o700);
            assert_eq!(file_mode, 0o600);
        }

        std::env::remove_var("RZN_APP_BASE_DIR");
        let _ = fs::remove_dir_all(app_base).await;
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
        assert!(response
            .pointer(&format!("/result/{}", concat!("legacy", "_browser_bridge")))
            .is_none());
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
}
