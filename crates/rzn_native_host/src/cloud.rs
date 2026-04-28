use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use rzn_contracts::v1::{
    ActionResultV1, ActorHelloV1, ActorReadyV1, CapabilitiesV1, CloudCommandAckV1,
    CloudCommandEnvelopeV1, CloudCommandKindV1, CloudCommandResultV1, CLOUD_CONTRACT_VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

const CLOUD_ACTOR_CONFIG_VERSION: &str = "rzn.cloud.actor_config.v1";
const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 15_000;
const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 45_000;
const DEFAULT_EXTENSION_RPC_GRACE_MS: u64 = 5_000;
const MAX_CACHED_COMMAND_RESULTS: usize = 256;
const DEFAULT_STATUS_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalCloudActorConfig {
    version: String,
    actor_id: String,
    workspace_id: String,
    actor_token: String,
    server_url: String,
    websocket_url: String,
    paired_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    connect_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CloudActorStatusSnapshot {
    pub supported: bool,
    pub actor_mode: String,
    pub config_path: String,
    pub configured: bool,
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub websocket_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paired_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_connected_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_ready_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct CloudActorRuntimeState {
    config: Option<LocalCloudActorConfig>,
    connected: bool,
    last_connected_at_ms: Option<u64>,
    last_ready_at_ms: Option<u64>,
    last_error: Option<String>,
}

#[derive(Debug)]
enum CloudActorControlMessage {
    ApplyConfig(Option<LocalCloudActorConfig>),
}

#[derive(Clone)]
struct CloudActorManager {
    status: Arc<Mutex<CloudActorRuntimeState>>,
    control_tx: mpsc::UnboundedSender<CloudActorControlMessage>,
}

static CLOUD_ACTOR_MANAGER: OnceLock<CloudActorManager> = OnceLock::new();

#[derive(Default)]
struct CommandResultCache {
    order: Vec<String>,
    entries: HashMap<String, CloudCommandResultV1>,
}

impl CommandResultCache {
    fn get(&self, command_id: &str) -> Option<CloudCommandResultV1> {
        self.entries.get(command_id).cloned()
    }

    fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
    }

    fn insert(&mut self, command_id: String, result: CloudCommandResultV1) {
        if !self.entries.contains_key(&command_id) {
            self.order.push(command_id.clone());
        }
        self.entries.insert(command_id.clone(), result);

        while self.order.len() > MAX_CACHED_COMMAND_RESULTS {
            let evicted = self.order.remove(0);
            self.entries.remove(&evicted);
        }
    }
}

pub(crate) fn maybe_spawn_cloud_actor(
    native_tx: mpsc::UnboundedSender<Vec<u8>>,
    native_pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
) {
    if CLOUD_ACTOR_MANAGER.get().is_some() {
        return;
    }

    let (config, last_error) = match load_local_actor_config() {
        Ok(config) => {
            if let Some(config) = config.as_ref() {
                tracing::info!(
                    actor_id = %config.actor_id,
                    websocket_url = %config.websocket_url,
                    "Hosted control plane enabled for native host"
                );
            } else {
                tracing::info!("Hosted control plane idle; no cloud actor config present");
            }
            (config, None)
        }
        Err(error) => {
            tracing::error!("Failed to load cloud actor config: {}", error);
            (None, Some(error.to_string()))
        }
    };

    let status = Arc::new(Mutex::new(CloudActorRuntimeState {
        config: config.clone(),
        connected: false,
        last_connected_at_ms: None,
        last_ready_at_ms: None,
        last_error,
    }));
    let (control_tx, control_rx) = mpsc::unbounded_channel::<CloudActorControlMessage>();
    let _ = CLOUD_ACTOR_MANAGER.set(CloudActorManager {
        status: status.clone(),
        control_tx,
    });
    let result_cache = Arc::new(Mutex::new(CommandResultCache::default()));

    tokio::spawn(async move {
        if let Err(error) = run_cloud_actor_loop(
            config,
            control_rx,
            native_tx,
            native_pending,
            result_cache,
            status,
        )
        .await
        {
            tracing::error!("Cloud actor loop exited: {}", error);
        }
    });
}

async fn run_cloud_actor_loop(
    mut current_config: Option<LocalCloudActorConfig>,
    mut control_rx: mpsc::UnboundedReceiver<CloudActorControlMessage>,
    native_tx: mpsc::UnboundedSender<Vec<u8>>,
    native_pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    result_cache: Arc<Mutex<CommandResultCache>>,
    status: Arc<Mutex<CloudActorRuntimeState>>,
) -> Result<()> {
    let mut reconnect_backoff = Duration::from_secs(1);

    loop {
        let Some(config) = current_config.clone() else {
            match control_rx.recv().await {
                Some(control) => {
                    current_config =
                        apply_cloud_control_message(control, &status, &result_cache).await;
                    reconnect_backoff = Duration::from_secs(1);
                    continue;
                }
                None => return Ok(()),
            };
        };

        let connect_timeout_ms = config
            .connect_timeout_ms
            .unwrap_or(DEFAULT_CONNECT_TIMEOUT_MS)
            .max(1);
        let connect_url = match build_actor_ws_url(&config) {
            Ok(url) => url.to_string(),
            Err(error) => {
                update_cloud_status_error(&status, error.to_string()).await;
                tracing::warn!("Invalid cloud actor websocket config: {}", error);
                match tokio::time::timeout(
                    Duration::from_millis(DEFAULT_STATUS_TIMEOUT_MS),
                    control_rx.recv(),
                )
                .await
                {
                    Ok(Some(control)) => {
                        current_config =
                            apply_cloud_control_message(control, &status, &result_cache).await;
                        reconnect_backoff = Duration::from_secs(1);
                        continue;
                    }
                    _ => continue,
                }
            }
        };

        tokio::select! {
            maybe_control = control_rx.recv() => {
                match maybe_control {
                    Some(control) => {
                        current_config =
                            apply_cloud_control_message(control, &status, &result_cache).await;
                        reconnect_backoff = Duration::from_secs(1);
                        continue;
                    }
                    None => return Ok(()),
                }
            }
            connect_result = tokio::time::timeout(
                Duration::from_millis(connect_timeout_ms),
                connect_async(connect_url),
            ) => {
                match connect_result {
                    Ok(Ok((socket, _response))) => {
                        tracing::info!(actor_id = %config.actor_id, "Connected native host cloud actor websocket");
                        reconnect_backoff = Duration::from_secs(1);
                        let mut session_task = tokio::spawn(run_cloud_actor_session(
                            socket,
                            config.clone(),
                            native_tx.clone(),
                            native_pending.clone(),
                            result_cache.clone(),
                            status.clone(),
                        ));

                        tokio::select! {
                            maybe_control = control_rx.recv() => {
                                session_task.abort();
                                let _ = (&mut session_task).await;
                                mark_cloud_actor_disconnected(&status).await;
                                match maybe_control {
                                    Some(control) => {
                                        current_config =
                                            apply_cloud_control_message(control, &status, &result_cache).await;
                                        reconnect_backoff = Duration::from_secs(1);
                                    }
                                    None => return Ok(()),
                                }
                                continue;
                            }
                            session_result = &mut session_task => {
                                mark_cloud_actor_disconnected(&status).await;
                                match session_result {
                                    Ok(Ok(())) => {}
                                    Ok(Err(error)) => {
                                        update_cloud_status_error(&status, error.to_string()).await;
                                        tracing::warn!("Cloud actor session ended: {}", error);
                                    }
                                    Err(error) if error.is_cancelled() => {}
                                    Err(error) => {
                                        update_cloud_status_error(&status, error.to_string()).await;
                                        tracing::warn!("Cloud actor task join error: {}", error);
                                    }
                                }
                            }
                        }
                    }
                    Ok(Err(error)) => {
                        update_cloud_status_error(&status, error.to_string()).await;
                        tracing::warn!("Cloud actor websocket connect failed: {}", error);
                    }
                    Err(_) => {
                        update_cloud_status_error(&status, "Cloud actor websocket connect timed out".to_string()).await;
                        tracing::warn!("Cloud actor websocket connect timed out");
                    }
                }
            }
        }

        tokio::select! {
            maybe_control = control_rx.recv() => {
                match maybe_control {
                    Some(control) => {
                        current_config =
                            apply_cloud_control_message(control, &status, &result_cache).await;
                        reconnect_backoff = Duration::from_secs(1);
                        continue;
                    }
                    None => return Ok(()),
                }
            }
            _ = tokio::time::sleep(reconnect_backoff) => {}
        }
        reconnect_backoff = std::cmp::min(reconnect_backoff * 2, Duration::from_secs(30));
    }
}

async fn run_cloud_actor_session(
    mut socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    config: LocalCloudActorConfig,
    native_tx: mpsc::UnboundedSender<Vec<u8>>,
    native_pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    result_cache: Arc<Mutex<CommandResultCache>>,
    status: Arc<Mutex<CloudActorRuntimeState>>,
) -> Result<()> {
    let hello = ActorHelloV1 {
        version: CLOUD_CONTRACT_VERSION.to_string(),
        message_type: "actor.hello".to_string(),
        actor_id: config.actor_id.clone(),
        workspace_id: config.workspace_id.clone(),
        extension_version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: CapabilitiesV1 {
            extension_actor: true,
            cdp_available: true,
            cdp_enabled: false,
            cdp_attached: false,
        },
        metadata: Some(json!({
            "actor_mode": "native_host",
            "native_host_pid": std::process::id(),
            "server_url": config.server_url,
        })),
    };
    socket
        .send(Message::Text(serde_json::to_string(&hello)?))
        .await
        .context("Send actor hello")?;

    let ready = await_actor_ready(&mut socket).await?;
    tracing::info!(
        actor_id = %ready.actor_id,
        heartbeat_interval_ms = ready.heartbeat_interval_ms,
        "Cloud actor ready"
    );
    mark_cloud_actor_ready(&status).await;
    let heartbeat = Duration::from_millis(ready.heartbeat_interval_ms.max(10_000));

    loop {
        tokio::select! {
            _ = tokio::time::sleep(heartbeat) => {
                socket.send(Message::Ping(Vec::new())).await.context("Send websocket ping")?;
            }
            maybe_message = socket.next() => {
                match maybe_message {
                    Some(Ok(Message::Text(text))) => {
                        let envelope: CloudCommandEnvelopeV1 =
                            serde_json::from_str(&text).context("Decode cloud command envelope")?;
                        if envelope.payload.kind != CloudCommandKindV1::BrowserCommand {
                            let result = error_command_result(
                                &envelope,
                                "Unsupported cloud command kind; only browser_command is implemented",
                            );
                            socket
                                .send(Message::Text(serde_json::to_string(&result)?))
                                .await
                                .context("Send unsupported command result")?;
                            continue;
                        }

                        if let Some(cached_result) = result_cache.lock().await.get(&envelope.command_id) {
                            let ack = CloudCommandAckV1 {
                                version: CLOUD_CONTRACT_VERSION.to_string(),
                                message_type: "command.ack".to_string(),
                                actor_id: envelope.actor_id.clone(),
                                run_id: envelope.run_id.clone(),
                                session_id: envelope.session_id.clone(),
                                command_id: envelope.command_id.clone(),
                                lease_id: envelope.lease_id.clone(),
                                accepted_at_ms: now_ms(),
                                trace_id: envelope.trace_id.clone(),
                            };
                            socket
                                .send(Message::Text(serde_json::to_string(&ack)?))
                                .await
                                .context("Send duplicate command ack")?;
                            socket
                                .send(Message::Text(serde_json::to_string(&cached_result)?))
                                .await
                                .context("Send cached command result")?;
                            continue;
                        }

                        let ack = CloudCommandAckV1 {
                            version: CLOUD_CONTRACT_VERSION.to_string(),
                            message_type: "command.ack".to_string(),
                            actor_id: envelope.actor_id.clone(),
                            run_id: envelope.run_id.clone(),
                            session_id: envelope.session_id.clone(),
                            command_id: envelope.command_id.clone(),
                            lease_id: envelope.lease_id.clone(),
                            accepted_at_ms: now_ms(),
                            trace_id: envelope.trace_id.clone(),
                        };
                        socket
                            .send(Message::Text(serde_json::to_string(&ack)?))
                            .await
                            .context("Send command ack")?;

                        let result = dispatch_command_to_extension(
                            &envelope,
                            config.request_timeout_ms.unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS),
                            native_tx.clone(),
                            native_pending.clone(),
                        ).await;
                        result_cache
                            .lock()
                            .await
                            .insert(envelope.command_id.clone(), result.clone());

                        socket
                            .send(Message::Text(serde_json::to_string(&result)?))
                            .await
                            .context("Send command result")?;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        socket.send(Message::Pong(payload)).await.context("Reply pong")?;
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => bail!("Cloud control plane closed websocket"),
                    Some(Err(error)) => return Err(error.into()),
                    _ => {}
                }
            }
        }
    }
}

async fn apply_cloud_control_message(
    control: CloudActorControlMessage,
    status: &Arc<Mutex<CloudActorRuntimeState>>,
    result_cache: &Arc<Mutex<CommandResultCache>>,
) -> Option<LocalCloudActorConfig> {
    match control {
        CloudActorControlMessage::ApplyConfig(next_config) => {
            result_cache.lock().await.clear();
            let mut guard = status.lock().await;
            guard.config = next_config.clone();
            guard.connected = false;
            guard.last_error = None;
            if next_config.is_none() {
                guard.last_connected_at_ms = None;
                guard.last_ready_at_ms = None;
            }
            next_config
        }
    }
}

async fn update_cloud_status_error(status: &Arc<Mutex<CloudActorRuntimeState>>, error: String) {
    let mut guard = status.lock().await;
    guard.connected = false;
    guard.last_error = Some(error);
}

async fn mark_cloud_actor_ready(status: &Arc<Mutex<CloudActorRuntimeState>>) {
    let now = now_ms();
    let mut guard = status.lock().await;
    guard.connected = true;
    guard.last_connected_at_ms = Some(now);
    guard.last_ready_at_ms = Some(now);
    guard.last_error = None;
}

async fn mark_cloud_actor_disconnected(status: &Arc<Mutex<CloudActorRuntimeState>>) {
    status.lock().await.connected = false;
}

fn manager_snapshot_from_state(state: &CloudActorRuntimeState) -> CloudActorStatusSnapshot {
    let config_path = resolve_cloud_actor_config_path();
    let config = state.config.as_ref();
    CloudActorStatusSnapshot {
        supported: true,
        actor_mode: "native_host".to_string(),
        config_path: config_path.display().to_string(),
        configured: config.is_some(),
        connected: state.connected,
        actor_id: config.map(|value| value.actor_id.clone()),
        workspace_id: config.map(|value| value.workspace_id.clone()),
        server_url: config.map(|value| value.server_url.clone()),
        websocket_url: config.map(|value| value.websocket_url.clone()),
        paired_at_ms: config.map(|value| value.paired_at_ms),
        connect_timeout_ms: config.and_then(|value| value.connect_timeout_ms),
        request_timeout_ms: config.and_then(|value| value.request_timeout_ms),
        last_connected_at_ms: state.last_connected_at_ms,
        last_ready_at_ms: state.last_ready_at_ms,
        last_error: state.last_error.clone(),
    }
}

async fn current_cloud_actor_status_snapshot() -> CloudActorStatusSnapshot {
    if let Some(manager) = CLOUD_ACTOR_MANAGER.get() {
        let guard = manager.status.lock().await;
        return manager_snapshot_from_state(&guard);
    }

    match load_local_actor_config() {
        Ok(config) => manager_snapshot_from_state(&CloudActorRuntimeState {
            config,
            connected: false,
            last_connected_at_ms: None,
            last_ready_at_ms: None,
            last_error: None,
        }),
        Err(error) => manager_snapshot_from_state(&CloudActorRuntimeState {
            config: None,
            connected: false,
            last_connected_at_ms: None,
            last_ready_at_ms: None,
            last_error: Some(error.to_string()),
        }),
    }
}

fn normalize_server_url(server_url: &str) -> Result<String> {
    let mut url = Url::parse(server_url)
        .with_context(|| format!("Invalid cloud server URL {}", server_url))?;
    if url.path() == "/" {
        url.set_path("");
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn websocket_url_from_server(server_url: &str) -> Result<String> {
    let mut url = Url::parse(server_url)?;
    let scheme = match url.scheme() {
        "https" => "wss",
        _ => "ws",
    };
    url.set_scheme(scheme)
        .map_err(|_| anyhow!("Failed to convert {} to websocket URL", server_url))?;
    url.set_path("/v1/actors/connect");
    url.set_query(None);
    Ok(url.to_string())
}

fn normalize_local_actor_config(
    mut config: LocalCloudActorConfig,
) -> Result<LocalCloudActorConfig> {
    if config.version.trim().is_empty() {
        config.version = CLOUD_ACTOR_CONFIG_VERSION.to_string();
    }
    if config.version != CLOUD_ACTOR_CONFIG_VERSION {
        bail!("Unsupported cloud actor config version {}", config.version);
    }
    if config.actor_id.trim().is_empty() {
        bail!("actor_id is required");
    }
    if config.workspace_id.trim().is_empty() {
        bail!("workspace_id is required");
    }
    if config.actor_token.trim().is_empty() {
        bail!("actor_token is required");
    }

    let server_url = normalize_server_url(&config.server_url)?;
    config.server_url = server_url.clone();
    config.websocket_url = if config.websocket_url.trim().is_empty() {
        websocket_url_from_server(&server_url)?
    } else {
        config.websocket_url.trim().to_string()
    };
    if config.paired_at_ms == 0 {
        config.paired_at_ms = now_ms();
    }
    if config.connect_timeout_ms.is_none() {
        config.connect_timeout_ms = Some(DEFAULT_CONNECT_TIMEOUT_MS);
    }
    if config.request_timeout_ms.is_none() {
        config.request_timeout_ms = Some(DEFAULT_REQUEST_TIMEOUT_MS);
    }
    Ok(config)
}

fn persist_local_actor_config(
    path: &std::path::Path,
    config: &LocalCloudActorConfig,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Create cloud actor config directory {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(config)?;
    std::fs::write(path, bytes)
        .with_context(|| format!("Write cloud actor config {}", path.display()))
}

fn clear_local_actor_config(path: &std::path::Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path)
            .with_context(|| format!("Remove cloud actor config {}", path.display()))?;
    }
    Ok(())
}

fn build_control_response(
    request: &Value,
    cmd: &str,
    success: bool,
    result: Value,
    error: Option<String>,
) -> Value {
    let req_id = request
        .get("req_id")
        .and_then(|value| value.as_str())
        .unwrap_or("cloud-control");
    let mut response = json!({
        "cmd": format!("{}_response", cmd),
        "req_id": req_id,
        "success": success,
        "result": result,
    });
    if let Some(error) = error {
        response["error"] = Value::String(error.clone());
        response["error_msg"] = Value::String(error);
    }
    response
}

pub(crate) async fn handle_local_control_command(request: &Value) -> Option<Value> {
    let cmd = request.get("cmd").and_then(|value| value.as_str())?;
    match cmd {
        "cloud_get_status" => {
            let snapshot = current_cloud_actor_status_snapshot().await;
            Some(build_control_response(
                request,
                cmd,
                true,
                serde_json::to_value(snapshot).unwrap_or_else(|_| json!({})),
                None,
            ))
        }
        "cloud_set_config" => {
            let payload = request.get("payload").cloned().unwrap_or_else(|| json!({}));
            let config_value = payload.get("config").cloned().unwrap_or(payload);
            let config: LocalCloudActorConfig = match serde_json::from_value(config_value) {
                Ok(config) => config,
                Err(error) => {
                    return Some(build_control_response(
                        request,
                        cmd,
                        false,
                        json!({}),
                        Some(format!("Invalid cloud config payload: {}", error)),
                    ));
                }
            };

            match apply_cloud_actor_config(config).await {
                Ok(snapshot) => Some(build_control_response(
                    request,
                    cmd,
                    true,
                    serde_json::to_value(snapshot).unwrap_or_else(|_| json!({})),
                    None,
                )),
                Err(error) => Some(build_control_response(
                    request,
                    cmd,
                    false,
                    json!({}),
                    Some(error.to_string()),
                )),
            }
        }
        "cloud_clear_config" => match clear_cloud_actor_runtime_config().await {
            Ok(snapshot) => Some(build_control_response(
                request,
                cmd,
                true,
                serde_json::to_value(snapshot).unwrap_or_else(|_| json!({})),
                None,
            )),
            Err(error) => Some(build_control_response(
                request,
                cmd,
                false,
                json!({}),
                Some(error.to_string()),
            )),
        },
        _ => None,
    }
}

async fn apply_cloud_actor_config(
    config: LocalCloudActorConfig,
) -> Result<CloudActorStatusSnapshot> {
    let normalized = normalize_local_actor_config(config)?;
    let path = resolve_cloud_actor_config_path();
    persist_local_actor_config(&path, &normalized)?;
    if let Some(manager) = CLOUD_ACTOR_MANAGER.get() {
        manager
            .control_tx
            .send(CloudActorControlMessage::ApplyConfig(Some(
                normalized.clone(),
            )))
            .map_err(|_| anyhow!("Cloud actor manager is unavailable"))?;
    }
    if let Some(manager) = CLOUD_ACTOR_MANAGER.get() {
        let mut guard = manager.status.lock().await;
        guard.config = Some(normalized);
        guard.connected = false;
        guard.last_error = None;
    }
    Ok(current_cloud_actor_status_snapshot().await)
}

async fn clear_cloud_actor_runtime_config() -> Result<CloudActorStatusSnapshot> {
    let path = resolve_cloud_actor_config_path();
    clear_local_actor_config(&path)?;
    if let Some(manager) = CLOUD_ACTOR_MANAGER.get() {
        manager
            .control_tx
            .send(CloudActorControlMessage::ApplyConfig(None))
            .map_err(|_| anyhow!("Cloud actor manager is unavailable"))?;
        let mut guard = manager.status.lock().await;
        guard.config = None;
        guard.connected = false;
        guard.last_connected_at_ms = None;
        guard.last_ready_at_ms = None;
        guard.last_error = None;
    }
    Ok(current_cloud_actor_status_snapshot().await)
}

async fn await_actor_ready(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Result<ActorReadyV1> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let maybe_message = tokio::time::timeout_at(deadline, socket.next())
            .await
            .context("Timed out waiting for actor.ready")?;
        match maybe_message {
            Some(Ok(Message::Text(text))) => {
                let ready: ActorReadyV1 =
                    serde_json::from_str(&text).context("Decode actor.ready")?;
                return Ok(ready);
            }
            Some(Ok(Message::Ping(payload))) => {
                socket.send(Message::Pong(payload)).await?;
            }
            Some(Ok(Message::Pong(_))) => {}
            Some(Ok(Message::Close(_))) | None => {
                bail!("Control plane disconnected before actor.ready")
            }
            Some(Err(error)) => return Err(error.into()),
            _ => {}
        }
    }
}

async fn dispatch_command_to_extension(
    envelope: &CloudCommandEnvelopeV1,
    default_request_timeout_ms: u64,
    native_tx: mpsc::UnboundedSender<Vec<u8>>,
    native_pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
) -> CloudCommandResultV1 {
    match dispatch_command_to_extension_inner(
        envelope,
        default_request_timeout_ms,
        native_tx,
        native_pending,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => error_command_result(envelope, error.to_string()),
    }
}

async fn dispatch_command_to_extension_inner(
    envelope: &CloudCommandEnvelopeV1,
    default_request_timeout_ms: u64,
    native_tx: mpsc::UnboundedSender<Vec<u8>>,
    native_pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
) -> Result<CloudCommandResultV1> {
    let command = envelope
        .payload
        .command
        .as_ref()
        .ok_or_else(|| anyhow!("browser_command payload missing command object"))?;
    let request_json = build_extension_request(
        envelope,
        command.cmd.as_str(),
        command.payload.clone(),
        command.data.clone(),
    );
    let request_bytes = serde_json::to_vec(&request_json).context("Serialize extension request")?;
    let (tx, rx) = oneshot::channel::<Value>();
    {
        let mut guard = native_pending.lock().await;
        guard.insert(envelope.command_id.clone(), tx);
    }
    if native_tx.send(request_bytes).is_err() {
        let mut guard = native_pending.lock().await;
        guard.remove(&envelope.command_id);
        bail!("Extension connection is not available");
    }

    let timeout_ms = compute_request_timeout_ms(
        envelope.deadline_ms,
        default_request_timeout_ms,
        DEFAULT_EXTENSION_RPC_GRACE_MS,
    );
    let response = match tokio::time::timeout(Duration::from_millis(timeout_ms.max(1)), rx).await {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => {
            let mut guard = native_pending.lock().await;
            guard.remove(&envelope.command_id);
            bail!(
                "Extension response channel closed for {}",
                envelope.command_id
            );
        }
        Err(_) => {
            let mut guard = native_pending.lock().await;
            guard.remove(&envelope.command_id);
            bail!(
                "Timed out waiting for extension response {}",
                envelope.command_id
            );
        }
    };

    Ok(command_result_from_extension(envelope, response))
}

fn build_extension_request(
    envelope: &CloudCommandEnvelopeV1,
    cmd: &str,
    payload: Option<Value>,
    data: Option<Value>,
) -> Value {
    let mut request = json!({
        "cmd": cmd,
        "req_id": envelope.command_id,
    });

    if let Some(mut payload_value) = payload {
        if let Some(obj) = payload_value.as_object_mut() {
            if !obj.contains_key("session_id") {
                obj.insert(
                    "session_id".to_string(),
                    Value::String(envelope.session_id.clone()),
                );
            }
        }
        request["payload"] = payload_value;
    } else {
        request["payload"] = json!({
            "session_id": envelope.session_id
        });
    }

    if let Some(data_value) = data {
        request["data"] = data_value;
    }

    request
}

fn command_result_from_extension(
    envelope: &CloudCommandEnvelopeV1,
    response: Value,
) -> CloudCommandResultV1 {
    let success = response
        .get("success")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let error_message = response
        .get("error_msg")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .or_else(|| {
            response
                .get("error")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
        });
    let capabilities = response
        .get("capabilities")
        .cloned()
        .and_then(|value| serde_json::from_value::<CapabilitiesV1>(value).ok());
    let action_result = ActionResultV1 {
        success,
        error_code: response
            .get("error_code")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        error: error_message.clone(),
        current_url: response
            .get("current_url")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .or_else(|| {
                response
                    .pointer("/result/url")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string())
            }),
        current_tab_id: response
            .get("current_tab_id")
            .and_then(|value| value.as_u64())
            .map(|value| value as u32)
            .or_else(|| {
                response
                    .pointer("/result/tabId")
                    .and_then(|value| value.as_u64())
                    .map(|value| value as u32)
            }),
        dom_hash: response
            .get("dom_hash")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        dom_snapshot: None,
        capabilities,
        raw: Some(response),
    };

    CloudCommandResultV1 {
        version: CLOUD_CONTRACT_VERSION.to_string(),
        message_type: "command.result".to_string(),
        actor_id: envelope.actor_id.clone(),
        run_id: envelope.run_id.clone(),
        session_id: envelope.session_id.clone(),
        command_id: envelope.command_id.clone(),
        lease_id: envelope.lease_id.clone(),
        success,
        finished_at_ms: now_ms(),
        trace_id: envelope.trace_id.clone(),
        result: Some(action_result),
        error: error_message,
    }
}

fn error_command_result(
    envelope: &CloudCommandEnvelopeV1,
    error: impl Into<String>,
) -> CloudCommandResultV1 {
    let error = error.into();
    CloudCommandResultV1 {
        version: CLOUD_CONTRACT_VERSION.to_string(),
        message_type: "command.result".to_string(),
        actor_id: envelope.actor_id.clone(),
        run_id: envelope.run_id.clone(),
        session_id: envelope.session_id.clone(),
        command_id: envelope.command_id.clone(),
        lease_id: envelope.lease_id.clone(),
        success: false,
        finished_at_ms: now_ms(),
        trace_id: envelope.trace_id.clone(),
        result: Some(ActionResultV1 {
            success: false,
            error_code: Some("CLOUD_ACTOR_ERROR".to_string()),
            error: Some(error.clone()),
            current_url: None,
            current_tab_id: None,
            dom_hash: None,
            dom_snapshot: None,
            capabilities: None,
            raw: None,
        }),
        error: Some(error),
    }
}

fn build_actor_ws_url(config: &LocalCloudActorConfig) -> Result<Url> {
    let mut url = Url::parse(&config.websocket_url)
        .with_context(|| format!("Invalid websocket URL {}", config.websocket_url))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("actor_id", &config.actor_id);
        pairs.append_pair("actor_token", &config.actor_token);
    }
    Ok(url)
}

fn load_local_actor_config() -> Result<Option<LocalCloudActorConfig>> {
    let path = resolve_cloud_actor_config_path();
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("Read cloud actor config {}", path.display()))?;
    let config: LocalCloudActorConfig =
        serde_json::from_slice(&bytes).context("Parse cloud actor config JSON")?;
    if config.version != CLOUD_ACTOR_CONFIG_VERSION {
        bail!(
            "Unsupported cloud actor config version {} in {}",
            config.version,
            path.display()
        );
    }
    Ok(Some(config))
}

fn resolve_cloud_actor_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("RZN_CLOUD_CONFIG_PATH") {
        return PathBuf::from(path);
    }
    let base = dirs::config_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("rzn").join("cloud_actor_v1.json")
}

fn compute_request_timeout_ms(
    deadline_ms: u64,
    default_request_timeout_ms: u64,
    rpc_grace_ms: u64,
) -> u64 {
    let now = now_ms();
    if deadline_ms <= now {
        return default_request_timeout_ms
            .saturating_add(rpc_grace_ms)
            .max(1);
    }
    deadline_ms
        .saturating_sub(now)
        .saturating_add(rpc_grace_ms)
        .max(1)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use rzn_contracts::v1::{CloudBrowserCommandV1, CloudCommandPayloadV1};

    fn sample_envelope() -> CloudCommandEnvelopeV1 {
        CloudCommandEnvelopeV1 {
            version: CLOUD_CONTRACT_VERSION.to_string(),
            message_type: "command.execute".to_string(),
            actor_id: "actor-1".to_string(),
            run_id: "run-1".to_string(),
            session_id: "session-1".to_string(),
            command_id: "command-1".to_string(),
            lease_id: "lease-1".to_string(),
            deadline_ms: now_ms() + 10_000,
            trace_id: Some("trace-1".to_string()),
            parent_command_id: None,
            planner_step_index: Some(0),
            payload: CloudCommandPayloadV1 {
                kind: CloudCommandKindV1::BrowserCommand,
                command: Some(CloudBrowserCommandV1 {
                    cmd: "execute_step".to_string(),
                    payload: Some(json!({
                        "step": {
                            "type": "click_element"
                        }
                    })),
                    data: None,
                }),
                side_effecting: Some(true),
                idempotency_policy: Some("single_delivery".to_string()),
                metadata: None,
            },
        }
    }

    #[test]
    fn build_extension_request_injects_session_id() {
        let envelope = sample_envelope();
        let request = build_extension_request(
            &envelope,
            "execute_step",
            envelope
                .payload
                .command
                .as_ref()
                .and_then(|cmd| cmd.payload.clone()),
            None,
        );
        assert_eq!(
            request
                .pointer("/payload/session_id")
                .and_then(|value| value.as_str()),
            Some("session-1")
        );
    }

    #[test]
    fn request_timeout_includes_rpc_grace() {
        let timeout_ms = compute_request_timeout_ms(now_ms().saturating_add(250), 1_000, 5_000);
        assert!((5_000..=5_250).contains(&timeout_ms));
    }

    #[test]
    fn elapsed_deadline_falls_back_to_default_plus_grace() {
        let timeout_ms = compute_request_timeout_ms(now_ms().saturating_sub(1), 1_000, 5_000);
        assert_eq!(timeout_ms, 6_000);
    }
}
