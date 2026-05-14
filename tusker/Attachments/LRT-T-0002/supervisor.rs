use anyhow::{anyhow, Context, Result};
use interprocess::local_socket::{
    tokio::Stream as LocalSocketStream,
    traits::tokio::{Listener as _, Stream as _},
    GenericFilePath, ListenerOptions, ToFsName,
};
use rzn_broker_endpoint::prune_stale_broker_endpoint;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::{timeout, Duration};
use uuid::Uuid;

use crate::native_runner::{self, NativeClient, NativeRunConfig, NativeRunMode, SnapshotMode};

pub(crate) const RZN_LOCAL_PROTOCOL_VERSION: &str = "rzn.local.v1";
const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;
const HANDSHAKE_TIMEOUT_MS: u64 = 2_000;
const REQUEST_TIMEOUT_MS: u64 = 30_000;
const EXTENSION_BRIDGE_TIMEOUT_MS: u64 = 20_000;
const SUPERVISOR_SOCKET_FILENAME: &str = "rzn-supervisor.sock";
const SUPERVISOR_TOKEN_FILENAME: &str = "rzn-supervisor-token-v1";

#[derive(Clone, Debug)]
pub(crate) struct SupervisorConfig {
    pub app_base: Option<PathBuf>,
    pub endpoint_path: Option<String>,
    pub mode: NativeRunMode,
    pub worker_cmd: Option<String>,
    pub worker_args: Vec<String>,
    pub allow_legacy_worker_fallback: bool,
}

impl SupervisorConfig {
    pub(crate) fn app_base_dir(&self) -> PathBuf {
        self.app_base
            .clone()
            .or_else(|| env_path("RZN_SUPERVISOR_APP_BASE"))
            .or_else(|| env_path("RZN_NATIVE_APP_BASE"))
            .or_else(|| env_path("RZN_APP_BASE"))
            .or_else(|| env_path("APP_BASE"))
            .unwrap_or_else(default_app_base_dir)
    }

    fn native_run_config(&self) -> NativeRunConfig {
        NativeRunConfig {
            workflow_path: String::new(),
            params: HashMap::new(),
            mode: self.mode,
            snapshot_mode: SnapshotMode::OnError,
            app_base: Some(self.app_base_dir().to_string_lossy().to_string()),
            endpoint_path: self.endpoint_path.clone(),
            worker_cmd: self.worker_cmd.clone(),
            worker_args: self.worker_args.clone(),
        }
    }

    fn allows_legacy_worker_fallback(&self) -> bool {
        self.allow_legacy_worker_fallback || env_flag("RZN_SUPERVISOR_ALLOW_LEGACY_WORKER_FALLBACK")
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SupervisorPaths {
    pub app_base: PathBuf,
    pub secure_dir: PathBuf,
    pub run_dir: PathBuf,
    pub socket_path: PathBuf,
    pub token_path: PathBuf,
}

impl SupervisorPaths {
    pub(crate) fn for_config(config: &SupervisorConfig) -> Self {
        let app_base = config.app_base_dir();
        let secure_dir = app_base.join("secure");
        let run_dir = app_base.join("run");
        Self {
            app_base,
            socket_path: run_dir.join(SUPERVISOR_SOCKET_FILENAME),
            token_path: secure_dir.join(SUPERVISOR_TOKEN_FILENAME),
            secure_dir,
            run_dir,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct SupervisorServeReport {
    pub ok: bool,
    pub protocol: &'static str,
    pub pid: u32,
    pub app_base: String,
    pub socket_path: String,
    pub token_path: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SupervisorClientStatus {
    pub ok: bool,
    pub result: Value,
}

struct SupervisorState {
    config: SupervisorConfig,
    paths: SupervisorPaths,
    browser_client: Mutex<Option<NativeClient>>,
    native_bridge: Mutex<Option<NativeHostBridge>>,
    native_bridge_pending: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    sessions: Mutex<HashSet<String>>,
    shutdown: AtomicBool,
}

#[derive(Clone)]
struct NativeHostBridge {
    id: String,
    tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl SupervisorState {
    fn new(config: SupervisorConfig) -> Self {
        let paths = SupervisorPaths::for_config(&config);
        Self {
            config,
            paths,
            browser_client: Mutex::new(None),
            native_bridge: Mutex::new(None),
            native_bridge_pending: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashSet::new()),
            shutdown: AtomicBool::new(false),
        }
    }

    async fn dispatch(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "runtime.hello" | "runtime.status" => Ok(self.runtime_status(false).await),
            "runtime.ensure_ready" => self.ensure_ready(params).await,
            "runtime.heal" => self.runtime_heal().await,
            "runtime.shutdown" => {
                self.shutdown.store(true, Ordering::SeqCst);
                Ok(json!({ "ok": true, "shutdown": true }))
            }
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| anyhow!("tools/call missing params.name"))?;
                let args = tool_call_arguments(&params);
                self.dispatch_tool(name, args).await
            }
            name if is_browser_tool(name) => self.dispatch_tool(name, params).await,
            _ => Err(anyhow!("Unknown supervisor method: {}", method)),
        }
    }

    async fn dispatch_tool(&self, name: &str, params: Value) -> Result<Value> {
        match name {
            "rzn.supervisor.health" => Ok(self.runtime_status(false).await),
            "rzn.worker.health" => {
                let worker = self
                    .call_browser_worker("rzn.worker.health", params, None)
                    .await?;
                Ok(json!({
                    "ok": true,
                    "supervisor": self.runtime_status(false).await,
                    "worker": worker
                }))
            }
            "rzn.worker.shutdown" => {
                let mut guard = self.browser_client.lock().await;
                let had_worker = guard.is_some();
                if let Some(client) = guard.as_mut() {
                    client.shutdown().await;
                }
                *guard = None;
                Ok(json!({
                    "ok": true,
                    "legacy_worker_shutdown": had_worker,
                    "supervisor_shutdown": false
                }))
            }
            "browser.session_open"
            | "browser.session_close"
            | "browser.snapshot"
            | "browser.execute_step"
            | "browser.poll_events" => {
                if let Some(value) = self
                    .try_dispatch_supervisor_browser_tool(name, params.clone())
                    .await?
                {
                    Ok(value)
                } else if self.config.allows_legacy_worker_fallback() {
                    self.call_browser_worker(name, params, None).await
                } else {
                    Err(self.native_bridge_required_error(name))
                }
            }
            _ => Err(anyhow!("Unknown supervisor tool: {}", name)),
        }
    }

    async fn runtime_status(&self, include_worker: bool) -> Value {
        let legacy_worker_fallback_allowed = self.config.allows_legacy_worker_fallback();
        let mut status = json!({
            "ok": true,
            "protocol": RZN_LOCAL_PROTOCOL_VERSION,
            "pid": std::process::id(),
            "app_base": self.paths.app_base.to_string_lossy(),
            "socket_path": self.paths.socket_path.to_string_lossy(),
            "token_path": self.paths.token_path.to_string_lossy(),
            "browser_proxy": {
                "mode": if legacy_worker_fallback_allowed {
                    "native_host_bridge_preferred_legacy_worker_fallback"
                } else {
                    "native_host_bridge_required"
                },
                "authority": "supervisor_handshake",
                "legacy_worker_fallback_allowed": legacy_worker_fallback_allowed
            },
            "native_host_bridge": {
                "connected": self.native_bridge.lock().await.is_some()
            }
        });

        if include_worker {
            let worker = self
                .call_browser_worker("rzn.worker.health", json!({}), Some(2_500))
                .await;
            status["worker"] = match worker {
                Ok(value) => value,
                Err(err) => json!({
                    "ok": false,
                    "error": err.to_string()
                }),
            };
        }

        status
    }

    async fn ensure_ready(&self, params: Value) -> Result<Value> {
        let prune = prune_stale_broker_endpoint(&self.paths.app_base).ok();
        let wait_ms = params
            .get("bridge_wait_ms")
            .and_then(|value| value.as_u64())
            .or_else(|| env_u64("RZN_SUPERVISOR_BRIDGE_WAIT_MS"))
            .unwrap_or(2_500);
        let native_host_bridge_connected = self.wait_for_native_bridge(wait_ms).await;
        let legacy_worker_fallback_allowed = self.config.allows_legacy_worker_fallback();
        let worker = if !native_host_bridge_connected && legacy_worker_fallback_allowed {
            Some(
                self.call_browser_worker("rzn.worker.health", json!({}), Some(5_000))
                    .await,
            )
        } else {
            None
        };
        let worker_ready = worker.as_ref().map(Result::is_ok).unwrap_or(false);
        let ready =
            native_host_bridge_connected || (legacy_worker_fallback_allowed && worker_ready);
        Ok(json!({
            "ok": ready,
            "ready": ready,
            "protocol": RZN_LOCAL_PROTOCOL_VERSION,
            "pid": std::process::id(),
            "app_base": self.paths.app_base.to_string_lossy(),
            "browser_proxy": {
                "mode": if legacy_worker_fallback_allowed {
                    "native_host_bridge_preferred_legacy_worker_fallback"
                } else {
                    "native_host_bridge_required"
                },
                "legacy_worker_fallback_allowed": legacy_worker_fallback_allowed
            },
            "native_host_bridge": {
                "connected": native_host_bridge_connected,
                "wait_ms": wait_ms
            },
            "prune": prune,
            "worker": match worker {
                Some(Ok(value)) => value,
                Some(Err(err)) => json!({ "ok": false, "error": err.to_string() }),
                None => json!({
                    "ok": native_host_bridge_connected,
                    "skipped": true,
                    "reason": if native_host_bridge_connected {
                        "native_host_bridge_connected"
                    } else {
                        "legacy_worker_fallback_disabled"
                    }
                }),
            },
            "error": if ready {
                Value::Null
            } else {
                json!("Native-host bridge is not connected. Open Chrome with the RZN extension enabled, then retry. Use --allow-legacy-worker-fallback only for compatibility debugging.")
            },
            "remediation": if ready {
                json!([])
            } else {
                json!([
                    "Open the existing Chrome profile with the RZN extension enabled.",
                    "Reload the extension if Chrome has suspended or restarted the service worker.",
                    "Run `rzn-browser supervisor status --json` and confirm native_host_bridge.connected is true."
                ])
            }
        }))
    }

    async fn wait_for_native_bridge(&self, wait_ms: u64) -> bool {
        if self.native_bridge.lock().await.is_some() {
            return true;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms);
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if self.native_bridge.lock().await.is_some() {
                return true;
            }
        }
        false
    }

    fn native_bridge_required_error(&self, tool_name: &str) -> anyhow::Error {
        anyhow!(
            "Browser tool '{}' requires the supervisor native-host bridge, but no bridge is connected. Open Chrome with the RZN extension enabled, reload the extension if needed, then retry. Legacy worker fallback is disabled on the supervisor default path.",
            tool_name
        )
    }

    async fn runtime_heal(&self) -> Result<Value> {
        let prune = prune_stale_broker_endpoint(&self.paths.app_base)
            .map_err(|err| anyhow!("Prune stale broker endpoint failed: {}", err))?;
        let status = self.runtime_status(true).await;
        Ok(json!({
            "ok": true,
            "protocol": RZN_LOCAL_PROTOCOL_VERSION,
            "prune": prune,
            "status": status
        }))
    }

    async fn call_browser_worker(
        &self,
        name: &str,
        params: Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value> {
        let mut guard = self.browser_client.lock().await;
        if guard.is_none() {
            let client = native_runner::connect_native(&self.config.native_run_config()).await?;
            *guard = Some(client);
        }

        let client = guard
            .as_mut()
            .ok_or_else(|| anyhow!("Browser client unavailable after connect"))?;

        let result = match timeout_ms {
            Some(ms) => client.send_request_with_timeout(name, params, ms).await,
            None => client.send_request(name, params).await,
        };

        if result.is_err() {
            *guard = None;
        }

        result
    }

    async fn try_dispatch_supervisor_browser_tool(
        &self,
        name: &str,
        params: Value,
    ) -> Result<Option<Value>> {
        match name {
            "browser.session_open" => self.try_session_open(params).await,
            "browser.session_close" => self.try_session_close(params).await,
            "browser.poll_events" => self.try_poll_events(params).await,
            "browser.snapshot" => {
                self.try_call_native_bridge("get_dom_snapshot", params)
                    .await
            }
            "browser.execute_step" => self.try_call_native_bridge("execute_step", params).await,
            _ => Ok(None),
        }
    }

    async fn try_session_open(&self, params: Value) -> Result<Option<Value>> {
        if self.native_bridge.lock().await.is_none() {
            return Ok(None);
        }

        let session_id = Uuid::new_v4().to_string();
        self.sessions.lock().await.insert(session_id.clone());
        let requested_url = params
            .get("url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Some(url) = requested_url.clone() {
            let payload = json!({
                "session_id": session_id,
                "step": { "type": "navigate_to_url", "url": url }
            });
            if let Some(result) = self.try_call_native_bridge("execute_step", payload).await? {
                return Ok(Some(json!({
                    "ok": true,
                    "session_id": session_id,
                    "url": requested_url.unwrap_or_default(),
                    "result": result
                })));
            }
            self.sessions.lock().await.remove(&session_id);
            return Ok(None);
        }

        Ok(Some(json!({
            "ok": true,
            "session_id": session_id,
            "url": ""
        })))
    }

    async fn try_session_close(&self, params: Value) -> Result<Option<Value>> {
        let session_id = params
            .get("session_id")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        if session_id.is_empty() {
            return Ok(Some(
                json!({ "ok": false, "error": "session_id is required" }),
            ));
        }
        self.sessions.lock().await.remove(&session_id);
        Ok(Some(json!({ "ok": true, "session_id": session_id })))
    }

    async fn try_poll_events(&self, params: Value) -> Result<Option<Value>> {
        let session_id = params
            .get("session_id")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        Ok(Some(json!({
            "ok": true,
            "session_id": session_id,
            "events": []
        })))
    }

    async fn try_call_native_bridge(&self, cmd: &str, params: Value) -> Result<Option<Value>> {
        let bridge = self.native_bridge.lock().await.clone();
        let Some(bridge) = bridge else {
            return Ok(None);
        };

        let id = format!("native-call-{}", Uuid::new_v4());
        let req_id = format!("supervisor-{}", Uuid::new_v4());
        let timeout_ms = extension_timeout_ms(&params);
        let (tx, rx) = oneshot::channel();
        self.native_bridge_pending
            .lock()
            .await
            .insert(id.clone(), tx);

        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "native_host.extension_call",
            "params": {
                "cmd": cmd,
                "payload": params,
                "req_id": req_id,
                "timeout_ms": timeout_ms
            }
        });
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .unwrap_or_default();
        let bytes = serde_json::to_vec(&request)?;
        if bridge.tx.send(bytes).is_err() {
            self.native_bridge_pending.lock().await.remove(&request_id);
            self.clear_native_bridge(&bridge.id).await;
            return Ok(None);
        }

        let response = match timeout(Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                self.native_bridge_pending.lock().await.remove(&request_id);
                self.clear_native_bridge(&bridge.id).await;
                return Ok(None);
            }
            Err(_) => {
                self.native_bridge_pending.lock().await.remove(&request_id);
                return Err(anyhow!(
                    "Native-host extension bridge timeout after {}ms",
                    timeout_ms
                ));
            }
        };

        if let Some(error) = response.get("error") {
            return Err(anyhow!("Native-host extension bridge error: {}", error));
        }
        Ok(Some(response.get("result").cloned().unwrap_or(response)))
    }

    async fn register_native_bridge(&self, id: String, tx: mpsc::UnboundedSender<Vec<u8>>) {
        *self.native_bridge.lock().await = Some(NativeHostBridge { id, tx });
    }

    async fn clear_native_bridge(&self, id: &str) {
        let mut guard = self.native_bridge.lock().await;
        if guard.as_ref().map(|bridge| bridge.id.as_str()) == Some(id) {
            *guard = None;
        }
    }

    async fn complete_native_bridge_response(&self, value: &Value) -> bool {
        let Some(id) = value.get("id").and_then(value_id_string) else {
            return false;
        };
        let tx = self.native_bridge_pending.lock().await.remove(&id);
        if let Some(tx) = tx {
            let _ = tx.send(value.clone());
            true
        } else {
            false
        }
    }
}

pub(crate) async fn serve(config: SupervisorConfig) -> Result<SupervisorServeReport> {
    let state = Arc::new(SupervisorState::new(config));
    prepare_paths(&state.paths)?;

    if state.paths.socket_path.exists() {
        match SupervisorClient::connect_with_paths(state.paths.clone()).await {
            Ok(mut client) => {
                let status = client.call("runtime.status", json!({})).await?;
                return Err(anyhow!(
                    "Supervisor already running at {}: {}",
                    state.paths.socket_path.display(),
                    status
                ));
            }
            Err(_) => {
                let _ = std::fs::remove_file(&state.paths.socket_path);
            }
        }
    }

    let name = state
        .paths
        .socket_path
        .clone()
        .to_fs_name::<GenericFilePath>()
        .with_context(|| {
            format!(
                "Invalid supervisor socket path {}",
                state.paths.socket_path.display()
            )
        })?;
    let listener = ListenerOptions::new()
        .name(name)
        .create_tokio()
        .with_context(|| {
            format!(
                "Failed to bind supervisor socket {}",
                state.paths.socket_path.display()
            )
        })?;

    let report = SupervisorServeReport {
        ok: true,
        protocol: RZN_LOCAL_PROTOCOL_VERSION,
        pid: std::process::id(),
        app_base: state.paths.app_base.to_string_lossy().to_string(),
        socket_path: state.paths.socket_path.to_string_lossy().to_string(),
        token_path: state.paths.token_path.to_string_lossy().to_string(),
    };

    loop {
        if state.shutdown.load(Ordering::SeqCst) {
            let mut guard = state.browser_client.lock().await;
            if let Some(client) = guard.as_mut() {
                client.shutdown().await;
            }
            break;
        }
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => {
                let mut guard = state.browser_client.lock().await;
                if let Some(client) = guard.as_mut() {
                    client.shutdown().await;
                }
                break;
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok(stream) => {
                        let state = state.clone();
                        tokio::spawn(async move {
                            if let Err(err) = handle_connection(stream, state).await {
                                eprintln!("[supervisor] connection error: {}", err);
                            }
                        });
                    }
                    Err(err) => {
                        eprintln!("[supervisor] accept error: {}", err);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }

    Ok(report)
}

pub(crate) async fn call(config: SupervisorConfig, method: &str, params: Value) -> Result<Value> {
    let paths = SupervisorPaths::for_config(&config);
    let mut client = SupervisorClient::connect_with_paths(paths).await?;
    client.call(method, params).await
}

pub(crate) async fn ensure_running(config: SupervisorConfig) -> Result<SupervisorClientStatus> {
    match call(config.clone(), "runtime.status", json!({})).await {
        Ok(result) => Ok(SupervisorClientStatus { ok: true, result }),
        Err(first_err) => {
            spawn_supervisor(&config).await?;
            let deadline = tokio::time::Instant::now() + Duration::from_millis(5_000);
            loop {
                match call(config.clone(), "runtime.status", json!({})).await {
                    Ok(result) => return Ok(SupervisorClientStatus { ok: true, result }),
                    Err(err) if tokio::time::Instant::now() < deadline => {
                        let _ = err;
                        tokio::time::sleep(Duration::from_millis(150)).await;
                    }
                    Err(err) => {
                        return Err(anyhow!(
                            "Supervisor did not become ready after spawn: {}; first error: {}",
                            err,
                            first_err
                        ))
                    }
                }
            }
        }
    }
}

async fn spawn_supervisor(config: &SupervisorConfig) -> Result<()> {
    let exe = std::env::current_exe().context("Resolve current executable")?;
    let mut command = std::process::Command::new(exe);
    command.arg("supervisor").arg("serve");
    if let Some(app_base) = config.app_base.as_ref() {
        command.arg("--app-base").arg(app_base);
    }
    match config.mode {
        NativeRunMode::Auto => {}
        NativeRunMode::Attach => {
            command.arg("--mode").arg("attach");
        }
        NativeRunMode::Spawn => {
            command.arg("--mode").arg("spawn");
        }
    }
    if let Some(endpoint_path) = config.endpoint_path.as_ref() {
        command.arg("--endpoint-path").arg(endpoint_path);
    }
    if let Some(worker_cmd) = config.worker_cmd.as_ref() {
        command.arg("--worker-cmd").arg(worker_cmd);
    }
    for arg in &config.worker_args {
        command.arg("--worker-arg").arg(arg);
    }
    if config.allow_legacy_worker_fallback {
        command.arg("--allow-legacy-worker-fallback");
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Spawn supervisor")?;
    Ok(())
}

async fn handle_connection(
    mut stream: LocalSocketStream,
    state: Arc<SupervisorState>,
) -> Result<()> {
    let token = read_token(&state.paths.token_path)?;
    let handshake = timeout(
        Duration::from_millis(HANDSHAKE_TIMEOUT_MS),
        read_frame(&mut stream),
    )
    .await
    .context("Supervisor handshake timeout")??;
    let value: Value = serde_json::from_slice(&handshake)?;
    if value.get("method").and_then(|value| value.as_str()) == Some("runtime.hello") {
        return handle_native_bridge_connection(stream, state, value, token).await;
    }

    let ok = value.get("type").and_then(|v| v.as_str()) == Some("rzn_local_handshake")
        && value.get("v").and_then(|v| v.as_u64()) == Some(1)
        && value.get("token").and_then(|v| v.as_str()) == Some(token.as_str());
    if !ok {
        let response = json!({ "ok": false, "error": "invalid supervisor handshake" });
        send_frame(&mut stream, &serde_json::to_vec(&response)?).await?;
        return Err(anyhow!("Invalid supervisor handshake"));
    }

    let response = json!({
        "ok": true,
        "protocol": RZN_LOCAL_PROTOCOL_VERSION,
        "pid": std::process::id()
    });
    send_frame(&mut stream, &serde_json::to_vec(&response)?).await?;

    loop {
        let frame = match read_frame(&mut stream).await {
            Ok(frame) => frame,
            Err(_) => break,
        };
        let request: Value = serde_json::from_slice(&frame)?;
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request
            .get("method")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));

        let response = match state.dispatch(method, params).await {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(err) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32000, "message": err.to_string() }
            }),
        };
        send_frame(&mut stream, &serde_json::to_vec(&response)?).await?;
    }

    Ok(())
}

async fn handle_native_bridge_connection(
    mut stream: LocalSocketStream,
    state: Arc<SupervisorState>,
    hello: Value,
    token: String,
) -> Result<()> {
    let id = hello.get("id").cloned().unwrap_or(Value::Null);
    let params = hello.get("params").cloned().unwrap_or_else(|| json!({}));
    let ok = params.get("version").and_then(|value| value.as_str())
        == Some(RZN_LOCAL_PROTOCOL_VERSION)
        && params.get("token").and_then(|value| value.as_str()) == Some(token.as_str())
        && params.get("role").and_then(|value| value.as_str()) == Some("native_host_bridge");
    if !ok {
        let response = json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32001, "message": "invalid native-host bridge hello" }
        });
        send_frame(&mut stream, &serde_json::to_vec(&response)?).await?;
        return Err(anyhow!("Invalid native-host bridge hello"));
    }

    let bridge_id = format!("native-host-{}", Uuid::new_v4());
    let (mut reader, mut writer) = stream.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    state.register_native_bridge(bridge_id.clone(), tx).await;

    let response = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "ok": true,
            "protocol": RZN_LOCAL_PROTOCOL_VERSION,
            "bridge_id": bridge_id,
            "pid": std::process::id(),
            "accepts": ["native_host.extension_call"]
        }
    });
    send_frame(&mut writer, &serde_json::to_vec(&response)?).await?;

    let writer_task = tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if send_frame(&mut writer, &bytes).await.is_err() {
                break;
            }
        }
    });

    loop {
        let frame = match read_frame(&mut reader).await {
            Ok(frame) => frame,
            Err(_) => break,
        };
        let value: Value = match serde_json::from_slice(&frame) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if state.complete_native_bridge_response(&value).await {
            continue;
        }
    }

    state.clear_native_bridge(&bridge_id).await;
    let _ = writer_task.await;
    Ok(())
}

struct SupervisorClient {
    stream: LocalSocketStream,
}

impl SupervisorClient {
    async fn connect_with_paths(paths: SupervisorPaths) -> Result<Self> {
        let token = read_token(&paths.token_path)?;
        let name = paths
            .socket_path
            .clone()
            .to_fs_name::<GenericFilePath>()
            .with_context(|| {
                format!(
                    "Invalid supervisor socket path {}",
                    paths.socket_path.display()
                )
            })?;
        let mut stream = timeout(
            Duration::from_millis(HANDSHAKE_TIMEOUT_MS),
            LocalSocketStream::connect(name),
        )
        .await
        .context("Connect supervisor timeout")??;

        let handshake = json!({
            "type": "rzn_local_handshake",
            "v": 1,
            "token": token,
            "client": {
                "name": "rzn-browser",
                "pid": std::process::id()
            }
        });
        send_frame(&mut stream, &serde_json::to_vec(&handshake)?).await?;
        let response = timeout(
            Duration::from_millis(HANDSHAKE_TIMEOUT_MS),
            read_frame(&mut stream),
        )
        .await
        .context("Supervisor handshake response timeout")??;
        let value: Value = serde_json::from_slice(&response)?;
        if value.get("ok").and_then(|value| value.as_bool()) != Some(true) {
            return Err(anyhow!("Supervisor handshake failed: {}", value));
        }
        Ok(Self { stream })
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let timeout_ms = supervisor_request_timeout_ms(&params);
        let id = format!("req-{}", Uuid::new_v4());
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        send_frame(&mut self.stream, &serde_json::to_vec(&request)?).await?;
        let response = timeout(
            Duration::from_millis(timeout_ms),
            read_frame(&mut self.stream),
        )
        .await
        .context("Supervisor request timeout")??;
        let value: Value = serde_json::from_slice(&response)?;
        if let Some(error) = value.get("error") {
            return Err(anyhow!("Supervisor error: {}", error));
        }
        Ok(value.get("result").cloned().unwrap_or(value))
    }
}

fn prepare_paths(paths: &SupervisorPaths) -> Result<()> {
    std::fs::create_dir_all(&paths.secure_dir)?;
    std::fs::create_dir_all(&paths.run_dir)?;
    get_or_create_token(&paths.token_path)?;
    Ok(())
}

fn get_or_create_token(path: &Path) -> Result<String> {
    if path.exists() {
        return read_token(path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let token = Uuid::new_v4().to_string();
    std::fs::write(path, format!("{}\n", token))?;
    Ok(token)
}

fn read_token(path: &Path) -> Result<String> {
    let token = std::fs::read_to_string(path)
        .with_context(|| format!("Read supervisor token {}", path.display()))?
        .trim()
        .to_string();
    if token.is_empty() {
        return Err(anyhow!("Supervisor token is empty: {}", path.display()));
    }
    Ok(token)
}

async fn send_frame<W: AsyncWrite + Unpin>(writer: &mut W, bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_FRAME_SIZE {
        return Err(anyhow!("Supervisor frame too large: {}", bytes.len()));
    }
    writer
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .await?;
    writer.write_all(bytes).await?;
    writer.flush().await?;
    Ok(())
}

async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    if len == 0 {
        return Err(anyhow!("Supervisor frame is empty"));
    }
    if len > MAX_FRAME_SIZE {
        return Err(anyhow!("Supervisor frame exceeds limit: {}", len));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

fn is_browser_tool(name: &str) -> bool {
    matches!(
        name,
        "browser.session_open"
            | "browser.session_close"
            | "browser.snapshot"
            | "browser.execute_step"
            | "browser.poll_events"
            | "rzn.worker.health"
            | "rzn.worker.shutdown"
            | "rzn.supervisor.health"
    )
}

fn tool_call_arguments(params: &Value) -> Value {
    let mut args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let timeout = params
        .get("timeout_ms")
        .or_else(|| params.get("timeoutMs"))
        .cloned();
    if let (Some(timeout), Value::Object(map)) = (timeout, &mut args) {
        if !map.contains_key("timeout_ms") && !map.contains_key("timeoutMs") {
            map.insert("timeout_ms".to_string(), timeout);
        }
    }
    args
}

fn extension_timeout_ms(params: &Value) -> u64 {
    params
        .get("timeout_ms")
        .and_then(|value| value.as_u64())
        .or_else(|| params.get("timeoutMs").and_then(|value| value.as_u64()))
        .or_else(|| {
            params.get("step").and_then(|step| {
                step.get("timeout_ms")
                    .and_then(|value| value.as_u64())
                    .or_else(|| step.get("timeoutMs").and_then(|value| value.as_u64()))
            })
        })
        .unwrap_or(EXTENSION_BRIDGE_TIMEOUT_MS)
        .saturating_add(5_000)
        .max(EXTENSION_BRIDGE_TIMEOUT_MS)
}

fn supervisor_request_timeout_ms(params: &Value) -> u64 {
    params
        .get("timeout_ms")
        .and_then(|value| value.as_u64())
        .or_else(|| params.get("timeoutMs").and_then(|value| value.as_u64()))
        .or_else(|| {
            params.get("arguments").and_then(|arguments| {
                arguments
                    .get("timeout_ms")
                    .and_then(|value| value.as_u64())
                    .or_else(|| arguments.get("timeoutMs").and_then(|value| value.as_u64()))
                    .or_else(|| {
                        arguments.get("step").and_then(|step| {
                            step.get("timeout_ms")
                                .and_then(|value| value.as_u64())
                                .or_else(|| step.get("timeoutMs").and_then(|value| value.as_u64()))
                        })
                    })
            })
        })
        .unwrap_or(REQUEST_TIMEOUT_MS)
        .saturating_add(5_000)
        .max(REQUEST_TIMEOUT_MS)
}

fn value_id_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn env_flag(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn default_app_base_dir() -> PathBuf {
    if let Some(root) = dirs::data_local_dir().or_else(dirs::data_dir) {
        return root.join("rzn-browser");
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".rzn-browser");
    }
    PathBuf::from(".rzn-browser")
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct JsonRpcRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> SupervisorConfig {
        SupervisorConfig {
            app_base: Some(PathBuf::from("/tmp/rzn-supervisor-test")),
            endpoint_path: None,
            mode: NativeRunMode::Auto,
            worker_cmd: None,
            worker_args: Vec::new(),
            allow_legacy_worker_fallback: false,
        }
    }

    #[test]
    fn default_paths_live_under_secure_and_run_dirs() {
        let config = test_config();
        let paths = SupervisorPaths::for_config(&config);
        assert_eq!(
            paths.socket_path,
            PathBuf::from("/tmp/rzn-supervisor-test/run/rzn-supervisor.sock")
        );
        assert_eq!(
            paths.token_path,
            PathBuf::from("/tmp/rzn-supervisor-test/secure/rzn-supervisor-token-v1")
        );
    }

    #[test]
    fn browser_tool_allowlist_is_explicit() {
        assert!(is_browser_tool("browser.execute_step"));
        assert!(is_browser_tool("rzn.worker.health"));
        assert!(is_browser_tool("rzn.worker.shutdown"));
        assert!(!is_browser_tool("runtime.status"));
        assert!(!is_browser_tool("shell.exec"));
    }

    #[tokio::test]
    async fn strict_supervisor_does_not_fall_back_to_worker_without_native_bridge() {
        let state = SupervisorState::new(test_config());
        let err = state
            .dispatch("browser.session_open", json!({}))
            .await
            .expect_err("strict supervisor should require native-host bridge");
        assert!(err
            .to_string()
            .contains("requires the supervisor native-host bridge"));
    }

    #[test]
    fn extension_timeout_uses_step_timeout_with_grace() {
        let params = json!({
            "step": {
                "type": "click",
                "timeout_ms": 12_000
            }
        });
        assert_eq!(extension_timeout_ms(&params), EXTENSION_BRIDGE_TIMEOUT_MS);
    }

    #[test]
    fn tools_call_timeout_is_forwarded_into_arguments() {
        let args = tool_call_arguments(&json!({
            "name": "browser.execute_step",
            "timeout_ms": 42_000,
            "arguments": {
                "session_id": "s1",
                "step": { "type": "click" }
            }
        }));
        assert_eq!(args.get("timeout_ms"), Some(&json!(42_000)));
    }
}
