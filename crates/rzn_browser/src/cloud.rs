use crate::workflow_params::apply_parameters;
use anyhow::{anyhow, bail, Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::{Args, Subcommand};
use futures_util::{SinkExt, StreamExt};
use rzn_contracts::v1::{
    ActionResultV1, ActorHelloV1, ActorReadyV1, CapabilitiesV1, CloudBrowserCommandV1,
    CloudCommandAckV1, CloudCommandEnvelopeV1, CloudCommandKindV1, CloudCommandPayloadV1,
    CloudCommandResultV1, CLOUD_CONTRACT_VERSION,
};
use rzn_core::secure_files::write_secret_file;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Read;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use url::Url;
use uuid::Uuid;

const DEFAULT_CLOUD_SERVER_URL: &str = "http://127.0.0.1:8787";
const DEFAULT_HEARTBEAT_INTERVAL_MS: u64 = 30_000;
const DEFAULT_WORKFLOW_TIMEOUT_MS: u64 = 45_000;
const CLOUD_ACTOR_CONFIG_VERSION: &str = "rzn.cloud.actor_config.v1";
const CONTROL_PLANE_STATE_VERSION: &str = "rzn.cloud.control_plane_state.v1";
const CLOUD_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CLOUD_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
static CLOUD_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn cloud_http_client() -> &'static reqwest::Client {
    CLOUD_HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(CLOUD_HTTP_CONNECT_TIMEOUT)
            .timeout(CLOUD_HTTP_REQUEST_TIMEOUT)
            .build()
            .expect("cloud HTTP client configuration is valid")
    })
}

#[derive(Subcommand, Debug)]
pub enum CloudCommands {
    /// Run the hosted cloud control plane.
    Serve(CloudServeArgs),

    /// Mint a one-time pairing code on the control plane.
    #[command(name = "issue-pairing-code")]
    IssuePairingCode(CloudIssuePairingCodeArgs),

    /// Redeem a pairing code locally and write broker actor config.
    Pair(CloudPairArgs),

    /// Submit a workflow to a paired actor and wait for completion.
    #[command(name = "run-workflow")]
    RunWorkflow(CloudRunWorkflowArgs),

    /// Fetch a previously recorded run.
    #[command(name = "get-run")]
    GetRun(CloudGetRunArgs),

    /// List paired actors and their connectivity state.
    #[command(name = "list-actors")]
    ListActors(CloudListActorsArgs),

    /// Dispatch a single hosted browser command and wait for its result.
    #[command(name = "exec-command")]
    ExecCommand(CloudExecCommandArgs),

    /// Dispatch a single execute_step browser command and wait for its result.
    #[command(name = "exec-step")]
    ExecStep(CloudExecStepArgs),
}

#[derive(Args, Debug)]
pub struct CloudServeArgs {
    /// Base HTTP listen address.
    #[arg(long, default_value = "127.0.0.1:8787")]
    pub listen: String,

    /// Public base URL used in pairing responses.
    #[arg(long, default_value = DEFAULT_CLOUD_SERVER_URL)]
    pub public_url: String,
}

#[derive(Args, Debug)]
pub struct CloudIssuePairingCodeArgs {
    /// Control plane base URL.
    #[arg(long, default_value = DEFAULT_CLOUD_SERVER_URL)]
    pub server: String,

    /// Workspace identifier to scope the actor into.
    #[arg(long, default_value = "default")]
    pub workspace_id: String,

    /// Pairing code TTL in seconds.
    #[arg(long, default_value_t = 600)]
    pub ttl_secs: u64,
}

#[derive(Args, Debug)]
pub struct CloudPairArgs {
    /// Control plane base URL.
    #[arg(long, default_value = DEFAULT_CLOUD_SERVER_URL)]
    pub server: String,

    /// One-time pairing code minted by `issue-pairing-code`.
    #[arg(long)]
    pub pairing_code: String,

    /// Stable local actor id to assign to this broker/device.
    #[arg(long)]
    pub actor_id: String,

    /// Override the config file path written for the broker actor.
    #[arg(long)]
    pub config_path: Option<String>,
}

#[derive(Args, Debug)]
pub struct CloudRunWorkflowArgs {
    /// Control plane base URL.
    #[arg(long, default_value = DEFAULT_CLOUD_SERVER_URL)]
    pub server: String,

    /// Paired actor id to target.
    #[arg(long)]
    pub actor_id: String,

    /// Path to the workflow JSON file.
    pub workflow_file: String,

    /// Parameters for the workflow (format: --param key=value).
    #[arg(long = "param", value_parser = super::parse_key_val::<String, String>)]
    pub params: Vec<(String, String)>,

    /// Optional explicit session id. Defaults to a fresh UUID.
    #[arg(long)]
    pub session_id: Option<String>,

    /// Default per-step timeout if the workflow step omits timeout_ms.
    #[arg(long, default_value_t = DEFAULT_WORKFLOW_TIMEOUT_MS)]
    pub timeout_ms: u64,
}

#[derive(Args, Debug)]
pub struct CloudGetRunArgs {
    /// Control plane base URL.
    #[arg(long, default_value = DEFAULT_CLOUD_SERVER_URL)]
    pub server: String,

    /// Run id returned from a previous hosted run.
    pub run_id: String,
}

#[derive(Args, Debug)]
pub struct CloudListActorsArgs {
    /// Control plane base URL.
    #[arg(long, default_value = DEFAULT_CLOUD_SERVER_URL)]
    pub server: String,
}

#[derive(Args, Debug)]
pub struct CloudExecCommandArgs {
    /// Control plane base URL.
    #[arg(long, default_value = DEFAULT_CLOUD_SERVER_URL)]
    pub server: String,

    /// Paired actor id to target.
    #[arg(long)]
    pub actor_id: String,

    /// Browser command to dispatch to the actor.
    #[arg(long)]
    pub cmd: String,

    /// Inline JSON payload for the browser command.
    #[arg(long, conflicts_with = "payload_file")]
    pub payload_json: Option<String>,

    /// File containing JSON payload for the browser command.
    #[arg(long)]
    pub payload_file: Option<String>,

    /// Inline JSON data blob for the browser command.
    #[arg(long, conflicts_with = "data_file")]
    pub data_json: Option<String>,

    /// File containing JSON data blob for the browser command.
    #[arg(long)]
    pub data_file: Option<String>,

    /// Inline JSON metadata stored on the cloud run record.
    #[arg(long, conflicts_with = "metadata_file")]
    pub metadata_json: Option<String>,

    /// File containing JSON metadata stored on the cloud run record.
    #[arg(long)]
    pub metadata_file: Option<String>,

    /// Optional explicit session id. Defaults to a fresh UUID.
    #[arg(long)]
    pub session_id: Option<String>,

    /// Optional explicit run id. Defaults to a fresh UUID.
    #[arg(long)]
    pub run_id: Option<String>,

    /// Command timeout budget.
    #[arg(long, default_value_t = DEFAULT_WORKFLOW_TIMEOUT_MS)]
    pub timeout_ms: u64,
}

#[derive(Args, Debug)]
pub struct CloudExecStepArgs {
    /// Control plane base URL.
    #[arg(long, default_value = DEFAULT_CLOUD_SERVER_URL)]
    pub server: String,

    /// Paired actor id to target.
    #[arg(long)]
    pub actor_id: String,

    /// Inline JSON step definition.
    #[arg(long, conflicts_with = "step_file")]
    pub step_json: Option<String>,

    /// File containing the JSON step definition.
    #[arg(long)]
    pub step_file: Option<String>,

    /// Bind the session to the currently active Chrome tab instead of creating a dedicated tab.
    #[arg(long)]
    pub use_current_tab: bool,

    /// Optional explicit session id. Defaults to a fresh UUID.
    #[arg(long)]
    pub session_id: Option<String>,

    /// Optional explicit run id. Defaults to a fresh UUID.
    #[arg(long)]
    pub run_id: Option<String>,

    /// Command timeout budget.
    #[arg(long, default_value_t = DEFAULT_WORKFLOW_TIMEOUT_MS)]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalCloudActorConfig {
    pub version: String,
    pub actor_id: String,
    pub workspace_id: String,
    pub actor_token: String,
    pub server_url: String,
    pub websocket_url: String,
    pub paired_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IssuePairingCodeRequest {
    workspace_id: String,
    ttl_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IssuePairingCodeResponse {
    pairing_code: String,
    workspace_id: String,
    expires_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RedeemPairingRequest {
    pairing_code: String,
    actor_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RedeemPairingResponse {
    actor_id: String,
    workspace_id: String,
    actor_token: String,
    server_url: String,
    websocket_url: String,
    paired_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunWorkflowRequest {
    actor_id: String,
    workflow: Value,
    #[serde(default)]
    parameters: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunWorkflowResponse {
    run: RunRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BrowserCommandRequest {
    actor_id: String,
    command: CloudBrowserCommandV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    side_effecting: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    idempotency_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BrowserCommandResponse {
    run: RunRecord,
    result: CloudCommandResultV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActorSummary {
    actor_id: String,
    workspace_id: String,
    paired_at_ms: u64,
    last_seen_ms: u64,
    connected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    connected_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extension_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    capabilities: Option<CapabilitiesV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ListActorsResponse {
    actors: Vec<ActorSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PairingCodeRecord {
    workspace_id: String,
    expires_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedActorRegistration {
    actor_id: String,
    workspace_id: String,
    actor_token: String,
    paired_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extension_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    capabilities: Option<CapabilitiesV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedControlPlaneState {
    version: String,
    #[serde(default)]
    actors: Vec<PersistedActorRegistration>,
    #[serde(default)]
    runs: Vec<RunRecord>,
}

#[derive(Debug, Clone)]
struct ActorConnection {
    sender: mpsc::UnboundedSender<String>,
    connected_at_ms: u64,
}

#[derive(Debug, Clone)]
struct ActorRegistration {
    actor_id: String,
    workspace_id: String,
    actor_token: String,
    paired_at_ms: u64,
    last_seen_ms: u64,
    extension_version: Option<String>,
    capabilities: Option<CapabilitiesV1>,
    metadata: Option<Value>,
    connection: Option<ActorConnection>,
}

#[derive(Debug, Clone)]
struct PendingCommand {
    actor_id: String,
    envelope: CloudCommandEnvelopeV1,
    queued_at_ms: u64,
    last_dispatched_at_ms: Option<u64>,
    accepted_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunRecord {
    run_id: String,
    actor_id: String,
    session_id: String,
    status: String,
    created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    finished_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workflow_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    steps: Vec<RunStepRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunStepRecord {
    step_index: usize,
    step_id: String,
    step_type: String,
    command_id: String,
    started_at_ms: u64,
    finished_at_ms: u64,
    success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ActorConnectQuery {
    actor_id: String,
}

#[derive(Default)]
struct ControlPlaneState {
    public_url: String,
    state_file: PathBuf,
    pairing_codes: RwLock<HashMap<String, PairingCodeRecord>>,
    actors: RwLock<HashMap<String, ActorRegistration>>,
    runs: RwLock<HashMap<String, RunRecord>>,
    command_waiters: Mutex<HashMap<String, oneshot::Sender<CloudCommandResultV1>>>,
    pending_commands: Mutex<HashMap<String, PendingCommand>>,
    persist_lock: Mutex<()>,
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

impl ControlPlaneState {
    async fn issue_pairing_code(
        &self,
        workspace_id: String,
        ttl_secs: u64,
    ) -> IssuePairingCodeResponse {
        let now = now_ms();
        let pairing_code = generate_pairing_code();
        let expires_at_ms = now.saturating_add(ttl_secs.max(1) * 1000);
        self.pairing_codes.write().await.insert(
            pairing_code.clone(),
            PairingCodeRecord {
                workspace_id: workspace_id.clone(),
                expires_at_ms,
            },
        );
        IssuePairingCodeResponse {
            pairing_code,
            workspace_id,
            expires_at_ms,
        }
    }

    async fn redeem_pairing_code(
        &self,
        request: RedeemPairingRequest,
    ) -> Result<RedeemPairingResponse> {
        let now = now_ms();
        let pairing_record = self
            .pairing_codes
            .write()
            .await
            .remove(&request.pairing_code)
            .ok_or_else(|| anyhow!("Unknown or already redeemed pairing code"))?;

        if pairing_record.expires_at_ms < now {
            bail!("Pairing code expired");
        }

        let actor_token = Uuid::new_v4().to_string();
        let paired_at_ms = now_ms();
        let websocket_url = websocket_url_from_server(&self.public_url)?;
        let actor = ActorRegistration {
            actor_id: request.actor_id.clone(),
            workspace_id: pairing_record.workspace_id.clone(),
            actor_token: actor_token.clone(),
            paired_at_ms,
            last_seen_ms: paired_at_ms,
            extension_version: None,
            capabilities: None,
            metadata: None,
            connection: None,
        };
        self.actors
            .write()
            .await
            .insert(request.actor_id.clone(), actor);
        self.persist_snapshot().await?;

        Ok(RedeemPairingResponse {
            actor_id: request.actor_id,
            workspace_id: pairing_record.workspace_id,
            actor_token,
            server_url: self.public_url.clone(),
            websocket_url,
            paired_at_ms,
        })
    }

    async fn actor_is_authorized(&self, actor_id: &str, actor_token: &str) -> bool {
        self.actors
            .read()
            .await
            .get(actor_id)
            .map(|actor| actor.actor_token == actor_token)
            .unwrap_or(false)
    }

    async fn actor_is_connected(&self, actor_id: &str) -> bool {
        self.actors
            .read()
            .await
            .get(actor_id)
            .and_then(|actor| actor.connection.as_ref())
            .is_some()
    }

    async fn list_actors(&self) -> Vec<ActorSummary> {
        let mut actors = self
            .actors
            .read()
            .await
            .values()
            .map(|actor| ActorSummary {
                actor_id: actor.actor_id.clone(),
                workspace_id: actor.workspace_id.clone(),
                paired_at_ms: actor.paired_at_ms,
                last_seen_ms: actor.last_seen_ms,
                connected: actor.connection.is_some(),
                connected_at_ms: actor.connection.as_ref().map(|conn| conn.connected_at_ms),
                extension_version: actor.extension_version.clone(),
                capabilities: actor.capabilities.clone(),
                metadata: actor.metadata.clone(),
            })
            .collect::<Vec<_>>();
        actors.sort_by(|left, right| left.actor_id.cmp(&right.actor_id));
        actors
    }

    async fn register_actor_connection(
        &self,
        hello: &ActorHelloV1,
        sender: mpsc::UnboundedSender<String>,
    ) -> Result<ActorReadyV1> {
        let mut actors = self.actors.write().await;
        let actor = actors
            .get_mut(&hello.actor_id)
            .ok_or_else(|| anyhow!("Actor {} is not paired", hello.actor_id))?;
        actor.last_seen_ms = now_ms();
        actor.extension_version = Some(hello.extension_version.clone());
        actor.capabilities = Some(hello.capabilities.clone());
        actor.metadata = hello.metadata.clone();
        let connection_id = Uuid::new_v4().to_string();
        actor.connection = Some(ActorConnection {
            sender,
            connected_at_ms: now_ms(),
        });

        Ok(ActorReadyV1 {
            version: CLOUD_CONTRACT_VERSION.to_string(),
            message_type: "actor.ready".to_string(),
            actor_id: hello.actor_id.clone(),
            heartbeat_interval_ms: DEFAULT_HEARTBEAT_INTERVAL_MS,
            resume_token: Some(connection_id),
            config: Some(json!({
                "public_url": self.public_url,
                "mode": "supervisor_first"
            })),
        })
    }

    async fn touch_actor(&self, actor_id: &str) {
        if let Some(actor) = self.actors.write().await.get_mut(actor_id) {
            actor.last_seen_ms = now_ms();
        }
    }

    async fn clear_actor_connection(&self, actor_id: &str) {
        if let Some(actor) = self.actors.write().await.get_mut(actor_id) {
            actor.connection = None;
            actor.last_seen_ms = now_ms();
        }
    }

    async fn mark_command_acked(&self, command_ack: &CloudCommandAckV1) {
        if let Some(pending) = self
            .pending_commands
            .lock()
            .await
            .get_mut(&command_ack.command_id)
        {
            pending.accepted_at_ms = Some(command_ack.accepted_at_ms);
        }
    }

    async fn complete_command(&self, command_result: CloudCommandResultV1) {
        self.pending_commands
            .lock()
            .await
            .remove(&command_result.command_id);
        if let Some(waiter) = self
            .command_waiters
            .lock()
            .await
            .remove(&command_result.command_id)
        {
            let _ = waiter.send(command_result);
        }
    }

    async fn dispatch_command(
        &self,
        actor_id: &str,
        envelope: CloudCommandEnvelopeV1,
        timeout: Duration,
    ) -> Result<CloudCommandResultV1> {
        if !self.actors.read().await.contains_key(actor_id) {
            bail!("Unknown actor {}", actor_id);
        }

        let payload = serde_json::to_string(&envelope)?;
        let (tx, rx) = oneshot::channel();
        self.pending_commands.lock().await.insert(
            envelope.command_id.clone(),
            PendingCommand {
                actor_id: actor_id.to_string(),
                envelope: envelope.clone(),
                queued_at_ms: now_ms(),
                last_dispatched_at_ms: None,
                accepted_at_ms: None,
            },
        );
        self.command_waiters
            .lock()
            .await
            .insert(envelope.command_id.clone(), tx);

        if !self
            .try_dispatch_pending_command(&envelope.command_id, Some(payload))
            .await?
        {
            super::write_log(
                "INFO",
                &format!(
                    "Queued command {} for actor {} until it reconnects",
                    envelope.command_id, actor_id
                ),
            );
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) => bail!("Actor {} dropped command result channel", actor_id),
            Err(_) => {
                self.pending_commands
                    .lock()
                    .await
                    .remove(&envelope.command_id);
                self.command_waiters
                    .lock()
                    .await
                    .remove(&envelope.command_id);
                bail!(
                    "Timed out waiting for command result {}",
                    envelope.command_id
                );
            }
        }
    }

    async fn try_dispatch_pending_command(
        &self,
        command_id: &str,
        pre_serialized_payload: Option<String>,
    ) -> Result<bool> {
        let (actor_id, payload) = {
            let mut pending = self.pending_commands.lock().await;
            let Some(entry) = pending.get_mut(command_id) else {
                return Ok(false);
            };
            entry.last_dispatched_at_ms = Some(now_ms());
            let payload = match pre_serialized_payload {
                Some(payload) => payload,
                None => serde_json::to_string(&entry.envelope)?,
            };
            (entry.actor_id.clone(), payload)
        };

        let sender = {
            let actors = self.actors.read().await;
            actors
                .get(&actor_id)
                .and_then(|actor| actor.connection.as_ref())
                .map(|conn| conn.sender.clone())
        };

        let Some(sender) = sender else {
            return Ok(false);
        };

        Ok(sender.send(payload).is_ok())
    }

    async fn resume_pending_commands(&self, actor_id: &str) -> Result<usize> {
        let command_ids = {
            let pending = self.pending_commands.lock().await;
            let mut queued: Vec<(u64, String)> = pending
                .values()
                .filter(|entry| entry.actor_id == actor_id)
                .map(|entry| (entry.queued_at_ms, entry.envelope.command_id.clone()))
                .collect();
            queued.sort_by_key(|(queued_at_ms, _)| *queued_at_ms);
            queued
                .into_iter()
                .map(|(_, command_id)| command_id)
                .collect::<Vec<_>>()
        };

        let mut resumed = 0usize;
        for command_id in command_ids {
            if self.try_dispatch_pending_command(&command_id, None).await? {
                resumed += 1;
            }
        }
        Ok(resumed)
    }

    async fn upsert_run(&self, run: &RunRecord) {
        self.runs
            .write()
            .await
            .insert(run.run_id.clone(), run.clone());
        if let Err(error) = self.persist_snapshot().await {
            super::write_log(
                "WARN",
                &format!("Failed to persist cloud run state: {}", error),
            );
        }
    }

    async fn get_run(&self, run_id: &str) -> Option<RunRecord> {
        self.runs.read().await.get(run_id).cloned()
    }

    async fn persist_snapshot(&self) -> Result<()> {
        if self.state_file.as_os_str().is_empty() {
            return Ok(());
        }

        let _guard = self.persist_lock.lock().await;
        let actors = self
            .actors
            .read()
            .await
            .values()
            .map(|actor| PersistedActorRegistration {
                actor_id: actor.actor_id.clone(),
                workspace_id: actor.workspace_id.clone(),
                actor_token: actor.actor_token.clone(),
                paired_at_ms: actor.paired_at_ms,
                extension_version: actor.extension_version.clone(),
                capabilities: actor.capabilities.clone(),
                metadata: actor.metadata.clone(),
            })
            .collect::<Vec<_>>();
        let runs = self.runs.read().await.values().cloned().collect::<Vec<_>>();
        let snapshot = PersistedControlPlaneState {
            version: CONTROL_PLANE_STATE_VERSION.to_string(),
            actors,
            runs,
        };

        let bytes = serde_json::to_vec_pretty(&snapshot)?;
        write_secret_file(&self.state_file, bytes)
            .with_context(|| format!("Write control plane state {}", self.state_file.display()))?;
        Ok(())
    }
}

pub async fn handle_cloud_commands(cmd: CloudCommands) -> Result<()> {
    match cmd {
        CloudCommands::Serve(args) => serve_control_plane(args).await,
        CloudCommands::IssuePairingCode(args) => issue_pairing_code(args).await,
        CloudCommands::Pair(args) => pair_actor(args).await,
        CloudCommands::RunWorkflow(args) => run_workflow(args).await,
        CloudCommands::GetRun(args) => get_run(args).await,
        CloudCommands::ListActors(args) => list_actors(args).await,
        CloudCommands::ExecCommand(args) => exec_command(args).await,
        CloudCommands::ExecStep(args) => exec_step(args).await,
    }
}

async fn serve_control_plane(args: CloudServeArgs) -> Result<()> {
    let listen_addr: SocketAddr = args
        .listen
        .parse()
        .with_context(|| format!("Invalid listen address {}", args.listen))?;

    let state = Arc::new(load_control_plane_state(&normalize_server_url(
        &args.public_url,
    )?)?);

    let app = Router::new()
        .route("/v1/pairing-codes", post(post_pairing_codes))
        .route("/v1/pair/redeem", post(post_pair_redeem))
        .route("/v1/actors", get(get_actors))
        .route("/v1/commands/browser", post(post_browser_command))
        .route("/v1/runs/workflow", post(post_runs_workflow))
        .route("/v1/runs/:run_id", get(get_run_status))
        .route("/v1/actors/connect", get(ws_actor_connect))
        .with_state(state.clone());

    let listener = TcpListener::bind(listen_addr)
        .await
        .with_context(|| format!("Bind control plane listener {}", listen_addr))?;

    println!("[CLOUD] listening on http://{}", listen_addr);
    println!("[CLOUD] public URL {}", state.public_url);
    println!(
        "[CLOUD] websocket {}",
        websocket_url_from_server(&state.public_url)?
    );

    axum::serve(listener, app)
        .await
        .context("Serve control plane")?;
    Ok(())
}

async fn issue_pairing_code(args: CloudIssuePairingCodeArgs) -> Result<()> {
    let request = IssuePairingCodeRequest {
        workspace_id: args.workspace_id,
        ttl_secs: args.ttl_secs,
    };
    let response: IssuePairingCodeResponse = cloud_http_client()
        .post(format!(
            "{}/v1/pairing-codes",
            normalize_server_url(&args.server)?
        ))
        .json(&request)
        .send()
        .await
        .context("POST /v1/pairing-codes")?
        .error_for_status()
        .context("Control plane rejected pairing code request")?
        .json()
        .await
        .context("Decode pairing code response")?;

    println!("pairing_code={}", response.pairing_code);
    println!("workspace_id={}", response.workspace_id);
    println!("expires_at_ms={}", response.expires_at_ms);
    Ok(())
}

async fn pair_actor(args: CloudPairArgs) -> Result<()> {
    let server_url = normalize_server_url(&args.server)?;
    let request = RedeemPairingRequest {
        pairing_code: args.pairing_code,
        actor_id: args.actor_id,
    };
    let response: RedeemPairingResponse = cloud_http_client()
        .post(format!("{}/v1/pair/redeem", server_url))
        .json(&request)
        .send()
        .await
        .context("POST /v1/pair/redeem")?
        .error_for_status()
        .context("Control plane rejected pairing request")?
        .json()
        .await
        .context("Decode pair response")?;

    let config = LocalCloudActorConfig {
        version: CLOUD_ACTOR_CONFIG_VERSION.to_string(),
        actor_id: response.actor_id.clone(),
        workspace_id: response.workspace_id.clone(),
        actor_token: response.actor_token.clone(),
        server_url: normalize_server_url(&response.server_url)?,
        websocket_url: normalize_websocket_url(&response.websocket_url)?,
        paired_at_ms: response.paired_at_ms,
        connect_timeout_ms: Some(15_000),
        request_timeout_ms: Some(DEFAULT_WORKFLOW_TIMEOUT_MS),
    };
    let path = resolve_cloud_actor_config_path(args.config_path.as_deref());
    persist_local_actor_config(&path, &config)?;

    println!("[PAIR] actor_id={}", response.actor_id);
    println!("[PAIR] workspace_id={}", response.workspace_id);
    println!("[PAIR] config_path={}", path.display());
    println!("[PAIR] websocket_url={}", response.websocket_url);
    Ok(())
}

async fn run_workflow(args: CloudRunWorkflowArgs) -> Result<()> {
    let workflow_path = PathBuf::from(&args.workflow_file);
    let workflow = load_json_file(&workflow_path)?;
    let request = RunWorkflowRequest {
        actor_id: args.actor_id,
        workflow,
        parameters: args.params.into_iter().collect(),
        session_id: args.session_id,
        timeout_ms: Some(args.timeout_ms.max(1)),
    };
    let response: RunWorkflowResponse = cloud_http_client()
        .post(format!(
            "{}/v1/runs/workflow",
            normalize_server_url(&args.server)?
        ))
        .json(&request)
        .send()
        .await
        .context("POST /v1/runs/workflow")?
        .error_for_status()
        .context("Hosted workflow run failed")?
        .json()
        .await
        .context("Decode run-workflow response")?;

    println!("{}", serde_json::to_string_pretty(&response.run)?);
    if response.run.status != "completed" {
        bail!(
            "Hosted workflow run {} finished with status {}",
            response.run.run_id,
            response.run.status
        );
    }
    Ok(())
}

async fn get_run(args: CloudGetRunArgs) -> Result<()> {
    let response: RunWorkflowResponse = cloud_http_client()
        .get(format!(
            "{}/v1/runs/{}",
            normalize_server_url(&args.server)?,
            args.run_id
        ))
        .send()
        .await
        .context("GET /v1/runs/:id")?
        .error_for_status()
        .context("Control plane could not fetch run")?
        .json()
        .await
        .context("Decode get-run response")?;

    println!("{}", serde_json::to_string_pretty(&response.run)?);
    Ok(())
}

async fn list_actors(args: CloudListActorsArgs) -> Result<()> {
    let response: ListActorsResponse = cloud_http_client()
        .get(format!("{}/v1/actors", normalize_server_url(&args.server)?))
        .send()
        .await
        .context("GET /v1/actors")?
        .error_for_status()
        .context("Control plane could not list actors")?
        .json()
        .await
        .context("Decode list-actors response")?;

    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

async fn exec_command(args: CloudExecCommandArgs) -> Result<()> {
    if args.cmd.trim().is_empty() {
        bail!("cmd is required");
    }

    let request = BrowserCommandRequest {
        actor_id: args.actor_id,
        command: CloudBrowserCommandV1 {
            cmd: args.cmd,
            payload: load_optional_json_input(
                args.payload_json.as_deref(),
                args.payload_file.as_deref(),
                "payload",
            )?,
            data: load_optional_json_input(
                args.data_json.as_deref(),
                args.data_file.as_deref(),
                "data",
            )?,
        },
        run_id: args.run_id,
        session_id: args.session_id,
        timeout_ms: Some(args.timeout_ms.max(1)),
        side_effecting: None,
        idempotency_policy: None,
        metadata: load_optional_json_input(
            args.metadata_json.as_deref(),
            args.metadata_file.as_deref(),
            "metadata",
        )?,
    };
    let response = post_browser_command_request(&args.server, &request).await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    if !response.result.success {
        bail!(
            "Hosted command {} failed for run {}",
            response.result.command_id,
            response.run.run_id
        );
    }
    Ok(())
}

async fn exec_step(args: CloudExecStepArgs) -> Result<()> {
    let step =
        load_required_json_input(args.step_json.as_deref(), args.step_file.as_deref(), "step")?;
    rzn_core::dsl::validate_action_value(&step)
        .map_err(|error| anyhow!("Step failed schema validation: {}", error))?;

    let mut payload = json!({
        "step": step,
    });
    if args.use_current_tab {
        payload["use_current_tab"] = Value::Bool(true);
    }

    let request = BrowserCommandRequest {
        actor_id: args.actor_id,
        command: CloudBrowserCommandV1 {
            cmd: "execute_step".to_string(),
            payload: Some(payload),
            data: None,
        },
        run_id: args.run_id,
        session_id: args.session_id,
        timeout_ms: Some(args.timeout_ms.max(1)),
        side_effecting: None,
        idempotency_policy: None,
        metadata: Some(json!({
            "source": "cloud.exec_step"
        })),
    };
    let response = post_browser_command_request(&args.server, &request).await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    if !response.result.success {
        bail!(
            "Hosted step {} failed for run {}",
            response.result.command_id,
            response.run.run_id
        );
    }
    Ok(())
}

async fn post_browser_command_request(
    server: &str,
    request: &BrowserCommandRequest,
) -> Result<BrowserCommandResponse> {
    cloud_http_client()
        .post(format!(
            "{}/v1/commands/browser",
            normalize_server_url(server)?
        ))
        .json(request)
        .send()
        .await
        .context("POST /v1/commands/browser")?
        .error_for_status()
        .context("Hosted browser command failed")?
        .json()
        .await
        .context("Decode exec-command response")
}

fn load_required_json_input(
    inline_json: Option<&str>,
    file_path: Option<&str>,
    label: &str,
) -> Result<Value> {
    load_optional_json_input(inline_json, file_path, label)?
        .ok_or_else(|| anyhow!("{} JSON is required", label))
}

fn load_optional_json_input(
    inline_json: Option<&str>,
    file_path: Option<&str>,
    label: &str,
) -> Result<Option<Value>> {
    match (inline_json, file_path) {
        (Some(_), Some(_)) => bail!("Specify only one of --{}-json or --{}-file", label, label),
        (Some(raw), None) => parse_inline_json(raw, label).map(Some),
        (None, Some(path)) => load_json_file(&PathBuf::from(path)).map(Some),
        (None, None) => Ok(None),
    }
}

fn parse_inline_json(raw: &str, label: &str) -> Result<Value> {
    if raw.trim() == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .with_context(|| format!("Read {} JSON from stdin", label))?;
        serde_json::from_str(&buffer).with_context(|| format!("Parse {} JSON from stdin", label))
    } else {
        serde_json::from_str(raw).with_context(|| format!("Parse inline {} JSON", label))
    }
}

async fn post_pairing_codes(
    State(state): State<Arc<ControlPlaneState>>,
    Json(request): Json<IssuePairingCodeRequest>,
) -> Result<Json<IssuePairingCodeResponse>, ApiError> {
    if request.workspace_id.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "workspace_id is required",
        ));
    }
    let response = state
        .issue_pairing_code(request.workspace_id, request.ttl_secs)
        .await;
    Ok(Json(response))
}

async fn post_pair_redeem(
    State(state): State<Arc<ControlPlaneState>>,
    Json(request): Json<RedeemPairingRequest>,
) -> Result<Json<RedeemPairingResponse>, ApiError> {
    if request.actor_id.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "actor_id is required",
        ));
    }
    state
        .redeem_pairing_code(request)
        .await
        .map(Json)
        .map_err(|err| ApiError::new(StatusCode::BAD_REQUEST, err.to_string()))
}

async fn get_actors(
    State(state): State<Arc<ControlPlaneState>>,
) -> Result<Json<ListActorsResponse>, ApiError> {
    Ok(Json(ListActorsResponse {
        actors: state.list_actors().await,
    }))
}

async fn post_browser_command(
    State(state): State<Arc<ControlPlaneState>>,
    Json(request): Json<BrowserCommandRequest>,
) -> Result<Json<BrowserCommandResponse>, ApiError> {
    if request.actor_id.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "actor_id is required",
        ));
    }
    if request.command.cmd.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "command.cmd is required",
        ));
    }
    if !state.actor_is_connected(&request.actor_id).await {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("Actor {} is not connected", request.actor_id),
        ));
    }

    perform_browser_command(state, request)
        .await
        .map(Json)
        .map_err(|err| ApiError::new(StatusCode::BAD_REQUEST, err.to_string()))
}

async fn post_runs_workflow(
    State(state): State<Arc<ControlPlaneState>>,
    Json(request): Json<RunWorkflowRequest>,
) -> Result<Json<RunWorkflowResponse>, ApiError> {
    if !state.actor_is_connected(&request.actor_id).await {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("Actor {} is not connected", request.actor_id),
        ));
    }

    perform_workflow_run(state, request)
        .await
        .map(|run| Json(RunWorkflowResponse { run }))
        .map_err(|err| ApiError::new(StatusCode::BAD_REQUEST, err.to_string()))
}

async fn get_run_status(
    State(state): State<Arc<ControlPlaneState>>,
    Path(run_id): Path<String>,
) -> Result<Json<RunWorkflowResponse>, ApiError> {
    let Some(run) = state.get_run(&run_id).await else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "run not found"));
    };
    Ok(Json(RunWorkflowResponse { run }))
}

async fn ws_actor_connect(
    State(state): State<Arc<ControlPlaneState>>,
    Query(query): Query<ActorConnectQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, ApiError> {
    let Some(actor_token) = bearer_token_from_headers(&headers) else {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "missing actor token",
        ));
    };
    if !state
        .actor_is_authorized(&query.actor_id, actor_token)
        .await
    {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid actor token",
        ));
    }

    Ok(ws.on_upgrade(move |socket| actor_socket(socket, state, query.actor_id)))
}

async fn actor_socket(socket: WebSocket, state: Arc<ControlPlaneState>, actor_id: String) {
    let (mut sink, mut stream) = socket.split();
    let (send_tx, mut send_rx) = mpsc::unbounded_channel::<String>();

    let writer = tokio::spawn(async move {
        while let Some(payload) = send_rx.recv().await {
            if sink.send(Message::Text(payload)).await.is_err() {
                break;
            }
        }
    });

    let hello = loop {
        match stream.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<ActorHelloV1>(&text) {
                Ok(hello) => break hello,
                Err(error) => {
                    super::write_log(
                        "WARN",
                        &format!("Ignoring invalid actor hello payload: {}", error),
                    );
                }
            },
            Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                writer.abort();
                state.clear_actor_connection(&actor_id).await;
                return;
            }
            _ => continue,
        }
    };

    if hello.actor_id != actor_id {
        writer.abort();
        state.clear_actor_connection(&actor_id).await;
        return;
    }

    let ready = match state
        .register_actor_connection(&hello, send_tx.clone())
        .await
    {
        Ok(ready) => ready,
        Err(error) => {
            writer.abort();
            state.clear_actor_connection(&actor_id).await;
            super::write_log(
                "ERROR",
                &format!("Failed to register actor {}: {}", actor_id, error),
            );
            return;
        }
    };

    if send_tx
        .send(
            serde_json::to_string(&ready)
                .unwrap_or_else(|_| "{\"type\":\"actor.ready\"}".to_string()),
        )
        .is_err()
    {
        writer.abort();
        state.clear_actor_connection(&actor_id).await;
        return;
    }

    if let Err(error) = state.resume_pending_commands(&actor_id).await {
        super::write_log(
            "WARN",
            &format!(
                "Failed to resume pending commands for actor {}: {}",
                actor_id, error
            ),
        );
    }

    while let Some(message) = stream.next().await {
        match message {
            Ok(Message::Text(text)) => {
                if let Ok(result) = serde_json::from_str::<CloudCommandResultV1>(&text) {
                    state.touch_actor(&actor_id).await;
                    state.complete_command(result).await;
                    continue;
                }
                if let Ok(ack) = serde_json::from_str::<CloudCommandAckV1>(&text) {
                    state.touch_actor(&actor_id).await;
                    state.mark_command_acked(&ack).await;
                    continue;
                }
            }
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {
                state.touch_actor(&actor_id).await;
            }
            Ok(Message::Close(_)) | Err(_) => break,
            _ => {}
        }
    }

    writer.abort();
    state.clear_actor_connection(&actor_id).await;
}

async fn perform_browser_command(
    state: Arc<ControlPlaneState>,
    request: BrowserCommandRequest,
) -> Result<BrowserCommandResponse> {
    let run_id = request.run_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let session_id = request
        .session_id
        .or_else(|| browser_command_session_id(&request.command))
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let normalized_command = normalize_browser_command_session(&request.command, &session_id);
    let timeout_ms = request
        .timeout_ms
        .unwrap_or(DEFAULT_WORKFLOW_TIMEOUT_MS)
        .max(1);
    let step_id = browser_command_step_id(&normalized_command);
    let step_type = browser_command_step_type(&normalized_command);
    let command_id = Uuid::new_v4().to_string();
    let created_at_ms = now_ms();
    let started_at_ms = created_at_ms;

    let mut run = RunRecord {
        run_id: run_id.clone(),
        actor_id: request.actor_id.clone(),
        session_id: session_id.clone(),
        status: "running".to_string(),
        created_at_ms,
        finished_at_ms: None,
        workflow_name: Some("cloud.exec_command".to_string()),
        error: None,
        steps: Vec::new(),
    };
    state.upsert_run(&run).await;

    let envelope = CloudCommandEnvelopeV1 {
        version: CLOUD_CONTRACT_VERSION.to_string(),
        message_type: "command.execute".to_string(),
        actor_id: request.actor_id.clone(),
        run_id: run_id.clone(),
        session_id: session_id.clone(),
        command_id: command_id.clone(),
        lease_id: Uuid::new_v4().to_string(),
        deadline_ms: now_ms().saturating_add(timeout_ms),
        trace_id: Some(run_id.clone()),
        parent_command_id: None,
        planner_step_index: Some(0),
        payload: CloudCommandPayloadV1 {
            kind: CloudCommandKindV1::BrowserCommand,
            command: Some(normalized_command.clone()),
            side_effecting: Some(
                request
                    .side_effecting
                    .unwrap_or_else(|| default_side_effecting_for_command(&normalized_command)),
            ),
            idempotency_policy: Some(
                request
                    .idempotency_policy
                    .unwrap_or_else(|| "single_delivery".to_string()),
            ),
            metadata: Some(build_browser_command_metadata(
                &normalized_command,
                request.metadata,
            )),
        },
    };

    let result = match state
        .dispatch_command(
            &request.actor_id,
            envelope.clone(),
            Duration::from_millis(timeout_ms.saturating_add(5_000)),
        )
        .await
    {
        Ok(result) => result,
        Err(error) => {
            failed_command_result(&envelope, "CONTROL_PLANE_DISPATCH_ERROR", error.to_string())
        }
    };

    let finished_at_ms = result.finished_at_ms;
    let error = result
        .error
        .clone()
        .or_else(|| result.result.as_ref().and_then(|inner| inner.error.clone()));

    run.steps.push(RunStepRecord {
        step_index: 0,
        step_id,
        step_type,
        command_id,
        started_at_ms,
        finished_at_ms,
        success: result.success,
        result: serde_json::to_value(&result).ok(),
        error: error.clone(),
    });
    run.status = if result.success {
        "completed".to_string()
    } else {
        "failed".to_string()
    };
    run.finished_at_ms = Some(finished_at_ms);
    run.error = error;
    state.upsert_run(&run).await;

    Ok(BrowserCommandResponse { run, result })
}

async fn perform_workflow_run(
    state: Arc<ControlPlaneState>,
    request: RunWorkflowRequest,
) -> Result<RunRecord> {
    let mut workflow = request.workflow;
    workflow = apply_parameters(workflow, &request.parameters);
    validate_required_params(&workflow, &request.parameters)?;
    let steps = extract_steps(&workflow)?;
    validate_steps(&steps)?;
    let prefer_current_tab = workflow_prefers_current_tab(&workflow);

    let run_id = Uuid::new_v4().to_string();
    let session_id = request
        .session_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let created_at_ms = now_ms();
    let mut run = RunRecord {
        run_id: run_id.clone(),
        actor_id: request.actor_id.clone(),
        session_id: session_id.clone(),
        status: "running".to_string(),
        created_at_ms,
        finished_at_ms: None,
        workflow_name: workflow
            .get("name")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        error: None,
        steps: Vec::new(),
    };
    state.upsert_run(&run).await;

    let default_timeout_ms = request
        .timeout_ms
        .unwrap_or(DEFAULT_WORKFLOW_TIMEOUT_MS)
        .max(1);

    for (step_index, step) in steps.iter().enumerate() {
        let step_id = step
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or("step")
            .to_string();
        let step_type = step
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_string();
        let step_timeout_ms = step
            .get("timeout_ms")
            .and_then(|value| value.as_u64())
            .or_else(|| step.get("timeoutMs").and_then(|value| value.as_u64()))
            .unwrap_or(default_timeout_ms)
            .max(1);
        let command_id = Uuid::new_v4().to_string();
        let started_at_ms = now_ms();

        if step_type == "wait_for_timeout" {
            tokio::time::sleep(Duration::from_millis(step_timeout_ms)).await;
            let finished_at_ms = now_ms();
            run.steps.push(RunStepRecord {
                step_index,
                step_id,
                step_type,
                command_id,
                started_at_ms,
                finished_at_ms,
                success: true,
                result: Some(json!({ "waited_ms": step_timeout_ms })),
                error: None,
            });
            state.upsert_run(&run).await;
            continue;
        }

        let payload = step_execution_payload(Some(&session_id), step, prefer_current_tab);
        let envelope = CloudCommandEnvelopeV1 {
            version: CLOUD_CONTRACT_VERSION.to_string(),
            message_type: "command.execute".to_string(),
            actor_id: request.actor_id.clone(),
            run_id: run_id.clone(),
            session_id: session_id.clone(),
            command_id: command_id.clone(),
            lease_id: Uuid::new_v4().to_string(),
            deadline_ms: now_ms().saturating_add(step_timeout_ms),
            trace_id: Some(run_id.clone()),
            parent_command_id: None,
            planner_step_index: Some(step_index as u32),
            payload: CloudCommandPayloadV1 {
                kind: CloudCommandKindV1::BrowserCommand,
                command: Some(rzn_contracts::v1::CloudBrowserCommandV1 {
                    cmd: "execute_step".to_string(),
                    payload: Some(payload),
                    data: None,
                }),
                side_effecting: Some(true),
                idempotency_policy: Some("single_delivery".to_string()),
                metadata: Some(json!({
                    "step_id": step_id,
                    "step_type": step_type,
                    "step_index": step_index
                })),
            },
        };

        let result = state
            .dispatch_command(
                &request.actor_id,
                envelope,
                Duration::from_millis(step_timeout_ms.saturating_add(5_000)),
            )
            .await;

        match result {
            Ok(command_result) => {
                let finished_at_ms = command_result.finished_at_ms;
                let success = command_result.success;
                let result_value = serde_json::to_value(&command_result).ok();
                let error = command_result.error.clone().or_else(|| {
                    command_result
                        .result
                        .as_ref()
                        .and_then(|result| result.error.clone())
                });

                run.steps.push(RunStepRecord {
                    step_index,
                    step_id: step_id.clone(),
                    step_type: step_type.clone(),
                    command_id: command_id.clone(),
                    started_at_ms,
                    finished_at_ms,
                    success,
                    result: result_value,
                    error: error.clone(),
                });

                if !success {
                    run.status = "failed".to_string();
                    run.finished_at_ms = Some(finished_at_ms);
                    run.error = error.or_else(|| Some(format!("Step {} failed", step_id)));
                    state.upsert_run(&run).await;
                    return Ok(run);
                }
            }
            Err(error) => {
                run.steps.push(RunStepRecord {
                    step_index,
                    step_id: step_id.clone(),
                    step_type: step_type.clone(),
                    command_id: command_id.clone(),
                    started_at_ms,
                    finished_at_ms: now_ms(),
                    success: false,
                    result: None,
                    error: Some(error.to_string()),
                });
                run.status = "failed".to_string();
                run.finished_at_ms = Some(now_ms());
                run.error = Some(error.to_string());
                state.upsert_run(&run).await;
                return Ok(run);
            }
        }

        state.upsert_run(&run).await;
    }

    run.status = "completed".to_string();
    run.finished_at_ms = Some(now_ms());
    state.upsert_run(&run).await;
    Ok(run)
}

fn browser_command_step_id(command: &CloudBrowserCommandV1) -> String {
    command
        .payload
        .as_ref()
        .and_then(|payload| payload.pointer("/step/id"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| command.cmd.clone())
}

fn browser_command_session_id(command: &CloudBrowserCommandV1) -> Option<String> {
    command
        .payload
        .as_ref()
        .and_then(|payload| payload.get("session_id"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn normalize_browser_command_session(
    command: &CloudBrowserCommandV1,
    session_id: &str,
) -> CloudBrowserCommandV1 {
    let mut normalized = command.clone();
    match normalized.payload.as_mut() {
        Some(Value::Object(payload)) => {
            payload.insert(
                "session_id".to_string(),
                Value::String(session_id.to_string()),
            );
        }
        Some(_) => {}
        None => {
            normalized.payload = Some(json!({
                "session_id": session_id
            }));
        }
    }
    normalized
}

fn browser_command_step_type(command: &CloudBrowserCommandV1) -> String {
    command
        .payload
        .as_ref()
        .and_then(|payload| payload.pointer("/step/type"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| command.cmd.clone())
}

fn default_side_effecting_for_command(command: &CloudBrowserCommandV1) -> bool {
    match command.cmd.as_str() {
        "get_active_tab"
        | "get_dom_snapshot"
        | "get_pruned_dom"
        | "get_dom_hash"
        | "process_dom"
        | "detect_auto_list"
        | "execute_extraction_plan"
        | "observe" => false,
        "execute_step" => !matches!(
            browser_command_step_type(command).as_str(),
            "get_current_url"
                | "get_page_source"
                | "get_dom_snapshot"
                | "wait_for_navigation"
                | "wait_for_network_idle"
                | "take_screenshot"
                | "observe"
        ),
        _ => true,
    }
}

fn build_browser_command_metadata(
    command: &CloudBrowserCommandV1,
    request_metadata: Option<Value>,
) -> Value {
    let mut metadata = serde_json::Map::new();
    metadata.insert("command".to_string(), Value::String(command.cmd.clone()));
    metadata.insert(
        "step_id".to_string(),
        Value::String(browser_command_step_id(command)),
    );
    metadata.insert(
        "step_type".to_string(),
        Value::String(browser_command_step_type(command)),
    );

    match request_metadata {
        Some(Value::Object(entries)) => {
            metadata.extend(entries);
        }
        Some(other) => {
            metadata.insert("request_metadata".to_string(), other);
        }
        None => {}
    }

    Value::Object(metadata)
}

fn failed_command_result(
    envelope: &CloudCommandEnvelopeV1,
    error_code: &str,
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
            error_code: Some(error_code.to_string()),
            error: Some(error.clone()),
            current_url: None,
            current_tab_id: None,
            current_tab_ref: None,
            dom_hash: None,
            dom_snapshot: None,
            capabilities: None,
            raw: None,
        }),
        error: Some(error),
    }
}

fn normalize_server_url(server: &str) -> Result<String> {
    let mut url = Url::parse(server).with_context(|| format!("Invalid server URL {}", server))?;
    validate_server_url(&url)?;
    if url.path() == "/" {
        url.set_path("");
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

pub fn resolve_cloud_actor_config_path(path_override: Option<&str>) -> PathBuf {
    if let Some(path) = path_override {
        return PathBuf::from(path);
    }
    if let Ok(path) = std::env::var("RZN_CLOUD_CONFIG_PATH") {
        let candidate = PathBuf::from(path);
        if is_allowed_cloud_config_path(&candidate) {
            return candidate;
        }
        eprintln!(
            "[CLOUD] ignoring RZN_CLOUD_CONFIG_PATH outside per-user config dir: {}",
            candidate.display()
        );
    }
    default_cloud_config_dir().join("cloud_actor_v1.json")
}

fn default_cloud_config_dir() -> PathBuf {
    dirs::config_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rzn")
}

fn is_allowed_cloud_config_path(path: &FsPath) -> bool {
    let Some(base) = normalize_path_for_prefix(&default_cloud_config_dir()) else {
        return false;
    };
    let Some(candidate) = normalize_path_for_prefix(path) else {
        return false;
    };
    candidate.starts_with(base)
}

fn normalize_path_for_prefix(path: &FsPath) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
        }
    }
    Some(normalized)
}

fn persist_local_actor_config(path: &FsPath, config: &LocalCloudActorConfig) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(config)?;
    write_secret_file(path, bytes).with_context(|| format!("Write actor config {}", path.display()))
}

fn load_json_file(path: &FsPath) -> Result<Value> {
    let bytes =
        std::fs::read(path).with_context(|| format!("Read workflow file {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("Parse workflow JSON {}", path.display()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn generate_pairing_code() -> String {
    let raw = Uuid::new_v4().simple().to_string().to_uppercase();
    raw.chars().take(8).collect()
}

fn websocket_url_from_server(server_url: &str) -> Result<String> {
    let mut url = Url::parse(server_url)?;
    let scheme = match url.scheme() {
        "https" => "wss",
        "http" if is_loopback_url(&url) => "ws",
        "http" => bail!("Cloud server URL must use https unless it targets loopback"),
        other => bail!("Unsupported cloud server URL scheme {}", other),
    };
    url.set_scheme(scheme)
        .map_err(|_| anyhow!("Failed to convert {} to websocket URL", server_url))?;
    url.set_path("/v1/actors/connect");
    url.set_query(None);
    Ok(url.to_string())
}

fn validate_server_url(url: &Url) -> Result<()> {
    match url.scheme() {
        "https" => Ok(()),
        "http" if is_loopback_url(url) => Ok(()),
        "http" => bail!("Cloud server URL must use https unless it targets loopback"),
        other => bail!("Unsupported cloud server URL scheme {}", other),
    }
}

fn validate_websocket_url(url: &Url) -> Result<()> {
    match url.scheme() {
        "wss" => Ok(()),
        "ws" if is_loopback_url(url) => Ok(()),
        "ws" => bail!("Cloud websocket URL must use wss unless it targets loopback"),
        other => bail!("Unsupported cloud websocket URL scheme {}", other),
    }
}

fn normalize_websocket_url(websocket_url: &str) -> Result<String> {
    let mut url = Url::parse(websocket_url.trim())
        .with_context(|| format!("Invalid cloud websocket URL {}", websocket_url))?;
    validate_websocket_url(&url)?;
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

fn is_loopback_url(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    host == "localhost"
        || host
            .parse::<IpAddr>()
            .map(|addr| addr.is_loopback())
            .unwrap_or(false)
}

fn bearer_token_from_headers(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn resolve_control_plane_state_path() -> PathBuf {
    if let Ok(path) = std::env::var("RZN_CLOUD_SERVER_STATE_PATH") {
        return PathBuf::from(path);
    }
    let base = dirs::config_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("rzn").join("cloud_control_plane_state_v1.json")
}

fn load_control_plane_state(public_url: &str) -> Result<ControlPlaneState> {
    let state_file = resolve_control_plane_state_path();
    let mut state = ControlPlaneState {
        public_url: public_url.to_string(),
        state_file: state_file.clone(),
        ..Default::default()
    };

    if !state_file.exists() {
        return Ok(state);
    }

    let bytes = std::fs::read(&state_file)
        .with_context(|| format!("Read control plane state {}", state_file.display()))?;
    let persisted: PersistedControlPlaneState =
        serde_json::from_slice(&bytes).context("Parse control plane state JSON")?;
    if persisted.version != CONTROL_PLANE_STATE_VERSION {
        bail!(
            "Unsupported control plane state version {} in {}",
            persisted.version,
            state_file.display()
        );
    }

    state.actors = RwLock::new(
        persisted
            .actors
            .into_iter()
            .map(|actor| {
                (
                    actor.actor_id.clone(),
                    ActorRegistration {
                        actor_id: actor.actor_id,
                        workspace_id: actor.workspace_id,
                        actor_token: actor.actor_token,
                        paired_at_ms: actor.paired_at_ms,
                        last_seen_ms: actor.paired_at_ms,
                        extension_version: actor.extension_version,
                        capabilities: actor.capabilities,
                        metadata: actor.metadata,
                        connection: None,
                    },
                )
            })
            .collect(),
    );
    state.runs = RwLock::new(
        persisted
            .runs
            .into_iter()
            .map(|run| (run.run_id.clone(), run))
            .collect(),
    );
    Ok(state)
}

fn validate_required_params(workflow: &Value, params: &HashMap<String, String>) -> Result<()> {
    let required = workflow
        .pointer("/browser_automation/sequences/0/required_variables")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    let mut missing = Vec::new();
    for var in required {
        if let Some(name) = var.get("name").and_then(|value| value.as_str()) {
            if !params.contains_key(name) {
                missing.push(name.to_string());
            }
        }
    }
    if !missing.is_empty() {
        bail!("Missing required parameters: {}", missing.join(", "));
    }
    Ok(())
}

fn extract_steps(workflow: &Value) -> Result<Vec<Value>> {
    workflow
        .pointer("/browser_automation/sequences/0/steps")
        .and_then(|value| value.as_array())
        .cloned()
        .ok_or_else(|| anyhow!("Workflow missing browser_automation.sequences[0].steps"))
}

fn workflow_prefers_current_tab(workflow: &Value) -> bool {
    workflow
        .pointer("/browser_automation/use_current_tab")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
        || workflow
            .pointer("/browser_automation/use_active_tab")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
}

fn step_execution_payload(
    session_id: Option<&str>,
    step: &Value,
    prefer_current_tab: bool,
) -> Value {
    let mut payload = json!({
        "step": step,
    });
    if let Some(session_id) = session_id {
        payload["session_id"] = Value::String(session_id.to_string());
    }
    if prefer_current_tab {
        payload["use_current_tab"] = Value::Bool(true);
    }
    payload
}

fn validate_steps(steps: &[Value]) -> Result<()> {
    for (index, step) in steps.iter().enumerate() {
        if let Err(error) = rzn_core::dsl::validate_action_value(step) {
            bail!("Step {} failed schema validation: {}", index + 1, error);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rzn_contracts::v1::{ActionResultV1, CloudBrowserCommandV1};

    #[test]
    fn websocket_url_uses_wss_except_loopback() {
        assert_eq!(
            websocket_url_from_server("http://127.0.0.1:8787").unwrap(),
            "ws://127.0.0.1:8787/v1/actors/connect"
        );
        assert_eq!(
            websocket_url_from_server("https://cloud.example.com").unwrap(),
            "wss://cloud.example.com/v1/actors/connect"
        );
        assert!(websocket_url_from_server("http://cloud.example.com").is_err());
    }

    #[test]
    fn cloud_config_path_rejects_parent_traversal_after_allowed_prefix() {
        let base = default_cloud_config_dir();

        assert!(is_allowed_cloud_config_path(
            &base.join("cloud_actor_v1.json")
        ));
        assert!(!is_allowed_cloud_config_path(
            &base.join("rzn").join("..").join("..").join("outside.json")
        ));
    }

    #[test]
    fn actor_bearer_token_is_read_from_header() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Bearer token-1".parse().unwrap());

        assert_eq!(bearer_token_from_headers(&headers), Some("token-1"));
    }

    #[test]
    fn actor_bearer_token_rejects_missing_or_malformed_header() {
        let mut headers = HeaderMap::new();

        assert_eq!(bearer_token_from_headers(&headers), None);
        headers.insert(AUTHORIZATION, "token-1".parse().unwrap());
        assert_eq!(bearer_token_from_headers(&headers), None);
    }

    #[test]
    fn apply_parameters_substitutes_strings_and_script_params() {
        let workflow = json!({
            "browser_automation": {
                "sequences": [{
                    "steps": [{
                        "type": "execute_javascript",
                        "script": "return params.query"
                    }, {
                        "type": "navigate_to_url",
                        "url": "https://example.com?q={query}"
                    }]
                }]
            }
        });
        let params = HashMap::from([("query".to_string(), "rust".to_string())]);

        let applied = apply_parameters(workflow, &params);
        assert_eq!(
            applied
                .pointer("/browser_automation/sequences/0/steps/1/url")
                .and_then(|value| value.as_str()),
            Some("https://example.com?q=rust")
        );
        assert_eq!(
            applied
                .pointer("/browser_automation/sequences/0/steps/0/params/query")
                .and_then(|value| value.as_str()),
            Some("rust")
        );
    }

    #[test]
    fn apply_parameters_expands_chained_param_defaults() {
        let workflow = json!({
            "browser_automation": {
                "sequences": [{
                    "steps": [{
                        "type": "navigate_to_url",
                        "url": "{app_url}"
                    }]
                }]
            }
        });
        let params = HashMap::from([
            (
                "app_url".to_string(),
                "https://apps.apple.com/{country}/app/id{app_id}".to_string(),
            ),
            ("country".to_string(), "us".to_string()),
            ("app_id".to_string(), "123456789".to_string()),
        ]);

        let applied = apply_parameters(workflow, &params);

        assert_eq!(
            applied
                .pointer("/browser_automation/sequences/0/steps/0/url")
                .and_then(|value| value.as_str()),
            Some("https://apps.apple.com/us/app/id123456789")
        );
    }

    fn sample_hello(actor_id: &str) -> ActorHelloV1 {
        ActorHelloV1 {
            version: CLOUD_CONTRACT_VERSION.to_string(),
            message_type: "actor.hello".to_string(),
            actor_id: actor_id.to_string(),
            workspace_id: "workspace-1".to_string(),
            extension_version: "0.1.0".to_string(),
            capabilities: CapabilitiesV1 {
                extension_actor: true,
                cdp_available: true,
                cdp_enabled: false,
                cdp_attached: false,
            },
            metadata: None,
        }
    }

    fn sample_envelope(actor_id: &str, command_id: &str) -> CloudCommandEnvelopeV1 {
        CloudCommandEnvelopeV1 {
            version: CLOUD_CONTRACT_VERSION.to_string(),
            message_type: "command.execute".to_string(),
            actor_id: actor_id.to_string(),
            run_id: "run-1".to_string(),
            session_id: "session-1".to_string(),
            command_id: command_id.to_string(),
            lease_id: "lease-1".to_string(),
            deadline_ms: now_ms() + 5_000,
            trace_id: Some("trace-1".to_string()),
            parent_command_id: None,
            planner_step_index: Some(0),
            payload: CloudCommandPayloadV1 {
                kind: CloudCommandKindV1::BrowserCommand,
                command: Some(CloudBrowserCommandV1 {
                    cmd: "execute_step".to_string(),
                    payload: Some(json!({
                        "session_id": "session-1",
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

    fn sample_browser_command_request(actor_id: &str) -> BrowserCommandRequest {
        BrowserCommandRequest {
            actor_id: actor_id.to_string(),
            command: CloudBrowserCommandV1 {
                cmd: "execute_step".to_string(),
                payload: Some(json!({
                    "session_id": "session-1",
                    "step": {
                        "id": "step-1",
                        "type": "get_current_url"
                    }
                })),
                data: None,
            },
            run_id: Some("run-1".to_string()),
            session_id: Some("session-1".to_string()),
            timeout_ms: Some(1_000),
            side_effecting: Some(false),
            idempotency_policy: Some("single_delivery".to_string()),
            metadata: Some(json!({
                "source": "test"
            })),
        }
    }

    fn sample_result(actor_id: &str, command_id: &str) -> CloudCommandResultV1 {
        sample_result_with_session(actor_id, command_id, "session-1")
    }

    fn sample_result_with_session(
        actor_id: &str,
        command_id: &str,
        session_id: &str,
    ) -> CloudCommandResultV1 {
        CloudCommandResultV1 {
            version: CLOUD_CONTRACT_VERSION.to_string(),
            message_type: "command.result".to_string(),
            actor_id: actor_id.to_string(),
            run_id: "run-1".to_string(),
            session_id: session_id.to_string(),
            command_id: command_id.to_string(),
            lease_id: "lease-1".to_string(),
            success: true,
            finished_at_ms: now_ms(),
            trace_id: Some("trace-1".to_string()),
            result: Some(ActionResultV1 {
                success: true,
                error_code: None,
                error: None,
                current_url: Some("https://example.com".to_string()),
                current_tab_id: Some(1),
                current_tab_ref: None,
                dom_hash: None,
                dom_snapshot: None,
                capabilities: None,
                raw: None,
            }),
            error: None,
        }
    }

    async fn test_state_with_actor(actor_id: &str) -> Arc<ControlPlaneState> {
        let state = Arc::new(ControlPlaneState {
            public_url: normalize_server_url("http://127.0.0.1:8787").unwrap(),
            ..Default::default()
        });
        state.actors.write().await.insert(
            actor_id.to_string(),
            ActorRegistration {
                actor_id: actor_id.to_string(),
                workspace_id: "workspace-1".to_string(),
                actor_token: "token-1".to_string(),
                paired_at_ms: now_ms(),
                last_seen_ms: now_ms(),
                extension_version: None,
                capabilities: None,
                metadata: None,
                connection: None,
            },
        );
        state
    }

    #[tokio::test]
    async fn pending_commands_resume_after_actor_reconnect() {
        let actor_id = "actor-1";
        let state = test_state_with_actor(actor_id).await;
        let hello = sample_hello(actor_id);

        let (tx1, mut rx1) = mpsc::unbounded_channel::<String>();
        state.register_actor_connection(&hello, tx1).await.unwrap();

        let envelope = sample_envelope(actor_id, "command-1");
        let state_for_dispatch = state.clone();
        let envelope_for_dispatch = envelope.clone();
        let dispatch_task = tokio::spawn(async move {
            state_for_dispatch
                .dispatch_command(actor_id, envelope_for_dispatch, Duration::from_secs(2))
                .await
        });

        let first_payload = tokio::time::timeout(Duration::from_millis(250), rx1.recv())
            .await
            .unwrap()
            .unwrap();
        let first_envelope: CloudCommandEnvelopeV1 = serde_json::from_str(&first_payload).unwrap();
        assert_eq!(first_envelope.command_id, "command-1");

        state.clear_actor_connection(actor_id).await;

        let (tx2, mut rx2) = mpsc::unbounded_channel::<String>();
        state.register_actor_connection(&hello, tx2).await.unwrap();
        assert_eq!(state.resume_pending_commands(actor_id).await.unwrap(), 1);

        let second_payload = tokio::time::timeout(Duration::from_millis(250), rx2.recv())
            .await
            .unwrap()
            .unwrap();
        let second_envelope: CloudCommandEnvelopeV1 =
            serde_json::from_str(&second_payload).unwrap();
        assert_eq!(second_envelope.command_id, "command-1");

        state
            .complete_command(sample_result(actor_id, "command-1"))
            .await;

        let result = dispatch_task.await.unwrap().unwrap();
        assert_eq!(result.command_id, "command-1");
        assert!(state.pending_commands.lock().await.is_empty());
    }

    #[tokio::test]
    async fn command_ack_updates_pending_lease_state() {
        let actor_id = "actor-1";
        let state = test_state_with_actor(actor_id).await;
        let command_id = "command-ack-1";
        state.pending_commands.lock().await.insert(
            command_id.to_string(),
            PendingCommand {
                actor_id: actor_id.to_string(),
                envelope: sample_envelope(actor_id, command_id),
                queued_at_ms: now_ms(),
                last_dispatched_at_ms: None,
                accepted_at_ms: None,
            },
        );

        state
            .mark_command_acked(&CloudCommandAckV1 {
                version: CLOUD_CONTRACT_VERSION.to_string(),
                message_type: "command.ack".to_string(),
                actor_id: actor_id.to_string(),
                run_id: "run-1".to_string(),
                session_id: "session-1".to_string(),
                command_id: command_id.to_string(),
                lease_id: "lease-1".to_string(),
                accepted_at_ms: 1234,
                trace_id: Some("trace-1".to_string()),
            })
            .await;

        let pending = state.pending_commands.lock().await;
        assert_eq!(
            pending
                .get(command_id)
                .and_then(|entry| entry.accepted_at_ms),
            Some(1234)
        );
    }

    #[tokio::test]
    async fn list_actors_reports_connection_state() {
        let actor_id = "actor-1";
        let state = test_state_with_actor(actor_id).await;

        let before = state.list_actors().await;
        assert_eq!(before.len(), 1);
        assert!(!before[0].connected);
        assert_eq!(before[0].connected_at_ms, None);

        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        state
            .register_actor_connection(&sample_hello(actor_id), tx)
            .await
            .unwrap();

        let after = state.list_actors().await;
        assert!(after[0].connected);
        assert!(after[0].connected_at_ms.is_some());
        assert_eq!(after[0].extension_version.as_deref(), Some("0.1.0"));
    }

    #[tokio::test]
    async fn perform_browser_command_records_single_step_run() {
        let actor_id = "actor-1";
        let state = test_state_with_actor(actor_id).await;
        let hello = sample_hello(actor_id);
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        state.register_actor_connection(&hello, tx).await.unwrap();

        let state_for_command = state.clone();
        let request = sample_browser_command_request(actor_id);
        let command_task =
            tokio::spawn(async move { perform_browser_command(state_for_command, request).await });

        let first_payload = tokio::time::timeout(Duration::from_millis(250), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let envelope: CloudCommandEnvelopeV1 = serde_json::from_str(&first_payload).unwrap();
        assert_eq!(
            envelope
                .payload
                .command
                .as_ref()
                .map(|command| command.cmd.as_str()),
            Some("execute_step")
        );
        assert_eq!(
            envelope.payload.metadata.as_ref().and_then(|metadata| {
                metadata.get("step_type").and_then(|value| value.as_str())
            }),
            Some("get_current_url")
        );

        state
            .complete_command(sample_result_with_session(
                actor_id,
                &envelope.command_id,
                "payload-session-1",
            ))
            .await;

        let response = command_task.await.unwrap().unwrap();
        assert!(response.result.success);
        assert_eq!(response.run.status, "completed");
        assert_eq!(
            response.run.workflow_name.as_deref(),
            Some("cloud.exec_command")
        );
        assert_eq!(response.run.steps.len(), 1);
        assert_eq!(response.run.steps[0].step_id, "step-1");
        assert_eq!(response.run.steps[0].step_type, "get_current_url");
        assert!(state.get_run(&response.run.run_id).await.is_some());
    }

    #[tokio::test]
    async fn perform_browser_command_adopts_payload_session_id() {
        let actor_id = "actor-1";
        let state = test_state_with_actor(actor_id).await;
        let hello = sample_hello(actor_id);
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        state.register_actor_connection(&hello, tx).await.unwrap();

        let mut request = sample_browser_command_request(actor_id);
        request.session_id = None;
        request.command.payload = Some(json!({
            "session_id": "payload-session-1",
            "step": {
                "type": "get_current_url"
            }
        }));

        let state_for_command = state.clone();
        let command_task =
            tokio::spawn(async move { perform_browser_command(state_for_command, request).await });

        let first_payload = tokio::time::timeout(Duration::from_millis(250), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let envelope: CloudCommandEnvelopeV1 = serde_json::from_str(&first_payload).unwrap();
        assert_eq!(envelope.session_id, "payload-session-1");
        assert_eq!(
            envelope
                .payload
                .command
                .as_ref()
                .and_then(|command| command.payload.as_ref())
                .and_then(|payload| payload.get("session_id"))
                .and_then(|value| value.as_str()),
            Some("payload-session-1")
        );

        state
            .complete_command(sample_result_with_session(
                actor_id,
                &envelope.command_id,
                "payload-session-1",
            ))
            .await;

        let response = command_task.await.unwrap().unwrap();
        assert_eq!(response.run.session_id, "payload-session-1");
        assert_eq!(response.result.session_id, "payload-session-1");
    }
}
