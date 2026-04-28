//! MCP stdio worker that bridges browser automation via the native host/extension stack.
//! In v1, this provides a mock mode with clear health diagnostics when the extension/native
//! host are missing.
//!
//! Tools:
//! - browser.session_open
//! - browser.session_close
//! - browser.snapshot
//! - browser.execute_step
//! - browser.poll_events
//! - rzn.worker.health
//! - rzn.worker.shutdown

use interprocess::local_socket::{
    tokio::Stream as LocalSocketStream,
    traits::tokio::{Listener as _, Stream as _},
    GenericFilePath, ListenerOptions, ToFsName,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::time::{timeout, Duration};
use tracing::{info, warn};
use uuid::Uuid;

use rzn_broker_endpoint::{
    clear_broker_endpoint_browser_bridge, clear_broker_endpoint_browser_worker,
    update_broker_endpoint_browser_bridge, update_broker_endpoint_browser_worker,
};

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_HANDSHAKE_BYTES: usize = 64 * 1024;
const HANDSHAKE_TIMEOUT_MS: u64 = 2000;
const BRIDGE_KEEPALIVE_INTERVAL_MS: u64 = 10_000;
const BRIDGE_KEEPALIVE_TIMEOUT_MS: u64 = 5_000;
const BROWSER_SYSTEM_ID: &str = "browser_automation";
const BROWSER_SYSTEM_METADATA_RELATIVE_PATH: &str =
    "resources/systems/browser_automation/system.metadata.yaml";
const BROWSER_SYSTEM_EXAMPLES_RELATIVE_DIR: &str = "examples/browser_automation";
const BRIDGE_TOKEN_FILENAME: &str = "browser_bridge_token_v1";
const BRIDGE_SOCKET_FILENAME: &str = "rzn-browser-bridge.sock";
const WORKER_CONTROL_TOKEN_FILENAME: &str = "browser_worker_token_v1";
const WORKER_CONTROL_SOCKET_FILENAME: &str = "rzn-browser-worker.sock";
const NATIVE_CALL_TIMEOUT_MS: u64 = 20000;
const HEALTH_PING_TIMEOUT_MS: u64 = 2500;
const SCREENSHOT_TIMEOUT_MS: u64 = 30000;
const NATIVE_STEP_TIMEOUT_GRACE_MS: u64 = 5000;

fn env_trimmed(key: &str) -> Option<String> {
    std::env::var(key).ok().and_then(|v| {
        let trimmed = v.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn infer_app_base_from_exe() -> Option<PathBuf> {
    // When installed as a plugin bundle, the worker lives under:
    //   {APP_BASE}/plugins/<plugin_id>/<version or current>/bin/.../rzn-browser-worker
    //
    // The desktop app normally passes RZN_APP_BASE_DIR when spawning plugin
    // workers, but this fallback keeps manual testing reliable and mirrors the
    // native host strategy.
    let exe = std::env::current_exe().ok()?;
    for ancestor in exe.ancestors() {
        if ancestor.file_name().and_then(|s| s.to_str()) == Some("plugins") {
            return ancestor.parent().map(|p| p.to_path_buf());
        }
    }
    None
}

fn default_app_base_dir() -> PathBuf {
    // Keep behavior close to the desktop host (debug vs release data dirs), but avoid
    // a hard dependency on the rznapp `config` crate so this worker can live in the
    // rzn-browser repo.
    let Some(data) = dirs::data_dir() else {
        return std::env::temp_dir();
    };
    if cfg!(debug_assertions) {
        data.join("rzn_debug")
    } else {
        data.join("rzn")
    }
}

fn resolve_app_base_dir() -> PathBuf {
    if let Some(base) = env_trimmed("RZN_APP_BASE_DIR") {
        return PathBuf::from(base);
    }
    if let Some(base) = infer_app_base_from_exe() {
        return base;
    }
    default_app_base_dir()
}

fn ensure_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format!("Failed to create dir {:?}: {}", path, e))
}

fn bridge_token_path(base: &Path) -> PathBuf {
    base.join("secure").join(BRIDGE_TOKEN_FILENAME)
}

fn bridge_socket_path(base: &Path) -> PathBuf {
    base.join("run").join(BRIDGE_SOCKET_FILENAME)
}

fn worker_control_token_path(base: &Path) -> PathBuf {
    base.join("secure").join(WORKER_CONTROL_TOKEN_FILENAME)
}

fn worker_control_socket_path(base: &Path) -> PathBuf {
    base.join("run").join(WORKER_CONTROL_SOCKET_FILENAME)
}

#[cfg(unix)]
fn unix_socket_path_limit() -> usize {
    100
}

#[cfg(not(unix))]
fn unix_socket_path_limit() -> usize {
    usize::MAX
}

#[cfg(unix)]
fn short_socket_fallback() -> PathBuf {
    let uid = unsafe { libc::geteuid() };
    std::env::temp_dir().join(format!("rzn-browser-bridge-{}.sock", uid))
}

#[cfg(not(unix))]
fn short_socket_fallback() -> PathBuf {
    std::env::temp_dir().join("rzn-browser-bridge.sock")
}

#[cfg(unix)]
fn short_worker_socket_fallback() -> PathBuf {
    let uid = unsafe { libc::geteuid() };
    std::env::temp_dir().join(format!("rzn-browser-worker-{}.sock", uid))
}

#[cfg(not(unix))]
fn short_worker_socket_fallback() -> PathBuf {
    std::env::temp_dir().join("rzn-browser-worker.sock")
}

fn resolve_bridge_token_path(base: &Path) -> PathBuf {
    if let Some(v) = env_trimmed("RZN_BROWSER_BRIDGE_TOKEN_PATH") {
        return PathBuf::from(v);
    }
    bridge_token_path(base)
}

fn resolve_bridge_socket_path(base: &Path) -> PathBuf {
    if let Some(v) = env_trimmed("RZN_BROWSER_BRIDGE_SOCKET_PATH") {
        return PathBuf::from(v);
    }
    let desired = bridge_socket_path(base);
    let len = desired.to_string_lossy().len();
    if len >= unix_socket_path_limit() {
        warn!(
            "Browser bridge socket path too long ({} bytes): {:?}; using short fallback",
            len, desired
        );
        return short_socket_fallback();
    }
    desired
}

fn resolve_worker_control_token_path(base: &Path) -> PathBuf {
    if let Some(v) = env_trimmed("RZN_BROWSER_WORKER_TOKEN_PATH") {
        return PathBuf::from(v);
    }
    worker_control_token_path(base)
}

fn resolve_worker_control_socket_path(base: &Path) -> PathBuf {
    if let Some(v) = env_trimmed("RZN_BROWSER_WORKER_SOCKET_PATH") {
        return PathBuf::from(v);
    }
    let desired = worker_control_socket_path(base);
    let len = desired.to_string_lossy().len();
    if len >= unix_socket_path_limit() {
        warn!(
            "Browser worker socket path too long ({} bytes): {:?}; using short fallback",
            len, desired
        );
        return short_worker_socket_fallback();
    }
    desired
}

fn read_token_file(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(s) => {
            let t = s.trim().to_string();
            if t.is_empty() {
                Ok(None)
            } else {
                Ok(Some(t))
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Failed to read token {:?}: {}", path, e)),
    }
}

fn write_token_file(path: &Path, token: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    match fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
    {
        Ok(mut f) => {
            use std::io::Write;
            f.write_all(token.as_bytes())
                .map_err(|e| format!("Failed to write token {:?}: {}", path, e))?;
            f.write_all(b"\n")
                .map_err(|e| format!("Failed to write token newline {:?}: {}", path, e))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("Failed to create token file {:?}: {}", path, e)),
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

fn get_or_create_token(path: &Path) -> Result<String, String> {
    if let Some(token) = read_token_file(path)? {
        return Ok(token);
    }
    let token = Uuid::new_v4().to_string();
    write_token_file(path, &token)?;
    read_token_file(path)?.ok_or_else(|| "Token file still empty".to_string())
}

fn remove_stale_socket(path: &Path) {
    if path.exists() {
        let _ = fs::remove_file(path);
    }
}

fn cleanup_runtime_artifacts(
    app_base: &Path,
    bridge_info: &BrowserBridgeInfo,
    control_info: &BrowserWorkerControlInfo,
) {
    let pid = Some(std::process::id());
    if let Err(e) = clear_broker_endpoint_browser_bridge(
        app_base,
        &bridge_info.socket_path.to_string_lossy(),
        pid,
    ) {
        warn!("[browser-bridge] failed to clear endpoint file: {}", e);
    }
    if let Err(e) = clear_broker_endpoint_browser_worker(
        app_base,
        &control_info.socket_path.to_string_lossy(),
        pid,
    ) {
        warn!("[browser-worker] failed to clear endpoint file: {}", e);
    }
    remove_stale_socket(&bridge_info.socket_path);
    remove_stale_socket(&control_info.socket_path);
}

fn plugin_native_host_candidates(root: &Path) -> [PathBuf; 5] {
    [
        root.join("bin/macos/universal/rzn-native-host"),
        root.join("bin/macos/arm64/rzn-native-host"),
        root.join("bin/macos/x86_64/rzn-native-host"),
        root.join("bin/windows/x86_64/rzn-native-host.exe"),
        root.join("bin/linux/x86_64/rzn-native-host"),
    ]
}

fn native_host_manifest_paths() -> Vec<PathBuf> {
    let host_names =
        vec![env_trimmed("RZN_NATIVE_HOST_NAME")
            .unwrap_or_else(|| "com.rzn.browser.broker".to_string())];
    #[cfg(target_os = "windows")]
    {
        return vec![];
    }

    #[cfg(not(target_os = "windows"))]
    {
        let Some(home) = dirs::home_dir() else {
            return vec![];
        };

        #[cfg(target_os = "macos")]
        let dir = home.join("Library/Application Support/Google/Chrome/NativeMessagingHosts");
        #[cfg(target_os = "linux")]
        let dir = home.join(".config/google-chrome/NativeMessagingHosts");
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            return vec![];
        }

        return host_names
            .into_iter()
            .map(|host_name| dir.join(format!("{}.json", host_name)))
            .collect();
    }
}

fn parse_native_host_path_from_manifest(contents: &str) -> Option<String> {
    let value: Value = serde_json::from_str(contents).ok()?;
    let path = value.get("path")?.as_str()?.trim();
    if path.is_empty() {
        return None;
    }
    Some(path.to_string())
}

fn detect_native_host_path() -> Option<String> {
    if let Some(path) = env_trimmed("RZN_NATIVE_HOST_PATH") {
        if Path::new(&path).exists() {
            return Some(path);
        }
    }

    if let Some(plugin_dir) = resolve_plugin_dir() {
        for candidate in plugin_native_host_candidates(&plugin_dir) {
            if candidate.exists() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }

    for manifest_path in native_host_manifest_paths() {
        if let Ok(contents) = fs::read_to_string(manifest_path) {
            if let Some(path) = parse_native_host_path_from_manifest(&contents) {
                if Path::new(&path).exists() {
                    return Some(path);
                }
            }
        }
    }

    let Some(home) = dirs::home_dir() else {
        return None;
    };

    #[cfg(target_os = "macos")]
    let install_candidates = [home.join("Library/Application Support/RZN/bin/rzn-native-host")];
    #[cfg(target_os = "linux")]
    let install_candidates = [home.join(".local/share/rzn/bin/rzn-native-host")];
    #[cfg(target_os = "windows")]
    let install_candidates = [PathBuf::from(r"C:\Program Files\RZN\rzn-native-host.exe")];
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let install_candidates: [PathBuf; 0] = [];

    for candidate in install_candidates {
        if candidate.exists() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }

    None
}

fn resolve_plugin_dir() -> Option<PathBuf> {
    env_trimmed("RZN_PLUGIN_DIR").map(PathBuf::from)
}

fn browser_system_metadata_path(plugin_dir: &Path) -> PathBuf {
    plugin_dir.join(BROWSER_SYSTEM_METADATA_RELATIVE_PATH)
}

fn browser_system_examples_dir(plugin_dir: &Path) -> PathBuf {
    plugin_dir.join(BROWSER_SYSTEM_EXAMPLES_RELATIVE_DIR)
}

fn list_relative_files(root: &Path, current: &Path, output: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };

    let mut paths = entries
        .filter_map(|entry| entry.ok().map(|item| item.path()))
        .collect::<Vec<_>>();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            list_relative_files(root, &path, output);
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        output.push(relative);
    }
}

fn browser_package_inventory(plugin_dir: Option<&Path>) -> Value {
    let Some(plugin_dir) = plugin_dir else {
        return json!({
            "configured": false,
            "system_id": BROWSER_SYSTEM_ID,
            "metadata_exists": false,
            "examples_exist": false,
            "example_count": 0,
            "example_files": [],
        });
    };

    let metadata_path = browser_system_metadata_path(plugin_dir);
    let examples_dir = browser_system_examples_dir(plugin_dir);
    let mut example_files = Vec::new();
    if examples_dir.is_dir() {
        list_relative_files(plugin_dir, &examples_dir, &mut example_files);
    }

    json!({
        "configured": true,
        "plugin_dir": plugin_dir.to_string_lossy(),
        "system_id": BROWSER_SYSTEM_ID,
        "metadata_path": metadata_path.to_string_lossy(),
        "metadata_exists": metadata_path.is_file(),
        "examples_dir": examples_dir.to_string_lossy(),
        "examples_exist": examples_dir.is_dir(),
        "example_count": example_files.len(),
        "example_files": example_files,
    })
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

async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Option<Vec<u8>>, String> {
    let mut len_buf = [0u8; 4];
    if let Err(e) = reader.read_exact(&mut len_buf).await {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            return Ok(None);
        }
        return Err(format!("read length: {}", e));
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(format!("frame too large: {}", len));
    }
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|e| format!("read frame: {}", e))?;
    Ok(Some(buf))
}

async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, payload: &[u8]) -> Result<(), String> {
    let len = payload.len();
    if len > MAX_FRAME_BYTES {
        return Err(format!("frame too large: {}", len));
    }
    let len_buf = (len as u32).to_le_bytes();
    writer
        .write_all(&len_buf)
        .await
        .map_err(|e| format!("write length: {}", e))?;
    writer
        .write_all(payload)
        .await
        .map_err(|e| format!("write payload: {}", e))?;
    writer.flush().await.map_err(|e| format!("flush: {}", e))?;
    Ok(())
}

fn id_key(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn bridge_keepalive_response_is_healthy(response: &Value) -> bool {
    if response.get("error").is_some() {
        return false;
    }

    !matches!(
        response
            .pointer("/result/success")
            .and_then(|v| v.as_bool()),
        Some(false)
    )
}

#[derive(Clone, Debug)]
struct BridgeSessionMeta {
    session_id: String,
    client_id: String,
    connected_at_ms: u64,
    last_seen_at_ms: Arc<std::sync::atomic::AtomicU64>,
    last_ping_ok_at_ms: Arc<std::sync::atomic::AtomicU64>,
}

impl BridgeSessionMeta {
    fn snapshot(&self, pending_count: usize) -> Value {
        json!({
            "session_id": self.session_id,
            "client_id": self.client_id,
            "connected_at_ms": self.connected_at_ms,
            "last_seen_at_ms": self.last_seen_at_ms.load(Ordering::SeqCst),
            "last_ping_ok_at_ms": self.last_ping_ok_at_ms.load(Ordering::SeqCst),
            "pending_count": pending_count,
        })
    }
}

#[derive(Clone, Debug, Default)]
struct BridgeDiagnostics {
    handshake_failures_total: u64,
    evictions_total: u64,
    keepalive_failures_total: u64,
    last_handshake_error: Option<Value>,
    last_eviction: Option<Value>,
}

#[derive(Clone, Debug)]
struct HandshakeFailure {
    code: &'static str,
    message: String,
}

#[derive(Clone)]
struct NativeHostHandle {
    client_id: String,
    session_id: String,
    sender: mpsc::UnboundedSender<Vec<u8>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    meta: BridgeSessionMeta,
}

#[derive(Default)]
struct NativeHostRegistry {
    hosts: Vec<NativeHostHandle>,
}

impl NativeHostRegistry {
    fn register(&mut self, handle: NativeHostHandle) {
        self.hosts.retain(|h| h.session_id != handle.session_id);
        self.hosts.push(handle);
    }

    fn unregister(&mut self, session_id: &str) {
        self.hosts.retain(|h| h.session_id != session_id);
    }

    fn select_host(&self, target: Option<&str>) -> Option<NativeHostHandle> {
        if let Some(target) = target {
            self.hosts
                .iter()
                .find(|h| h.client_id == target || h.session_id == target)
                .cloned()
        } else {
            self.hosts
                .iter()
                .max_by_key(|host| {
                    (
                        host.meta.last_ping_ok_at_ms.load(Ordering::SeqCst),
                        host.meta.last_seen_at_ms.load(Ordering::SeqCst),
                        host.meta.connected_at_ms,
                    )
                })
                .cloned()
        }
    }

    fn has_hosts(&self) -> bool {
        !self.hosts.is_empty()
    }

    fn host_count(&self) -> usize {
        self.hosts.len()
    }

    fn client_ids(&self) -> Vec<String> {
        self.hosts
            .iter()
            .map(|host| host.client_id.clone())
            .collect()
    }
}

#[derive(Clone, Debug)]
struct BrowserBridgeInfo {
    socket_path: PathBuf,
    token_path: PathBuf,
}

#[derive(Clone, Debug)]
struct BrowserWorkerControlInfo {
    socket_path: PathBuf,
    token_path: PathBuf,
}

#[derive(Clone)]
struct BrowserBridge {
    info: BrowserBridgeInfo,
    registry: Arc<Mutex<NativeHostRegistry>>,
    diagnostics: Arc<Mutex<BridgeDiagnostics>>,
}

impl BrowserBridge {
    async fn has_connection(&self) -> bool {
        self.registry.lock().await.has_hosts()
    }

    async fn host_count(&self) -> usize {
        self.registry.lock().await.host_count()
    }

    async fn client_ids(&self) -> Vec<String> {
        self.registry.lock().await.client_ids()
    }

    async fn session_snapshots(&self) -> Vec<Value> {
        let hosts = {
            let guard = self.registry.lock().await;
            guard.hosts.clone()
        };
        let mut sessions = Vec::with_capacity(hosts.len());
        for host in hosts {
            let pending_count = host.pending.lock().await.len();
            sessions.push(host.meta.snapshot(pending_count));
        }
        sessions
    }

    async fn diagnostics_snapshot(&self) -> Value {
        let guard = self.diagnostics.lock().await;
        json!({
            "handshake_failures_total": guard.handshake_failures_total,
            "evictions_total": guard.evictions_total,
            "keepalive_failures_total": guard.keepalive_failures_total,
            "last_handshake_error": guard.last_handshake_error,
            "last_eviction": guard.last_eviction,
        })
    }

    async fn evict_session(&self, session_id: &str, reason: &str, keepalive_failure: bool) {
        let removed = {
            let mut registry = self.registry.lock().await;
            let before = registry.hosts.len();
            registry.unregister(session_id);
            before != registry.hosts.len()
        };
        if removed {
            let mut diagnostics = self.diagnostics.lock().await;
            diagnostics.evictions_total += 1;
            if keepalive_failure {
                diagnostics.keepalive_failures_total += 1;
            }
            diagnostics.last_eviction = Some(json!({
                "session_id": session_id,
                "reason": reason,
                "at_ms": now_epoch_ms(),
            }));
        }
    }

    async fn send_native_request(
        &self,
        method: &str,
        params: Value,
        timeout_ms: Option<u64>,
        target: Option<String>,
    ) -> Result<Value, String> {
        let handle = {
            let guard = self.registry.lock().await;
            guard.select_host(target.as_deref())
        }
        .ok_or_else(|| "No native host connected".to_string())?;

        let request_id = Uuid::new_v4().to_string();
        let request = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params
        });
        let bytes = serde_json::to_vec(&request).map_err(|e| format!("encode request: {}", e))?;

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = handle.pending.lock().await;
            pending.insert(request_id.clone(), tx);
        }

        if handle.sender.send(bytes).is_err() {
            let mut pending = handle.pending.lock().await;
            pending.remove(&request_id);
            drop(pending);
            self.evict_session(&handle.session_id, "send to native host failed", false)
                .await;
            return Err("Native host disconnected".to_string());
        }

        let timeout_ms = timeout_ms.unwrap_or(NATIVE_CALL_TIMEOUT_MS);
        let response = match timeout(Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                let mut pending = handle.pending.lock().await;
                pending.remove(&request_id);
                drop(pending);
                self.evict_session(&handle.session_id, "native response channel closed", false)
                    .await;
                return Err("Native host response channel closed".to_string());
            }
            Err(_) => {
                let mut pending = handle.pending.lock().await;
                pending.remove(&request_id);
                drop(pending);
                self.evict_session(&handle.session_id, "native host request timed out", true)
                    .await;
                return Err(format!("Native host timeout after {}ms", timeout_ms));
            }
        };

        if let Some(error) = response.get("error") {
            self.evict_session(
                &handle.session_id,
                &format!("native host returned error: {}", error),
                false,
            )
            .await;
            return Err(format!("Native host error: {}", error));
        }
        if let Some(result) = response.get("result") {
            return Ok(result.clone());
        }
        self.evict_session(
            &handle.session_id,
            "native host response missing result",
            false,
        )
        .await;
        Err("Native host response missing result".to_string())
    }
}

async fn start_browser_bridge(
    app_base: &Path,
    registry: Arc<Mutex<NativeHostRegistry>>,
) -> Result<(BrowserBridgeInfo, Arc<Mutex<BridgeDiagnostics>>), String> {
    ensure_dir(app_base)?;
    let secure_dir = app_base.join("secure");
    let run_dir = app_base.join("run");
    ensure_dir(&secure_dir)?;
    ensure_dir(&run_dir)?;

    let token_path = resolve_bridge_token_path(app_base);
    get_or_create_token(&token_path)?;
    let socket_path = resolve_bridge_socket_path(app_base);
    remove_stale_socket(&socket_path);

    let name = socket_path
        .clone()
        .to_fs_name::<GenericFilePath>()
        .map_err(|e| format!("Invalid socket path {:?}: {}", socket_path, e))?;

    let listener = ListenerOptions::new()
        .name(name)
        .create_tokio()
        .map_err(|e| format!("Failed to bind bridge socket {:?}: {}", socket_path, e))?;

    info!(
        "[browser-bridge] listening socket={:?} token_path={:?}",
        socket_path, token_path
    );

    let diagnostics = Arc::new(Mutex::new(BridgeDiagnostics::default()));
    let info_out = BrowserBridgeInfo {
        socket_path: socket_path.clone(),
        token_path: token_path.clone(),
    };

    if let Err(e) = update_broker_endpoint_browser_bridge(
        app_base,
        info_out.socket_path.to_string_lossy().to_string(),
        info_out.token_path.to_string_lossy().to_string(),
        Some(std::process::id()),
    ) {
        warn!("[browser-bridge] failed to write endpoint file: {}", e);
    }

    let diagnostics_for_accept = diagnostics.clone();
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok(stream) => {
                    let token_path = token_path.clone();
                    let registry = registry.clone();
                    let diagnostics = diagnostics_for_accept.clone();
                    tokio::spawn(handle_bridge_connection(
                        stream,
                        token_path,
                        registry,
                        diagnostics,
                    ));
                }
                Err(e) => {
                    warn!("[browser-bridge] accept error: {}", e);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    });

    Ok((info_out, diagnostics))
}

async fn start_worker_control(
    app_base: &Path,
    worker: BrowserWorker,
    shutdown: Arc<AtomicBool>,
) -> Result<BrowserWorkerControlInfo, String> {
    ensure_dir(app_base)?;
    let secure_dir = app_base.join("secure");
    let run_dir = app_base.join("run");
    ensure_dir(&secure_dir)?;
    ensure_dir(&run_dir)?;

    let token_path = resolve_worker_control_token_path(app_base);
    let token = get_or_create_token(&token_path)?;
    let socket_path = resolve_worker_control_socket_path(app_base);
    remove_stale_socket(&socket_path);

    let name = socket_path
        .clone()
        .to_fs_name::<GenericFilePath>()
        .map_err(|e| format!("Invalid socket path {:?}: {}", socket_path, e))?;

    let listener = ListenerOptions::new()
        .name(name)
        .create_tokio()
        .map_err(|e| {
            format!(
                "Failed to bind worker control socket {:?}: {}",
                socket_path, e
            )
        })?;

    info!(
        "[browser-worker] control listening socket={:?} token_path={:?}",
        socket_path, token_path
    );

    let info_out = BrowserWorkerControlInfo {
        socket_path: socket_path.clone(),
        token_path: token_path.clone(),
    };

    if let Err(e) = update_broker_endpoint_browser_worker(
        app_base,
        info_out.socket_path.to_string_lossy().to_string(),
        info_out.token_path.to_string_lossy().to_string(),
        Some(std::process::id()),
    ) {
        warn!("[browser-worker] failed to write endpoint file: {}", e);
    }

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok(stream) => {
                    let token = token.clone();
                    let worker = worker.clone();
                    let shutdown = shutdown.clone();
                    tokio::spawn(handle_worker_control_connection(
                        stream, token, worker, shutdown,
                    ));
                }
                Err(e) => {
                    warn!("[browser-worker] control accept error: {}", e);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    });

    Ok(info_out)
}

async fn handle_worker_control_connection(
    stream: LocalSocketStream,
    expected_token: String,
    worker: BrowserWorker,
    shutdown: Arc<AtomicBool>,
) {
    let (mut reader, writer) = stream.split();
    let (writer_tx, mut writer_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let writer_task = tokio::spawn(async move {
        let mut writer = writer;
        while let Some(bytes) = writer_rx.recv().await {
            if write_frame(&mut writer, &bytes).await.is_err() {
                break;
            }
        }
    });

    let frame = match timeout(
        Duration::from_millis(HANDSHAKE_TIMEOUT_MS),
        read_frame(&mut reader),
    )
    .await
    {
        Ok(Ok(Some(frame))) => frame,
        Ok(Ok(None)) => {
            drop(writer_tx);
            let _ = writer_task.await;
            return;
        }
        Ok(Err(e)) => {
            warn!("[browser-worker] handshake read error: {}", e);
            drop(writer_tx);
            let _ = writer_task.await;
            return;
        }
        Err(_) => {
            warn!("[browser-worker] handshake timeout");
            drop(writer_tx);
            let _ = writer_task.await;
            return;
        }
    };

    if frame.len() > MAX_HANDSHAKE_BYTES {
        warn!("[browser-worker] handshake too large: {}", frame.len());
        drop(writer_tx);
        let _ = writer_task.await;
        return;
    }

    let handshake: Value = match serde_json::from_slice(&frame) {
        Ok(v) => v,
        Err(e) => {
            warn!("[browser-worker] handshake parse error: {}", e);
            drop(writer_tx);
            let _ = writer_task.await;
            return;
        }
    };

    let message_type = handshake.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let token = handshake
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let version = handshake.get("v").and_then(|v| v.as_u64()).unwrap_or(0);
    if message_type != "rzn_browser_worker_handshake" || version != 1 || token != expected_token {
        let resp = json!({
            "type": "rzn_browser_worker_handshake_ok",
            "v": 1,
            "ok": false,
            "error": { "code": "AUTH_FAILED", "message": "Invalid handshake" }
        });
        let _ = writer_tx.send(serde_json::to_vec(&resp).unwrap_or_default());
        drop(writer_tx);
        let _ = writer_task.await;
        return;
    }

    let resp = json!({
        "type": "rzn_browser_worker_handshake_ok",
        "v": 1,
        "ok": true,
        "protocol": "mcp+framed/1"
    });
    let _ = writer_tx.send(serde_json::to_vec(&resp).unwrap_or_default());

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        let frame = match read_frame(&mut reader).await {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(e) => {
                warn!("[browser-worker] control read error: {}", e);
                break;
            }
        };

        let request: Value = match serde_json::from_slice(&frame) {
            Ok(v) => v,
            Err(e) => {
                let resp = jsonrpc_error(None, -32700, &format!("Parse error: {}", e));
                let _ = writer_tx.send(serde_json::to_vec(&resp).unwrap_or_default());
                continue;
            }
        };

        if let Some(resp) = process_mcp_request(&worker, &shutdown, request).await {
            let _ = writer_tx.send(serde_json::to_vec(&resp).unwrap_or_default());
        }
    }

    drop(writer_tx);
    let _ = writer_task.await;
}

async fn handle_bridge_connection(
    stream: LocalSocketStream,
    token_path: PathBuf,
    registry: Arc<Mutex<NativeHostRegistry>>,
    diagnostics: Arc<Mutex<BridgeDiagnostics>>,
) {
    let (mut reader, writer) = stream.split();
    let (writer_tx, mut writer_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let writer_task = tokio::spawn(async move {
        let mut writer = writer;
        while let Some(bytes) = writer_rx.recv().await {
            if write_frame(&mut writer, &bytes).await.is_err() {
                break;
            }
        }
    });

    let frame = match timeout(
        Duration::from_millis(HANDSHAKE_TIMEOUT_MS),
        read_frame(&mut reader),
    )
    .await
    {
        Ok(Ok(Some(frame))) => frame,
        Ok(Ok(None)) => {
            drop(writer_tx);
            let _ = writer_task.await;
            return;
        }
        Ok(Err(e)) => {
            warn!("[browser-bridge] handshake read error: {}", e);
            record_bridge_handshake_failure(
                &diagnostics,
                HandshakeFailure {
                    code: "HANDSHAKE_READ_ERROR",
                    message: e,
                },
            )
            .await;
            drop(writer_tx);
            let _ = writer_task.await;
            return;
        }
        Err(_) => {
            warn!("[browser-bridge] handshake timeout");
            record_bridge_handshake_failure(
                &diagnostics,
                HandshakeFailure {
                    code: "HANDSHAKE_TIMEOUT",
                    message: format!("No handshake frame within {}ms", HANDSHAKE_TIMEOUT_MS),
                },
            )
            .await;
            drop(writer_tx);
            let _ = writer_task.await;
            return;
        }
    };

    if frame.len() > MAX_HANDSHAKE_BYTES {
        warn!("[browser-bridge] handshake too large: {}", frame.len());
        record_bridge_handshake_failure(
            &diagnostics,
            HandshakeFailure {
                code: "HANDSHAKE_TOO_LARGE",
                message: format!("Handshake frame exceeded {} bytes", MAX_HANDSHAKE_BYTES),
            },
        )
        .await;
        drop(writer_tx);
        let _ = writer_task.await;
        return;
    }

    let handshake: Value = match serde_json::from_slice(&frame) {
        Ok(v) => v,
        Err(e) => {
            warn!("[browser-bridge] handshake parse error: {}", e);
            record_bridge_handshake_failure(
                &diagnostics,
                HandshakeFailure {
                    code: "HANDSHAKE_PARSE_ERROR",
                    message: e.to_string(),
                },
            )
            .await;
            drop(writer_tx);
            let _ = writer_task.await;
            return;
        }
    };

    if let Err(failure) = validate_bridge_handshake(&handshake, &token_path) {
        let resp = json!({
            "type": "rzn_browser_bridge_handshake_ok",
            "v": 1,
            "ok": false,
            "error": { "code": failure.code, "message": failure.message }
        });
        let _ = writer_tx.send(serde_json::to_vec(&resp).unwrap_or_default());
        record_bridge_handshake_failure(&diagnostics, failure).await;
        drop(writer_tx);
        let _ = writer_task.await;
        return;
    }

    let session_id = Uuid::new_v4().to_string();
    let client = handshake
        .get("client")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let client_id = client
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            client
                .get("kind")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| session_id.clone());

    let resp = json!({
        "type": "rzn_browser_bridge_handshake_ok",
        "v": 1,
        "ok": true,
        "session_id": session_id,
        "server": {
            "name": "rzn-browser-worker",
            "version": env!("CARGO_PKG_VERSION"),
            "protocol": "rzn-browser-bridge/1"
        }
    });
    if writer_tx
        .send(serde_json::to_vec(&resp).unwrap_or_default())
        .is_err()
    {
        drop(writer_tx);
        let _ = writer_task.await;
        return;
    }

    let pending = Arc::new(Mutex::new(HashMap::<String, oneshot::Sender<Value>>::new()));
    let connected_at_ms = now_epoch_ms();
    let last_seen_at_ms = Arc::new(std::sync::atomic::AtomicU64::new(connected_at_ms));
    let last_ping_ok_at_ms = Arc::new(std::sync::atomic::AtomicU64::new(connected_at_ms));
    let meta = BridgeSessionMeta {
        session_id: session_id.clone(),
        client_id: client_id.clone(),
        connected_at_ms,
        last_seen_at_ms: last_seen_at_ms.clone(),
        last_ping_ok_at_ms: last_ping_ok_at_ms.clone(),
    };
    let (close_tx, close_rx) = watch::channel(false);
    let mut read_close_rx = close_rx.clone();
    {
        let mut guard = registry.lock().await;
        guard.register(NativeHostHandle {
            client_id: client_id.clone(),
            session_id: session_id.clone(),
            sender: writer_tx.clone(),
            pending: pending.clone(),
            meta,
        });
    }

    info!(
        "[browser-bridge] connected session_id={} client_id={}",
        session_id, client_id
    );

    let keepalive_pending = pending.clone();
    let keepalive_writer = writer_tx.clone();
    let keepalive_registry = registry.clone();
    let keepalive_diagnostics = diagnostics.clone();
    let keepalive_session_id = session_id.clone();
    let keepalive_last_ping_ok_at_ms = last_ping_ok_at_ms.clone();
    let keepalive_task = tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(Duration::from_millis(BRIDGE_KEEPALIVE_INTERVAL_MS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;
        let mut keepalive_failed = false;
        loop {
            if *close_rx.borrow() {
                break;
            }

            let ping_id = format!("bridge-ping-{}", Uuid::new_v4());
            let ping = json!({
                "jsonrpc": "2.0",
                "id": ping_id,
                "method": "browser.session",
                "params": {
                    "cmd": "ping",
                    "payload": {
                        "source": "browser_bridge_keepalive"
                    }
                }
            });
            let bytes = match serde_json::to_vec(&ping) {
                Ok(bytes) => bytes,
                Err(e) => {
                    warn!(
                        "[browser-bridge] keepalive encode error session_id={} err={}",
                        keepalive_session_id, e
                    );
                    break;
                }
            };

            let (tx, rx) = oneshot::channel();
            {
                let mut guard = keepalive_pending.lock().await;
                guard.insert(ping_id.clone(), tx);
            }

            if keepalive_writer.send(bytes).is_err() {
                let mut guard = keepalive_pending.lock().await;
                guard.remove(&ping_id);
                keepalive_failed = true;
                break;
            }

            match timeout(Duration::from_millis(BRIDGE_KEEPALIVE_TIMEOUT_MS), rx).await {
                Ok(Ok(response)) if bridge_keepalive_response_is_healthy(&response) => {
                    keepalive_last_ping_ok_at_ms.store(now_epoch_ms(), Ordering::SeqCst);
                }
                Ok(Ok(response)) => {
                    warn!(
                        "[browser-bridge] keepalive unhealthy response session_id={} response={}",
                        keepalive_session_id, response
                    );
                    keepalive_failed = true;
                    break;
                }
                Ok(Err(_)) => {
                    let mut guard = keepalive_pending.lock().await;
                    guard.remove(&ping_id);
                    warn!(
                        "[browser-bridge] keepalive response channel closed session_id={}",
                        keepalive_session_id
                    );
                    keepalive_failed = true;
                    break;
                }
                Err(_) => {
                    let mut guard = keepalive_pending.lock().await;
                    guard.remove(&ping_id);
                    warn!(
                        "[browser-bridge] keepalive timeout session_id={} timeout_ms={}",
                        keepalive_session_id, BRIDGE_KEEPALIVE_TIMEOUT_MS
                    );
                    keepalive_failed = true;
                    break;
                }
            }

            interval.tick().await;
        }

        if !keepalive_failed {
            return;
        }

        {
            let mut guard = keepalive_registry.lock().await;
            guard.unregister(&keepalive_session_id);
        }
        let mut diag = keepalive_diagnostics.lock().await;
        diag.evictions_total += 1;
        diag.keepalive_failures_total += 1;
        diag.last_eviction = Some(json!({
            "session_id": keepalive_session_id,
            "reason": "bridge keepalive failed",
            "at_ms": now_epoch_ms(),
        }));
    });

    loop {
        let frame = tokio::select! {
            _ = read_close_rx.changed() => {
                break;
            }
            result = read_frame(&mut reader) => {
                match result {
                    Ok(Some(frame)) => frame,
                    Ok(None) => break,
                    Err(e) => {
                        warn!(
                            "[browser-bridge] read error session_id={} err={}",
                            session_id, e
                        );
                        break;
                    }
                }
            }
        };
        if frame.is_empty() {
            continue;
        }
        last_seen_at_ms.store(now_epoch_ms(), Ordering::SeqCst);

        let msg: Value = match serde_json::from_slice(&frame) {
            Ok(v) => v,
            Err(e) => {
                warn!("[browser-bridge] message parse error: {}", e);
                continue;
            }
        };

        if let Some(id_value) = msg.get("id").and_then(id_key) {
            let is_response = msg.get("method").is_none()
                && (msg.get("result").is_some() || msg.get("error").is_some());
            if is_response {
                if let Some(tx) = pending.lock().await.remove(&id_value) {
                    last_ping_ok_at_ms.store(now_epoch_ms(), Ordering::SeqCst);
                    let _ = tx.send(msg);
                    continue;
                }
            }
        }
    }

    {
        let mut guard = registry.lock().await;
        guard.unregister(&session_id);
    }
    let _ = close_tx.send(true);
    keepalive_task.abort();
    drop(writer_tx);
    let _ = writer_task.await;
    info!("[browser-bridge] disconnected session_id={}", session_id);
}

fn validate_bridge_handshake(handshake: &Value, token_path: &Path) -> Result<(), HandshakeFailure> {
    let message_type = handshake.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if message_type != "rzn_browser_bridge_handshake" {
        return Err(HandshakeFailure {
            code: "INVALID_TYPE",
            message: "Unexpected handshake type".to_string(),
        });
    }

    let version = handshake.get("v").and_then(|v| v.as_u64()).unwrap_or(0);
    if version != 1 {
        return Err(HandshakeFailure {
            code: "UNSUPPORTED_VERSION",
            message: format!("Unsupported handshake version {}", version),
        });
    }

    let presented_token = handshake
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if presented_token.is_empty() {
        return Err(HandshakeFailure {
            code: "TOKEN_MISSING",
            message: "Handshake token missing".to_string(),
        });
    }

    let expected_token = match read_token_file(token_path) {
        Ok(Some(token)) => token,
        Ok(None) => {
            return Err(HandshakeFailure {
                code: "TOKEN_EMPTY",
                message: format!("Bridge token missing or empty at {:?}", token_path),
            });
        }
        Err(err) => {
            return Err(HandshakeFailure {
                code: "TOKEN_READ_FAILED",
                message: err,
            });
        }
    };

    if presented_token != expected_token {
        return Err(HandshakeFailure {
            code: "AUTH_FAILED",
            message: "Bridge token mismatch".to_string(),
        });
    }

    Ok(())
}

async fn record_bridge_handshake_failure(
    diagnostics: &Arc<Mutex<BridgeDiagnostics>>,
    failure: HandshakeFailure,
) {
    let mut guard = diagnostics.lock().await;
    guard.handshake_failures_total += 1;
    guard.last_handshake_error = Some(json!({
        "code": failure.code,
        "message": failure.message,
        "at_ms": now_epoch_ms(),
    }));
}

#[derive(Debug, Clone)]
struct BrowserSession {
    session_id: String,
    url: String,
    title: String,
    events: Vec<Value>,
}

#[derive(Clone)]
struct BrowserWorker {
    mock_mode: bool,
    sessions: Arc<Mutex<HashMap<String, BrowserSession>>>,
    bridge: BrowserBridge,
}

impl BrowserWorker {
    fn new(mock_mode: bool, bridge: BrowserBridge) -> Self {
        Self {
            mock_mode,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            bridge,
        }
    }

    fn tool_list(&self) -> Value {
        json!([
            {
                "name": "browser.session_open",
                "description": "Open a browser session via the extension/native host",
                "inputSchema": { "type": "object", "properties": { "url": { "type": "string" } }, "additionalProperties": true }
            },
            {
                "name": "browser.session_close",
                "description": "Close a browser session",
                "inputSchema": { "type": "object", "properties": { "session_id": { "type": "string" } }, "required": ["session_id"] }
            },
            {
                "name": "browser.snapshot",
                "description": "Get a page snapshot for a session",
                "inputSchema": { "type": "object", "properties": { "session_id": { "type": "string" } }, "required": ["session_id"] }
            },
            {
                "name": "browser.execute_step",
                "description": "Execute an action step in the browser session",
                "inputSchema": { "type": "object", "properties": { "session_id": { "type": "string" }, "step": { "type": "object" } }, "required": ["session_id", "step"], "additionalProperties": true }
            },
            {
                "name": "browser.poll_events",
                "description": "Poll for any pending browser events",
                "inputSchema": { "type": "object", "properties": { "session_id": { "type": "string" } }, "required": ["session_id"] }
            },
            {
                "name": "rzn.worker.health",
                "description": "Return worker health diagnostics",
                "inputSchema": { "type": "object", "additionalProperties": false }
            },
            {
                "name": "rzn.worker.shutdown",
                "description": "Request the worker to shut down",
                "inputSchema": { "type": "object", "additionalProperties": false }
            }
        ])
    }

    fn default_timeout_for_step(step_type: Option<&str>) -> Option<u64> {
        match step_type {
            Some("take_screenshot") => Some(SCREENSHOT_TIMEOUT_MS),
            _ => None,
        }
    }

    fn payload_timeout_ms(payload: &Value) -> Option<u64> {
        let step = payload.get("step");
        let explicit = step
            .and_then(|step| {
                step.get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .or_else(|| step.get("timeoutMs").and_then(|v| v.as_u64()))
            })
            .or_else(|| {
                payload
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .or_else(|| payload.get("timeoutMs").and_then(|v| v.as_u64()))
            });
        if explicit.is_some() {
            return explicit;
        }
        let step_type = step
            .and_then(|step| step.get("type"))
            .and_then(|v| v.as_str())
            .or_else(|| payload.get("type").and_then(|v| v.as_str()));
        Self::default_timeout_for_step(step_type)
    }

    fn requested_session_url(params: &Value) -> Option<String> {
        params
            .get("url")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(|url| url.to_string())
    }

    async fn call_browser_session(&self, cmd: &str, payload: Value) -> Result<Value, String> {
        self.call_browser_session_with_timeout(cmd, payload, None)
            .await
    }

    async fn call_browser_session_with_timeout(
        &self,
        cmd: &str,
        payload: Value,
        timeout_override_ms: Option<u64>,
    ) -> Result<Value, String> {
        let request_id = Uuid::new_v4().to_string();
        let req_id = Uuid::new_v4().to_string();
        let timeout_ms = timeout_override_ms.unwrap_or_else(|| {
            Self::payload_timeout_ms(&payload)
                .map(|ms| ms.saturating_add(NATIVE_STEP_TIMEOUT_GRACE_MS).max(ms))
                .unwrap_or(NATIVE_CALL_TIMEOUT_MS)
        });
        let params = json!({
            "cmd": cmd,
            "payload": payload,
            "req_id": req_id,
            "request_id": request_id,
            "timeout_ms": timeout_ms
        });
        self.bridge
            .send_native_request("browser.session", params, Some(timeout_ms), None)
            .await
    }

    async fn health_structured(&self) -> Value {
        let plugin_id = env_trimmed("RZN_PLUGIN_ID").unwrap_or_else(|| "unknown".to_string());
        let worker_id =
            env_trimmed("RZN_PLUGIN_WORKER_ID").unwrap_or_else(|| "browser".to_string());
        let plugin_version =
            env_trimmed("RZN_PLUGIN_VERSION").unwrap_or_else(|| "unknown".to_string());
        let plugin_dir = resolve_plugin_dir();
        let packaged_browser_system = browser_package_inventory(plugin_dir.as_deref());
        let native_host_path = detect_native_host_path();
        let bridge_socket = self.bridge.info.socket_path.clone();
        let bridge_token = self.bridge.info.token_path.clone();
        let browser_session_count = self.sessions.lock().await.len();
        let bridge_host_count = self.bridge.host_count().await;
        let bridge_client_ids = self.bridge.client_ids().await;
        let bridge_sessions = self.bridge.session_snapshots().await;
        let bridge_diagnostics = self.bridge.diagnostics_snapshot().await;

        let mut remediation = Vec::new();
        let native_host_ok = native_host_path
            .as_ref()
            .map(|p| Path::new(p).exists())
            .unwrap_or(false);
        let bridge_socket_exists = bridge_socket.exists();
        let bridge_token_exists = bridge_token.exists();
        if !native_host_ok {
            remediation.push(
                "Install the rzn-browser plugin bundle (missing native host binary)".to_string(),
            );
        }
        if !bridge_socket_exists || !bridge_token_exists {
            remediation
                .push("Restart the rzn-browser worker to recreate bridge endpoints".to_string());
        }
        if plugin_dir.is_some() {
            if !packaged_browser_system
                .get("metadata_exists")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                remediation.push(
                    "Reinstall the rzn-browser plugin bundle to restore browser system metadata"
                        .to_string(),
                );
            }
            if !packaged_browser_system
                .get("examples_exist")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                remediation.push(
                    "Reinstall the rzn-browser plugin bundle to restore packaged browser examples"
                        .to_string(),
                );
            }
        }

        let bridge_connected = self.bridge.has_connection().await;
        let mut native_host_connected = false;
        let mut extension_connected = false;
        let mut ping: Option<Value> = None;
        let mut last_error: Option<String> = None;
        let mut ping_duration_ms: Option<u64> = None;
        if !self.mock_mode && remediation.is_empty() {
            if !bridge_connected {
                remediation.push(
                    "Open Chrome/Edge with the RZN extension enabled so it launches the native host".to_string(),
                );
                remediation.push(
                    "If the extension is installed, re-run the native messaging manifest installer and restart the browser".to_string(),
                );
            } else {
                let started = std::time::Instant::now();
                let ping_payload = json!({
                    "source": "worker_health",
                    "timeout_ms": HEALTH_PING_TIMEOUT_MS
                });
                let mut ping_result = self
                    .call_browser_session_with_timeout(
                        "ping",
                        ping_payload.clone(),
                        Some(HEALTH_PING_TIMEOUT_MS),
                    )
                    .await;
                if ping_result.is_err() && self.bridge.has_connection().await {
                    ping_result = self
                        .call_browser_session_with_timeout(
                            "ping",
                            ping_payload,
                            Some(HEALTH_PING_TIMEOUT_MS),
                        )
                        .await;
                }
                match ping_result {
                    Ok(res) => {
                        native_host_connected = true;
                        extension_connected = true;
                        ping = Some(res);
                        ping_duration_ms = Some(started.elapsed().as_millis() as u64);
                    }
                    Err(e) => {
                        last_error = Some(e.clone());
                        remediation.push(
                            "Open Chrome/Edge with the RZN extension enabled so it connects to the native host".to_string(),
                        );
                        remediation.push(
                            "If the extension is installed, re-run the native messaging manifest installer and restart the browser".to_string(),
                        );
                    }
                }
            }
        }

        let ready = self.mock_mode || native_host_connected;
        json!({
            "ok": ready,
            "id": format!("{}/{}", plugin_id, worker_id),
            "plugin_version": plugin_version,
            "mcp_protocol_version": MCP_PROTOCOL_VERSION,
            "ready": ready,
            "details": {
                "mode": if self.mock_mode { "mock" } else { "native" },
                "native_host_path": native_host_path,
                "browser_bridge_socket_path": bridge_socket.to_string_lossy(),
                "browser_bridge_token_path": bridge_token.to_string_lossy(),
                "browser_bridge_socket_exists": bridge_socket_exists,
                "browser_bridge_token_exists": bridge_token_exists,
                "plugin_dir": plugin_dir.map(|p| p.to_string_lossy().to_string()),
                "packaged_browser_system": packaged_browser_system,
                "bridge_connected": bridge_connected,
                "bridge_host_count": bridge_host_count,
                "bridge_client_ids": bridge_client_ids,
                "bridge_sessions": bridge_sessions,
                "bridge_diagnostics": bridge_diagnostics,
                "native_host_connected": native_host_connected,
                "extension_connected": extension_connected,
                "browser_session_count": browser_session_count,
                "ping_duration_ms": ping_duration_ms,
                "checked_at_ms": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|d| d.as_millis() as u64),
                "ping": ping,
                "last_error": last_error,
                "remediation": remediation
            }
        })
    }

    async fn session_open(&self, params: Value) -> Value {
        let requested_url = Self::requested_session_url(&params);
        let session_url = requested_url.clone().unwrap_or_default();
        if self.mock_mode {
            let session_id = Uuid::new_v4().to_string();
            let session = BrowserSession {
                session_id: session_id.clone(),
                url: session_url.clone(),
                title: "Mock Browser".to_string(),
                events: vec![json!({"type": "info", "message": "mock session created"})],
            };
            let mut sessions = self.sessions.lock().await;
            sessions.insert(session_id.clone(), session);
            return build_tool_result(
                "ok".to_string(),
                json!({ "ok": true, "session_id": session_id, "url": session_url }),
                false,
                HashMap::new(),
            );
        }
        let session_id = Uuid::new_v4().to_string();
        {
            let mut sessions = self.sessions.lock().await;
            sessions.insert(
                session_id.clone(),
                BrowserSession {
                    session_id: session_id.clone(),
                    url: session_url.clone(),
                    title: "Browser".to_string(),
                    events: Vec::new(),
                },
            );
        }

        if let Some(url) = requested_url {
            let step = json!({ "type": "navigate_to_url", "url": url });
            match self
                .call_browser_session(
                    "execute_step",
                    json!({ "session_id": session_id, "step": step }),
                )
                .await
            {
                Ok(res) => build_tool_result(
                    "ok".to_string(),
                    json!({ "ok": true, "session_id": session_id, "url": session_url, "result": res }),
                    false,
                    HashMap::new(),
                ),
                Err(e) => build_tool_result(
                    "session_open failed".to_string(),
                    json!({ "ok": false, "error": e, "session_id": session_id, "url": session_url }),
                    true,
                    HashMap::new(),
                ),
            }
        } else {
            build_tool_result(
                "ok".to_string(),
                json!({ "ok": true, "session_id": session_id, "url": session_url }),
                false,
                HashMap::new(),
            )
        }
    }

    async fn session_close(&self, params: Value) -> Value {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if self.mock_mode {
            let mut sessions = self.sessions.lock().await;
            sessions.remove(&session_id);
            return build_tool_result(
                "ok".to_string(),
                json!({ "ok": true, "session_id": session_id }),
                false,
                HashMap::new(),
            );
        }
        let mut sessions = self.sessions.lock().await;
        sessions.remove(&session_id);
        build_tool_result(
            "ok".to_string(),
            json!({ "ok": true, "session_id": session_id }),
            false,
            HashMap::new(),
        )
    }

    async fn snapshot(&self, params: Value) -> Value {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if self.mock_mode {
            let sessions = self.sessions.lock().await;
            let session = sessions.get(&session_id);
            let title = session
                .map(|s| s.title.clone())
                .unwrap_or_else(|| "Mock Page".to_string());
            return build_tool_result(
                "ok".to_string(),
                json!({ "ok": true, "session_id": session_id, "title": title, "html": "<html><body><p>mock snapshot</p></body></html>" }),
                false,
                HashMap::new(),
            );
        }
        match self
            .call_browser_session("get_dom_snapshot", json!({ "session_id": session_id }))
            .await
        {
            Ok(res) => build_tool_result(
                "ok".to_string(),
                json!({ "ok": true, "session_id": session_id, "result": res }),
                false,
                HashMap::new(),
            ),
            Err(e) => build_tool_result(
                "snapshot failed".to_string(),
                json!({ "ok": false, "error": e, "session_id": session_id }),
                true,
                HashMap::new(),
            ),
        }
    }

    async fn execute_step(&self, params: Value) -> Value {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let step = params.get("step").cloned().unwrap_or_else(|| json!({}));
        if self.mock_mode {
            let mut sessions = self.sessions.lock().await;
            if let Some(session) = sessions.get_mut(&session_id) {
                session
                    .events
                    .push(json!({ "type": "debug", "message": "mock execute_step", "step": step }));
            }
            return build_tool_result(
                "ok".to_string(),
                json!({ "ok": true, "session_id": session_id, "step": step }),
                false,
                HashMap::new(),
            );
        }
        let step_type = step
            .get("type")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let is_eval_step = matches!(
            step_type.as_str(),
            "execute_javascript" | "eval_main_world" | "eval_isolated_world"
        );
        if is_eval_step {
            let world = match step_type.as_str() {
                "eval_main_world" => Some("main"),
                "eval_isolated_world" => Some("isolated"),
                _ => step.get("world").and_then(|v| v.as_str()),
            };
            let payload = BrowserWorker::build_eval_payload(&params, &session_id, &step, world);
            return match self.call_browser_session("eval_with_cdp", payload).await {
                Ok(res) => build_tool_result(
                    "ok".to_string(),
                    json!({ "ok": true, "session_id": session_id, "result": res }),
                    false,
                    HashMap::new(),
                ),
                Err(e) => build_tool_result(
                    "execute_step failed".to_string(),
                    json!({ "ok": false, "error": e, "session_id": session_id }),
                    true,
                    HashMap::new(),
                ),
            };
        }
        let payload = BrowserWorker::build_step_payload(&params, &session_id, &step);
        match self.call_browser_session("execute_step", payload).await {
            Ok(res) => build_tool_result(
                "ok".to_string(),
                json!({ "ok": true, "session_id": session_id, "result": res }),
                false,
                HashMap::new(),
            ),
            Err(e) => build_tool_result(
                "execute_step failed".to_string(),
                json!({ "ok": false, "error": e, "session_id": session_id }),
                true,
                HashMap::new(),
            ),
        }
    }

    fn build_eval_payload(
        params: &Value,
        session_id: &str,
        step: &Value,
        world: Option<&str>,
    ) -> Value {
        let mut payload = json!({
            "session_id": session_id,
            "script": step.get("script").cloned().unwrap_or(Value::String(String::new())),
            "args": step.get("args").cloned().unwrap_or(Value::Array(Vec::new())),
            "params": step.get("params").cloned().unwrap_or_else(|| json!({})),
            "return_value": step.get("return_value").and_then(|v| v.as_bool()).unwrap_or(true),
            "world": world,
            "timeout_ms": step
                .get("timeout_ms")
                .cloned()
                .or_else(|| step.get("timeoutMs").cloned())
                .unwrap_or(Value::Null),
        });

        let prefer_current_tab = step
            .get("use_current_tab")
            .and_then(|v| v.as_bool())
            .or_else(|| params.get("use_current_tab").and_then(|v| v.as_bool()))
            .unwrap_or(false);
        if prefer_current_tab {
            payload["use_current_tab"] = Value::Bool(true);
        }
        let prefer_active_tab = step
            .get("use_active_tab")
            .and_then(|v| v.as_bool())
            .or_else(|| params.get("use_active_tab").and_then(|v| v.as_bool()))
            .unwrap_or(false);
        if prefer_active_tab {
            payload["use_active_tab"] = Value::Bool(true);
        }

        payload
    }

    fn build_step_payload(params: &Value, session_id: &str, step: &Value) -> Value {
        let mut payload = json!({
            "session_id": session_id,
            "step": step,
        });

        let prefer_current_tab = step
            .get("use_current_tab")
            .and_then(|v| v.as_bool())
            .or_else(|| params.get("use_current_tab").and_then(|v| v.as_bool()))
            .unwrap_or(false);
        if prefer_current_tab {
            payload["use_current_tab"] = Value::Bool(true);
        }

        let prefer_active_tab = step
            .get("use_active_tab")
            .and_then(|v| v.as_bool())
            .or_else(|| params.get("use_active_tab").and_then(|v| v.as_bool()))
            .unwrap_or(false);
        if prefer_active_tab {
            payload["use_active_tab"] = Value::Bool(true);
        }

        payload
    }

    async fn poll_events(&self, params: Value) -> Value {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mut sessions = self.sessions.lock().await;
        let events = if let Some(session) = sessions.get_mut(&session_id) {
            std::mem::take(&mut session.events)
        } else {
            Vec::new()
        };
        build_tool_result(
            "ok".to_string(),
            json!({ "ok": true, "events": events, "session_id": session_id }),
            false,
            HashMap::new(),
        )
    }
}

async fn process_mcp_request(
    worker: &BrowserWorker,
    shutdown: &AtomicBool,
    request: Value,
) -> Option<Value> {
    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = request.get("id").cloned();
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));

    let response = match method {
        "initialize" => Some(jsonrpc_result(
            id.unwrap_or(Value::Null),
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "serverInfo": { "name": "rzn-browser-worker", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "tools": { "listChanged": false } }
            }),
        )),
        "tools/list" => Some(jsonrpc_result(
            id.unwrap_or(Value::Null),
            json!({ "tools": worker.tool_list() }),
        )),
        "tools/call" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let result = match name {
                "browser.session_open" => worker.session_open(args).await,
                "browser.session_close" => worker.session_close(args).await,
                "browser.snapshot" => worker.snapshot(args).await,
                "browser.execute_step" => worker.execute_step(args).await,
                "browser.poll_events" => worker.poll_events(args).await,
                "rzn.worker.health" => build_tool_result(
                    "ok".to_string(),
                    worker.health_structured().await,
                    false,
                    HashMap::new(),
                ),
                "rzn.worker.shutdown" => {
                    shutdown.store(true, Ordering::SeqCst);
                    build_tool_result(
                        "ok".to_string(),
                        json!({ "ok": true }),
                        false,
                        HashMap::new(),
                    )
                }
                _ => build_tool_result(
                    "unknown tool".to_string(),
                    json!({ "ok": false, "error": format!("unknown tool: {}", name) }),
                    true,
                    HashMap::new(),
                ),
            };
            Some(jsonrpc_result(id.unwrap_or(Value::Null), result))
        }
        _ => Some(jsonrpc_error(
            id,
            -32601,
            &format!("Method not found: {}", method),
        )),
    };

    Some(response?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mock_mode = env_trimmed("RZN_BROWSER_WORKER_MOCK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let app_base = resolve_app_base_dir();
    let registry = Arc::new(Mutex::new(NativeHostRegistry::default()));
    let (bridge_info, diagnostics) = start_browser_bridge(&app_base, registry.clone())
        .await
        .map_err(|e| format!("bridge start failed: {}", e))?;
    let bridge_cleanup_info = bridge_info.clone();
    let bridge = BrowserBridge {
        info: bridge_info,
        registry,
        diagnostics,
    };
    let worker = BrowserWorker::new(mock_mode, bridge);
    let shutdown = Arc::new(AtomicBool::new(false));
    let control_info = start_worker_control(&app_base, worker.clone(), shutdown.clone())
        .await
        .map_err(|e| format!("worker control start failed: {}", e))?;

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();

    let mut line = String::new();
    let mut stdin_closed = false;
    loop {
        if stdin_closed {
            if shutdown.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        }

        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            info!("[browser-worker] stdin closed; keeping socket worker alive until shutdown");
            stdin_closed = true;
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let resp = jsonrpc_error(None, -32700, &format!("Parse error: {}", e));
                stdout
                    .write_all(serde_json::to_string(&resp)?.as_bytes())
                    .await?;
                stdout.write_all(b"\n").await?;
                stdout.flush().await?;
                continue;
            }
        };

        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        if let Some(resp) = process_mcp_request(&worker, &shutdown, request).await {
            stdout
                .write_all(serde_json::to_string(&resp)?.as_bytes())
                .await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }

    cleanup_runtime_artifacts(&app_base, &bridge_cleanup_info, &control_info);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Result<Self, String> {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), unique));
            fs::create_dir_all(&path).map_err(|e| format!("create temp dir {:?}: {}", path, e))?;
            Ok(Self { path })
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_file(path: &Path, contents: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, contents).expect("write file");
    }

    #[test]
    fn browser_package_inventory_reports_packaged_metadata_and_examples() {
        let temp = TempDirGuard::new("rzn-browser-worker-package").expect("temp dir");
        write_file(
            &temp.path.join(BROWSER_SYSTEM_METADATA_RELATIVE_PATH),
            b"version: 1\nsystem:\n  id: browser_automation\n",
        );
        write_file(
            &temp
                .path
                .join("examples/browser_automation/open_page_get_title.json"),
            br#"{"id":"open_page_get_title"}"#,
        );
        write_file(
            &temp.path.join("examples/browser_automation/README.md"),
            b"# Browser examples\n",
        );

        let inventory = browser_package_inventory(Some(&temp.path));

        assert_eq!(inventory["configured"], Value::Bool(true));
        assert_eq!(
            inventory["system_id"],
            Value::String(BROWSER_SYSTEM_ID.to_string())
        );
        assert_eq!(inventory["metadata_exists"], Value::Bool(true));
        assert_eq!(inventory["examples_exist"], Value::Bool(true));
        assert_eq!(inventory["example_count"], Value::from(2));
        assert_eq!(
            inventory["example_files"],
            json!([
                "examples/browser_automation/README.md",
                "examples/browser_automation/open_page_get_title.json"
            ])
        );
    }

    #[test]
    fn browser_package_inventory_handles_missing_plugin_dir() {
        let inventory = browser_package_inventory(None);

        assert_eq!(inventory["configured"], Value::Bool(false));
        assert_eq!(
            inventory["system_id"],
            Value::String(BROWSER_SYSTEM_ID.to_string())
        );
        assert_eq!(inventory["metadata_exists"], Value::Bool(false));
        assert_eq!(inventory["examples_exist"], Value::Bool(false));
        assert_eq!(inventory["example_count"], Value::from(0));
        assert_eq!(inventory["example_files"], json!([]));
    }

    #[test]
    fn parse_native_host_path_from_manifest_extracts_path() {
        let path = parse_native_host_path_from_manifest(
            r#"{"name":"com.rzn.browser.broker","path":"/tmp/rzn-native-host","type":"stdio"}"#,
        );

        assert_eq!(path, Some("/tmp/rzn-native-host".to_string()));
    }

    #[test]
    fn payload_timeout_ms_uses_screenshot_default_when_unspecified() {
        let payload = json!({
            "step": {
                "type": "take_screenshot",
                "format": "png"
            }
        });

        assert_eq!(
            BrowserWorker::payload_timeout_ms(&payload),
            Some(SCREENSHOT_TIMEOUT_MS)
        );
    }

    #[test]
    fn payload_timeout_ms_prefers_explicit_timeout_over_step_default() {
        let payload = json!({
            "step": {
                "type": "take_screenshot",
                "timeout_ms": 12345
            }
        });

        assert_eq!(BrowserWorker::payload_timeout_ms(&payload), Some(12345));
    }

    #[test]
    fn build_eval_payload_preserves_params_for_eval_steps() {
        let step = json!({
            "type": "execute_javascript",
            "script": "return window.__rzn_params.message_text;",
            "params": {
                "message_text": "PONG",
                "model_slug": "GPT-5"
            },
            "timeout_ms": 9876
        });

        let payload =
            BrowserWorker::build_eval_payload(&json!({}), "session-1", &step, Some("main"));

        assert_eq!(
            payload["session_id"],
            Value::String("session-1".to_string())
        );
        assert_eq!(
            payload["params"],
            json!({
                "message_text": "PONG",
                "model_slug": "GPT-5"
            })
        );
        assert_eq!(payload["world"], Value::String("main".to_string()));
        assert_eq!(payload["timeout_ms"], Value::from(9876));
    }

    #[test]
    fn build_eval_payload_preserves_current_tab_flags() {
        let step = json!({
            "type": "execute_javascript",
            "script": "document.title",
            "use_current_tab": true,
            "use_active_tab": true
        });

        let payload =
            BrowserWorker::build_eval_payload(&json!({}), "session-1", &step, Some("main"));

        assert_eq!(payload["use_current_tab"], Value::Bool(true));
        assert_eq!(payload["use_active_tab"], Value::Bool(true));
    }

    #[test]
    fn build_eval_payload_preserves_top_level_current_tab_flags() {
        let step = json!({
            "type": "execute_javascript",
            "script": "document.title"
        });
        let payload = BrowserWorker::build_eval_payload(
            &json!({
                "use_current_tab": true,
                "use_active_tab": true
            }),
            "session-1",
            &step,
            Some("main"),
        );

        assert_eq!(payload["use_current_tab"], Value::Bool(true));
        assert_eq!(payload["use_active_tab"], Value::Bool(true));
    }

    #[test]
    fn build_step_payload_preserves_current_tab_flags() {
        let step = json!({
            "type": "wait_for_element",
            "selector": "#prompt-textarea",
            "use_current_tab": true,
            "use_active_tab": true
        });

        let payload = BrowserWorker::build_step_payload(&json!({}), "session-1", &step);

        assert_eq!(
            payload["session_id"],
            Value::String("session-1".to_string())
        );
        assert_eq!(payload["step"], step);
        assert_eq!(payload["use_current_tab"], Value::Bool(true));
        assert_eq!(payload["use_active_tab"], Value::Bool(true));
    }

    #[test]
    fn build_step_payload_preserves_top_level_current_tab_flags() {
        let step = json!({
            "type": "wait_for_element",
            "selector": "#prompt-textarea"
        });

        let payload = BrowserWorker::build_step_payload(
            &json!({
                "use_current_tab": true,
                "use_active_tab": true
            }),
            "session-1",
            &step,
        );

        assert_eq!(payload["use_current_tab"], Value::Bool(true));
        assert_eq!(payload["use_active_tab"], Value::Bool(true));
    }

    #[test]
    fn validate_bridge_handshake_accepts_rotated_token_file() {
        let temp = TempDirGuard::new("rzn-browser-worker-bridge-token").expect("temp dir");
        let token_path = temp.path.join("browser_bridge_token_v1");
        write_file(&token_path, b"token-one\n");

        let first = json!({
            "type": "rzn_browser_bridge_handshake",
            "v": 1,
            "token": "token-one"
        });
        assert!(validate_bridge_handshake(&first, &token_path).is_ok());

        write_file(&token_path, b"token-two\n");
        let rotated = json!({
            "type": "rzn_browser_bridge_handshake",
            "v": 1,
            "token": "token-two"
        });
        assert!(validate_bridge_handshake(&rotated, &token_path).is_ok());
    }

    #[test]
    fn validate_bridge_handshake_rejects_stale_token_after_rotation() {
        let temp = TempDirGuard::new("rzn-browser-worker-bridge-stale").expect("temp dir");
        let token_path = temp.path.join("browser_bridge_token_v1");
        write_file(&token_path, b"fresh-token\n");

        let stale = json!({
            "type": "rzn_browser_bridge_handshake",
            "v": 1,
            "token": "stale-token"
        });
        let failure = validate_bridge_handshake(&stale, &token_path).expect_err("stale token");

        assert_eq!(failure.code, "AUTH_FAILED");
    }

    #[test]
    fn requested_session_url_ignores_missing_or_blank_values() {
        assert_eq!(BrowserWorker::requested_session_url(&json!({})), None);
        assert_eq!(
            BrowserWorker::requested_session_url(&json!({ "url": "   " })),
            None
        );
        assert_eq!(
            BrowserWorker::requested_session_url(&json!({ "url": "https://google.com" })),
            Some("https://google.com".to_string())
        );
    }

    #[test]
    fn bridge_keepalive_requires_successful_response() {
        assert!(bridge_keepalive_response_is_healthy(&json!({
            "jsonrpc": "2.0",
            "id": "1",
            "result": { "success": true }
        })));
        assert!(!bridge_keepalive_response_is_healthy(&json!({
            "jsonrpc": "2.0",
            "id": "1",
            "result": { "success": false }
        })));
        assert!(!bridge_keepalive_response_is_healthy(&json!({
            "jsonrpc": "2.0",
            "id": "1",
            "error": { "code": -32003, "message": "Extension timeout" }
        })));
    }
}
