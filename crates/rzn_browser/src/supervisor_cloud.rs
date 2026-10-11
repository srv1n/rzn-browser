use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use rzn_contracts::browser::{
    ActorHello, ActorReady, BrowserActionResult, Capabilities, CloudCommandAck,
    CloudCommandEnvelope, CloudCommandKind, CloudCommandResult, CLOUD_CONTRACT,
};
use rzn_core::runtime_paths::default_app_base_dir;
use rzn_core::secure_files::{ensure_private_dir, set_secret_file_permissions, write_secret_file};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::net::IpAddr;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::http::{HeaderValue, Request};
use tokio_tungstenite::tungstenite::Message;
use url::Url;

use crate::cloud::{resolve_cloud_actor_config_path, LocalCloudActorConfig};

const CLOUD_ACTOR_CONFIG_CONTRACT: &str = "rzn.cloud.actor_config";
const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 15_000;
const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 45_000;
const DEFAULT_EXTENSION_RPC_GRACE_MS: u64 = 5_000;
const CONFIG_RELOAD_INTERVAL_MS: u64 = 5_000;
const MAX_CACHED_COMMAND_RESULTS: usize = 256;
const COMMAND_RECEIPT_FILE: &str = "cloud_command_receipts.json";
const COMMAND_RECEIPT_VERSION: u32 = 1;
const MAX_COMMAND_RECEIPT_FILE_BYTES: u64 = 64 * 1024 * 1024;
const RECEIPT_REOPEN_BACKOFF: Duration = Duration::from_secs(60);
static RECEIPT_TMP_SEQ: AtomicU64 = AtomicU64::new(0);
const MAX_QUEUED_CLOUD_MESSAGES: usize = 256;
/// Client keepalive: ping cadence and the silence that forces a reconnect.
const CLOUD_PING_INTERVAL: Duration = Duration::from_secs(20);
const CLOUD_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) struct CloudDispatchRequest {
    pub envelope: CloudCommandEnvelope,
    pub default_request_timeout_ms: u64,
    pub respond_to: oneshot::Sender<CloudCommandResult>,
}

#[derive(Clone)]
pub(crate) struct SupervisorCloudActor {
    status: Arc<Mutex<CloudActorRuntimeState>>,
    control_tx: mpsc::UnboundedSender<CloudActorControlMessage>,
}

#[derive(Debug, Clone, Default)]
struct CloudActorRuntimeState {
    config: Option<LocalCloudActorConfig>,
    connected: bool,
    last_connected_at_ms: Option<u64>,
    last_ready_at_ms: Option<u64>,
    last_error: Option<String>,
    last_command_id: Option<String>,
    last_result_at_ms: Option<u64>,
    inflight_command_id: Option<String>,
    dedupe_cache_size: usize,
    command_receipt_error: Option<String>,
}

#[derive(Debug)]
enum CloudActorControlMessage {
    ApplyConfig {
        next_config: Option<LocalCloudActorConfig>,
        respond_to: Option<oneshot::Sender<()>>,
    },
}

#[derive(Default)]
struct CommandResultCache {
    order: Vec<String>,
    entries: HashMap<String, CloudCommandResult>,
    fingerprints: HashMap<String, String>,
    inflight: HashMap<String, watch::Receiver<Option<CloudCommandResult>>>,
    inflight_fingerprints: HashMap<String, String>,
    generation: u64,
    active_scope: Option<String>,
    receipts: Option<CommandReceiptJournal>,
    persistence_error: Option<String>,
    receipt_path: Option<PathBuf>,
    journal_retry_after: Option<Instant>,
}

impl CommandResultCache {
    fn get(&self, command_id: &str) -> Option<CloudCommandResult> {
        self.entries.get(command_id).cloned()
    }

    fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
        self.fingerprints.clear();
        self.inflight.clear();
        self.inflight_fingerprints.clear();
        self.generation = self.generation.wrapping_add(1);
        self.active_scope = None;
    }

    fn activate_scope(&mut self, scope: String) {
        if self.active_scope.as_deref() == Some(scope.as_str()) {
            return;
        }
        self.order.clear();
        self.entries.clear();
        self.fingerprints.clear();
        self.inflight.clear();
        self.inflight_fingerprints.clear();
        self.generation = self.generation.wrapping_add(1);
        self.active_scope = Some(scope);
    }

    fn insert(&mut self, command_id: String, fingerprint: String, result: CloudCommandResult) {
        if !self.entries.contains_key(&command_id) {
            self.order.push(command_id.clone());
        }
        self.entries.insert(command_id.clone(), result);
        self.fingerprints.insert(command_id.clone(), fingerprint);

        while self.order.len() > MAX_CACHED_COMMAND_RESULTS {
            let evicted = self.order.remove(0);
            self.entries.remove(&evicted);
            self.fingerprints.remove(&evicted);
        }
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CommandReceiptFile {
    version: u32,
    entries: Vec<CommandReceipt>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CommandReceipt {
    scope: String,
    command_id: String,
    fingerprint: String,
    deadline_ms: u64,
    identity: ReceiptIdentity,
    state: CommandReceiptState,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReceiptIdentity {
    actor_id: String,
    run_id: String,
    session_id: String,
    command_id: String,
    lease_id: String,
    trace_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state", content = "result", rename_all = "snake_case")]
enum CommandReceiptState {
    Accepted,
    Completed(CloudCommandResult),
}

struct CommandReceiptJournal {
    path: PathBuf,
    entries: Vec<CommandReceipt>,
    lock: Arc<CommandReceiptJournalLock>,
    #[cfg(test)]
    persist_test_hook: Option<(Arc<AtomicBool>, Arc<AtomicU64>, Duration)>,
}

struct CommandReceiptJournalLock {
    _file: fs::File,
}

impl CommandReceiptJournal {
    fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            ensure_private_dir(parent).context("Secure cloud command receipt directory")?;
        }
        let lock = acquire_command_receipt_journal_lock(&path)?;
        let (entries, recovered) = read_command_receipt_entries(&path, true)?;
        let journal = Self {
            path,
            entries,
            lock,
            #[cfg(test)]
            persist_test_hook: None,
        };
        if recovered {
            journal.persist()?;
        }
        Ok(journal)
    }

    async fn reload_async(&mut self) -> Result<()> {
        let path = self.path.clone();
        let lock = self.lock.clone();
        let allow_missing = self.entries.is_empty();
        let (entries, recovered) = tokio::task::spawn_blocking(move || {
            let _lock = lock;
            read_command_receipt_entries(&path, allow_missing)
        })
        .await
        .context("Cloud command receipt reload task failed")??;
        self.entries = entries;
        if recovered {
            self.persist_async().await?;
        }
        Ok(())
    }

    fn find(&self, scope: &str, command_id: &str) -> Option<&CommandReceipt> {
        self.entries
            .iter()
            .find(|entry| entry.scope == scope && entry.command_id == command_id)
    }

    fn prune_expired_completed(&mut self, now: u64) {
        // Cloud command_id and deadline_ms are immutable across server retries.
        // After the deadline, a replay is rejected before admission, so expired
        // terminal receipts are safe to remove from the bounded local cache.
        self.entries.retain(|entry| {
            entry.deadline_ms > now || matches!(entry.state, CommandReceiptState::Accepted)
        });
    }

    async fn accept(&mut self, receipt: CommandReceipt) -> Result<()> {
        self.entries.push(receipt);
        if let Err(error) = self.persist_async().await {
            self.entries.pop();
            return Err(error);
        }
        Ok(())
    }

    async fn complete(
        &mut self,
        scope: &str,
        command_id: &str,
        result: CloudCommandResult,
    ) -> Result<()> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.scope == scope && entry.command_id == command_id)
            .context("Accepted cloud command receipt disappeared")?;
        let previous = std::mem::replace(
            &mut self.entries[index].state,
            CommandReceiptState::Completed(result),
        );
        if let Err(error) = self.persist_async().await {
            self.entries[index].state = previous;
            return Err(error);
        }
        Ok(())
    }

    async fn persist_async(&self) -> Result<()> {
        let path = self.path.clone();
        let lock = self.lock.clone();
        let file = CommandReceiptFile {
            version: COMMAND_RECEIPT_VERSION,
            entries: self.entries.clone(),
        };
        #[cfg(test)]
        let test_hook = self.persist_test_hook.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = lock;
            #[cfg(test)]
            if let Some((started, writes, delay)) = test_hook {
                let write_number = writes.fetch_add(1, Ordering::AcqRel);
                if write_number == 0 {
                    started.store(true, Ordering::Release);
                    std::thread::sleep(delay);
                }
            }
            persist_receipt_file(&path, &file)
        })
        .await
        .context("Cloud command receipt writer task failed")??;
        Ok(())
    }

    fn persist(&self) -> Result<()> {
        let file = CommandReceiptFile {
            version: COMMAND_RECEIPT_VERSION,
            entries: self.entries.clone(),
        };
        persist_receipt_file(&self.path, &file).with_context(|| {
            format!(
                "Persist cloud command receipt journal {}",
                self.path.display()
            )
        })
    }
}

// Keep this lock separate from the atomically replaced journal. Locking the
// journal inode itself would stop protecting the path after rename.
fn acquire_command_receipt_journal_lock(path: &Path) -> Result<Arc<CommandReceiptJournalLock>> {
    let lock_path = path.with_extension("lock");
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(&lock_path)
        .with_context(|| format!("Open cloud command receipt lock {}", lock_path.display()))?;
    set_secret_file_permissions(&lock_path).context("Secure cloud command receipt lock")?;
    file.try_lock().map_err(|error| {
        anyhow!(
            "Cloud command receipt journal is already owned or cannot be locked ({}): {error}",
            lock_path.display()
        )
    })?;
    Ok(Arc::new(CommandReceiptJournalLock { _file: file }))
}

fn read_command_receipt_entries(
    path: &Path,
    allow_missing: bool,
) -> Result<(Vec<CommandReceipt>, bool)> {
    if let Ok(metadata) = fs::metadata(path) {
        if metadata.len() > MAX_COMMAND_RECEIPT_FILE_BYTES {
            bail!("Cloud command receipt journal exceeds its byte limit");
        }
    }
    let mut entries = match fs::read(path) {
        Ok(bytes) => {
            set_secret_file_permissions(path)
                .context("Secure cloud command receipt file permissions")?;
            let file: CommandReceiptFile =
                serde_json::from_slice(&bytes).context("Parse cloud command receipt journal")?;
            if file.version != COMMAND_RECEIPT_VERSION {
                bail!("Unsupported cloud command receipt journal version");
            }
            if file.entries.len() > MAX_CACHED_COMMAND_RESULTS {
                bail!("Cloud command receipt journal exceeds its capacity");
            }
            let mut keys = std::collections::HashSet::new();
            for entry in &file.entries {
                if !keys.insert((entry.scope.clone(), entry.command_id.clone())) {
                    bail!("Cloud command receipt journal contains duplicate keys");
                }
            }
            file.entries
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound && allow_missing => Vec::new(),
        Err(error) => return Err(error).context("Read cloud command receipt journal"),
    };

    // Any accepted receipt found at process startup or recovery has no live
    // dispatcher. Convert it to a terminal result so commands are never replayed.
    let mut recovered = false;
    for entry in &mut entries {
        if matches!(entry.state, CommandReceiptState::Accepted) {
            entry.state = CommandReceiptState::Completed(indeterminate_receipt_result(
                &entry.identity,
                "Supervisor restarted while this command was in flight; its outcome is indeterminate and it was not replayed.",
            ));
            recovered = true;
        }
    }
    Ok((entries, recovered))
}

fn persist_receipt_file(path: &Path, file: &CommandReceiptFile) -> Result<()> {
    let bytes = serde_json::to_vec(file)?;
    if bytes.len() as u64 > MAX_COMMAND_RECEIPT_FILE_BYTES {
        bail!("Cloud command receipt journal exceeds its byte limit");
    }
    write_receipt_file_atomic(path, &bytes)
        .with_context(|| format!("Persist cloud command receipt journal {}", path.display()))
}

fn open_command_result_cache() -> CommandResultCache {
    let path = default_app_base_dir()
        .join("secure")
        .join(COMMAND_RECEIPT_FILE);
    match CommandReceiptJournal::open(path) {
        Ok(receipts) => CommandResultCache {
            receipt_path: Some(receipts.path.clone()),
            receipts: Some(receipts),
            ..CommandResultCache::default()
        },
        Err(error) => CommandResultCache {
            receipt_path: Some(
                default_app_base_dir()
                    .join("secure")
                    .join(COMMAND_RECEIPT_FILE),
            ),
            persistence_error: Some(error.to_string()),
            journal_retry_after: Some(Instant::now() + RECEIPT_REOPEN_BACKOFF),
            ..CommandResultCache::default()
        },
    }
}

async fn retry_receipt_journal(cache: &mut CommandResultCache) -> bool {
    if cache.persistence_error.is_none() {
        return true;
    }
    cache
        .inflight
        .retain(|_, receiver| receiver.has_changed().is_ok());
    if !cache.inflight.is_empty() {
        cache.journal_retry_after = Some(Instant::now() + RECEIPT_REOPEN_BACKOFF);
        return false;
    }
    if cache
        .journal_retry_after
        .is_some_and(|retry_after| Instant::now() < retry_after)
    {
        return false;
    }
    let Some(path) = cache.receipt_path.clone() else {
        return false;
    };
    let retry = if let Some(journal) = cache.receipts.as_mut() {
        journal.reload_async().await.map(|()| None)
    } else {
        tokio::task::spawn_blocking(move || CommandReceiptJournal::open(path))
            .await
            .context("Cloud command receipt reopen task failed")
            .and_then(|result| result)
            .map(Some)
    };
    match retry {
        Ok(Some(receipts)) => {
            cache.receipts = Some(receipts);
            cache.persistence_error = None;
            cache.journal_retry_after = None;
            true
        }
        Ok(None) => {
            cache.persistence_error = None;
            cache.journal_retry_after = None;
            true
        }
        Err(error) => {
            cache.persistence_error = Some(error.to_string());
            cache.journal_retry_after = Some(Instant::now() + RECEIPT_REOPEN_BACKOFF);
            false
        }
    }
}

fn write_receipt_file_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "receipt path has no parent"))?;
    ensure_private_dir(parent)?;
    let sequence = RECEIPT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("receipts");
    let tmp = parent.join(format!(
        ".{file_name}.{}.{nonce}.{sequence}.tmp",
        std::process::id()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let write_result = (|| {
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        set_secret_file_permissions(path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_result
}

fn receipt_scope(config: &LocalCloudActorConfig) -> String {
    let mut digest = Sha256::new();
    digest.update(b"rzn-cloud-command-receipt-v1\0");
    digest.update(config.actor_id.as_bytes());
    digest.update([0]);
    digest.update(config.workspace_id.as_bytes());
    digest.update([0]);
    digest.update(config.server_url.as_bytes());
    format!("{:x}", digest.finalize())
}

fn receipt_identity(envelope: &CloudCommandEnvelope) -> ReceiptIdentity {
    ReceiptIdentity {
        actor_id: envelope.actor_id.clone(),
        run_id: envelope.run_id.clone(),
        session_id: envelope.session_id.clone(),
        command_id: envelope.command_id.clone(),
        lease_id: envelope.lease_id.clone(),
        trace_id: envelope.trace_id.clone(),
    }
}

/// A redelivery may carry a new lease, but command identity and effects must
/// otherwise remain fixed for the lifetime of command_id. Top-level session
/// affinity is authoritative, so ignore any stale nested payload copy.
fn command_fingerprint(envelope: &CloudCommandEnvelope) -> String {
    let mut payload = serde_json::to_value(&envelope.payload).unwrap_or(Value::Null);
    if let Some(command_payload) = payload
        .pointer_mut("/command/payload")
        .and_then(Value::as_object_mut)
    {
        command_payload.remove("session_id");
    }
    let identity = json!({
        "version": &envelope.version,
        "type": &envelope.message_type,
        "actor_id": &envelope.actor_id,
        "run_id": &envelope.run_id,
        "session_id": &envelope.session_id,
        "command_id": &envelope.command_id,
        "deadline_ms": envelope.deadline_ms,
        "trace_id": &envelope.trace_id,
        "parent_command_id": &envelope.parent_command_id,
        "planner_step_index": envelope.planner_step_index,
        "payload": payload,
    });
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&identity).expect("serialize cloud command identity"))
    )
}

fn command_id_conflict_result(envelope: &CloudCommandEnvelope) -> CloudCommandResult {
    let mut result = error_command_result(
        envelope,
        "Cloud command_id was reused with a different command identity; command not dispatched",
    );
    if let Some(action) = result.result.as_mut() {
        action.error_code = Some("COMMAND_ID_CONFLICT".to_string());
    }
    result
}

fn indeterminate_receipt_result(identity: &ReceiptIdentity, error: &str) -> CloudCommandResult {
    CloudCommandResult {
        version: CLOUD_CONTRACT.to_string(),
        message_type: "command.result".to_string(),
        actor_id: identity.actor_id.clone(),
        run_id: identity.run_id.clone(),
        session_id: identity.session_id.clone(),
        command_id: identity.command_id.clone(),
        lease_id: identity.lease_id.clone(),
        trace_id: identity.trace_id.clone(),
        success: false,
        finished_at_ms: now_ms(),
        result: Some(BrowserActionResult {
            success: false,
            error_code: Some("COMMAND_INDETERMINATE".to_string()),
            error: Some(error.to_string()),
            current_url: None,
            current_tab_id: None,
            current_tab_ref: None,
            dom_hash: None,
            dom_snapshot: None,
            capabilities: None,
            raw: Some(json!({"runtime_owner":"supervisor","error":error})),
        }),
        error: Some(error.to_string()),
    }
}

fn expired_command_result(envelope: &CloudCommandEnvelope, error: &str) -> CloudCommandResult {
    let mut result = error_command_result(envelope, error);
    if let Some(action) = result.result.as_mut() {
        action.error_code = Some("COMMAND_EXPIRED".to_string());
    }
    result
}

pub(crate) fn spawn_cloud_actor(
    dispatch_tx: mpsc::UnboundedSender<CloudDispatchRequest>,
) -> SupervisorCloudActor {
    let (config, mut last_error) = match load_local_actor_config() {
        Ok(config) => (config, None),
        Err(error) => (None, Some(error.to_string())),
    };
    let result_cache = open_command_result_cache();
    let command_receipt_error = result_cache.persistence_error.clone();
    if last_error.is_none() {
        last_error = result_cache
            .persistence_error
            .clone()
            .map(|error| format!("Cloud command receipt journal unavailable: {error}"));
    }
    let status = Arc::new(Mutex::new(CloudActorRuntimeState {
        config: config.clone(),
        connected: false,
        last_connected_at_ms: None,
        last_ready_at_ms: None,
        last_error,
        last_command_id: None,
        last_result_at_ms: None,
        inflight_command_id: None,
        dedupe_cache_size: 0,
        command_receipt_error,
    }));
    let (control_tx, control_rx) = mpsc::unbounded_channel::<CloudActorControlMessage>();
    let result_cache = Arc::new(Mutex::new(result_cache));

    tokio::spawn(run_cloud_actor_loop(
        config,
        control_rx,
        dispatch_tx,
        result_cache,
        status.clone(),
    ));

    SupervisorCloudActor { status, control_tx }
}

pub(crate) fn disabled_status(native_host_bridge_connected: bool) -> Value {
    json!({
        "supported": true,
        "actor_mode": "supervisor",
        "runtime_owner": "supervisor",
        "lifecycle": "not_started_in_this_process",
        "config_path": resolve_cloud_actor_config_path(None).to_string_lossy(),
        "configured": false,
        "connected": false,
        "native_host_bridge_connected": native_host_bridge_connected,
        "spool_depth": 0,
        "dedupe_cache_size": 0
    })
}

impl SupervisorCloudActor {
    pub(crate) async fn status(&self, native_host_bridge_connected: bool) -> Value {
        let state = self.status.lock().await.clone();
        json!({
            "supported": true,
            "actor_mode": "supervisor",
            "runtime_owner": "supervisor",
            "config_path": resolve_cloud_actor_config_path(None).to_string_lossy(),
            "configured": state.config.is_some(),
            "connected": state.connected,
            "native_host_bridge_connected": native_host_bridge_connected,
            "actor_id": state.config.as_ref().map(|config| config.actor_id.clone()),
            "workspace_id": state.config.as_ref().map(|config| config.workspace_id.clone()),
            "server_url": state.config.as_ref().map(|config| config.server_url.clone()),
            "websocket_url": state.config.as_ref().map(|config| config.websocket_url.clone()),
            "paired_at_ms": state.config.as_ref().map(|config| config.paired_at_ms),
            "connect_timeout_ms": state.config.as_ref().and_then(|config| config.connect_timeout_ms),
            "request_timeout_ms": state.config.as_ref().and_then(|config| config.request_timeout_ms),
            "last_connected_at_ms": state.last_connected_at_ms,
            "last_ready_at_ms": state.last_ready_at_ms,
            "last_command_id": state.last_command_id,
            "last_result_at_ms": state.last_result_at_ms,
            "last_error": state.last_error,
            "spool_depth": usize::from(state.inflight_command_id.is_some()),
            "inflight_command_id": state.inflight_command_id,
            "dedupe_cache_size": state.dedupe_cache_size,
            "command_receipt_journal_healthy": state.command_receipt_error.is_none(),
            "command_receipt_journal_error": state.command_receipt_error
        })
    }

    pub(crate) async fn apply_config_value(
        &self,
        value: Value,
        native_host_bridge_connected: bool,
    ) -> Result<Value> {
        let config_value = value.get("config").cloned().unwrap_or(value);
        let config: LocalCloudActorConfig =
            serde_json::from_value(config_value).context("Parse cloud actor config")?;
        let config = normalize_local_actor_config(config)?;
        persist_local_actor_config(&resolve_cloud_actor_config_path(None), &config)?;
        let (ack_tx, ack_rx) = oneshot::channel();
        self.control_tx
            .send(CloudActorControlMessage::ApplyConfig {
                next_config: Some(config),
                respond_to: Some(ack_tx),
            })
            .map_err(|_| anyhow!("Supervisor cloud actor loop is unavailable"))?;
        wait_for_config_apply_ack(ack_rx).await?;
        Ok(self.status(native_host_bridge_connected).await)
    }

    pub(crate) async fn clear_config(&self, native_host_bridge_connected: bool) -> Result<Value> {
        remove_local_actor_config(&resolve_cloud_actor_config_path(None))?;
        let (ack_tx, ack_rx) = oneshot::channel();
        self.control_tx
            .send(CloudActorControlMessage::ApplyConfig {
                next_config: None,
                respond_to: Some(ack_tx),
            })
            .map_err(|_| anyhow!("Supervisor cloud actor loop is unavailable"))?;
        wait_for_config_apply_ack(ack_rx).await?;
        Ok(self.status(native_host_bridge_connected).await)
    }
}

async fn wait_for_config_apply_ack(ack_rx: oneshot::Receiver<()>) -> Result<()> {
    match tokio::time::timeout(Duration::from_secs(2), ack_rx).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(anyhow!(
            "Supervisor cloud actor loop closed before applying config"
        )),
        Err(_) => Err(anyhow!("Timed out applying supervisor cloud actor config")),
    }
}

async fn run_cloud_actor_loop(
    mut current_config: Option<LocalCloudActorConfig>,
    mut control_rx: mpsc::UnboundedReceiver<CloudActorControlMessage>,
    dispatch_tx: mpsc::UnboundedSender<CloudDispatchRequest>,
    result_cache: Arc<Mutex<CommandResultCache>>,
    status: Arc<Mutex<CloudActorRuntimeState>>,
) {
    let mut reconnect_backoff = Duration::from_secs(1);

    loop {
        let Some(config) = current_config.clone() else {
            tokio::select! {
                control = control_rx.recv() => {
                    let Some(control) = control else { return; };
                    current_config = apply_cloud_control_message(control, &status, &result_cache).await;
                    reconnect_backoff = Duration::from_secs(1);
                }
                _ = tokio::time::sleep(Duration::from_millis(CONFIG_RELOAD_INTERVAL_MS)) => {
                    match load_local_actor_config() {
                        Ok(config) => {
                            if config.is_some() {
                                current_config = config.clone();
                                let mut guard = status.lock().await;
                                guard.config = config;
                                guard.last_error = None;
                            }
                        }
                        Err(error) => update_cloud_status_error(&status, error.to_string()).await,
                    }
                }
            }
            continue;
        };

        match wait_for_receipt_journal_or_control(&result_cache, &status, &mut control_rx).await {
            ReceiptJournalGateOutcome::Ready => {}
            ReceiptJournalGateOutcome::RetryLater => continue,
            ReceiptJournalGateOutcome::Control(control) => {
                current_config = apply_cloud_control_message(control, &status, &result_cache).await;
                reconnect_backoff = Duration::from_secs(1);
                continue;
            }
            ReceiptJournalGateOutcome::ControlClosed => return,
        }

        let connect_timeout_ms = config
            .connect_timeout_ms
            .unwrap_or(DEFAULT_CONNECT_TIMEOUT_MS)
            .max(1);
        let connect_request = match build_actor_ws_request(&config) {
            Ok(request) => request,
            Err(error) => {
                update_cloud_status_error(&status, error.to_string()).await;
                match wait_for_backoff_or_control(reconnect_backoff, &mut control_rx).await {
                    BackoffOutcome::Control(control) => {
                        current_config =
                            apply_cloud_control_message(control, &status, &result_cache).await;
                        reconnect_backoff = Duration::from_secs(1);
                    }
                    BackoffOutcome::ControlClosed => return,
                    BackoffOutcome::Elapsed => {
                        reconnect_backoff = next_backoff(reconnect_backoff);
                    }
                }
                continue;
            }
        };

        match wait_for_cloud_connection_or_control(
            Duration::from_millis(connect_timeout_ms),
            connect_async(connect_request),
            &mut control_rx,
        )
        .await
        {
            ConnectPhaseOutcome::Connected((socket, _)) => {
                mark_cloud_actor_connected(&status).await;
                reconnect_backoff = Duration::from_secs(1);
                let mut session_task = tokio::spawn(run_cloud_actor_session(
                    socket,
                    config.clone(),
                    dispatch_tx.clone(),
                    result_cache.clone(),
                    status.clone(),
                ));
                tokio::select! {
                    result = &mut session_task => {
                        mark_cloud_actor_disconnected(&status).await;
                        match result {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => {
                                update_cloud_status_error(&status, error.to_string()).await;
                            }
                            Err(error) => {
                                update_cloud_status_error(&status, error.to_string()).await;
                            }
                        }
                    }
                    control = control_rx.recv() => {
                        let Some(control) = control else { return; };
                        session_task.abort();
                        mark_cloud_actor_disconnected(&status).await;
                        current_config = apply_cloud_control_message(control, &status, &result_cache).await;
                        reconnect_backoff = Duration::from_secs(1);
                        continue;
                    }
                }
            }
            ConnectPhaseOutcome::ConnectFailed(error) => {
                update_cloud_status_error(&status, error).await;
            }
            ConnectPhaseOutcome::ConnectTimedOut => {
                update_cloud_status_error(
                    &status,
                    "Cloud actor websocket connect timed out".to_string(),
                )
                .await;
            }
            ConnectPhaseOutcome::Control(control) => {
                current_config = apply_cloud_control_message(control, &status, &result_cache).await;
                reconnect_backoff = Duration::from_secs(1);
                continue;
            }
            ConnectPhaseOutcome::ControlClosed => return,
        }

        match wait_for_backoff_or_control(reconnect_backoff, &mut control_rx).await {
            BackoffOutcome::Control(control) => {
                current_config = apply_cloud_control_message(control, &status, &result_cache).await;
                reconnect_backoff = Duration::from_secs(1);
            }
            BackoffOutcome::ControlClosed => return,
            BackoffOutcome::Elapsed => {
                reconnect_backoff = next_backoff(reconnect_backoff);
            }
        }
    }
}

enum ReceiptJournalGateOutcome {
    Ready,
    RetryLater,
    Control(CloudActorControlMessage),
    ControlClosed,
}

async fn wait_for_receipt_journal_or_control(
    result_cache: &Arc<Mutex<CommandResultCache>>,
    status: &Arc<Mutex<CloudActorRuntimeState>>,
    control_rx: &mut mpsc::UnboundedReceiver<CloudActorControlMessage>,
) -> ReceiptJournalGateOutcome {
    let retry_delay = {
        let cache = result_cache.lock().await;
        if cache.persistence_error.is_none() {
            return ReceiptJournalGateOutcome::Ready;
        }
        cache
            .journal_retry_after
            .map(|retry_after| retry_after.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::ZERO)
    };

    match wait_for_backoff_or_control(retry_delay, control_rx).await {
        BackoffOutcome::Control(control) => ReceiptJournalGateOutcome::Control(control),
        BackoffOutcome::ControlClosed => ReceiptJournalGateOutcome::ControlClosed,
        BackoffOutcome::Elapsed => {
            let mut cache = result_cache.lock().await;
            let recovered = retry_receipt_journal(&mut cache).await;
            let error = cache.persistence_error.clone();
            drop(cache);
            status.lock().await.command_receipt_error = error;
            if recovered {
                ReceiptJournalGateOutcome::Ready
            } else {
                ReceiptJournalGateOutcome::RetryLater
            }
        }
    }
}

enum BackoffOutcome {
    Elapsed,
    Control(CloudActorControlMessage),
    ControlClosed,
}

async fn wait_for_backoff_or_control(
    backoff: Duration,
    control_rx: &mut mpsc::UnboundedReceiver<CloudActorControlMessage>,
) -> BackoffOutcome {
    tokio::select! {
        control = control_rx.recv() => {
            match control {
                Some(control) => BackoffOutcome::Control(control),
                None => BackoffOutcome::ControlClosed,
            }
        }
        _ = tokio::time::sleep(backoff) => BackoffOutcome::Elapsed,
    }
}

enum ConnectPhaseOutcome<T> {
    Connected(T),
    ConnectFailed(String),
    ConnectTimedOut,
    Control(CloudActorControlMessage),
    ControlClosed,
}

async fn wait_for_cloud_connection_or_control<F, T, E>(
    connect_timeout: Duration,
    connect_future: F,
    control_rx: &mut mpsc::UnboundedReceiver<CloudActorControlMessage>,
) -> ConnectPhaseOutcome<T>
where
    F: std::future::Future<Output = std::result::Result<T, E>>,
    E: std::fmt::Display,
{
    tokio::select! {
        connection = tokio::time::timeout(connect_timeout, connect_future) => {
            match connection {
                Ok(Ok(value)) => ConnectPhaseOutcome::Connected(value),
                Ok(Err(error)) => ConnectPhaseOutcome::ConnectFailed(error.to_string()),
                Err(_) => ConnectPhaseOutcome::ConnectTimedOut,
            }
        }
        control = control_rx.recv() => {
            match control {
                Some(control) => ConnectPhaseOutcome::Control(control),
                None => ConnectPhaseOutcome::ControlClosed,
            }
        }
    }
}

async fn run_cloud_actor_session<S>(
    socket: tokio_tungstenite::WebSocketStream<S>,
    config: LocalCloudActorConfig,
    dispatch_tx: mpsc::UnboundedSender<CloudDispatchRequest>,
    result_cache: Arc<Mutex<CommandResultCache>>,
    status: Arc<Mutex<CloudActorRuntimeState>>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    run_cloud_actor_session_with(
        socket,
        config,
        dispatch_tx,
        result_cache,
        status,
        CLOUD_PING_INTERVAL,
        CLOUD_IDLE_TIMEOUT,
    )
    .await
}

/// Session body with injectable keepalive timings (tests use milliseconds).
///
/// The read loop only services the socket: it pings every `ping_every`, and
/// errors out (→ the caller's reconnect path) when nothing at all arrives for
/// `idle_timeout`, so a half-open TCP connection (laptop sleep) cannot keep
/// status "connected" forever. Commands run on a separate worker task, one at
/// a time in arrival order (ack, then result — same wire order as before), so
/// a long command never stalls pings/pongs.
async fn run_cloud_actor_session_with<S>(
    mut socket: tokio_tungstenite::WebSocketStream<S>,
    config: LocalCloudActorConfig,
    dispatch_tx: mpsc::UnboundedSender<CloudDispatchRequest>,
    result_cache: Arc<Mutex<CommandResultCache>>,
    status: Arc<Mutex<CloudActorRuntimeState>>,
    ping_every: Duration,
    idle_timeout: Duration,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let hello = ActorHello {
        version: CLOUD_CONTRACT.to_string(),
        message_type: "actor.hello".to_string(),
        actor_id: config.actor_id.clone(),
        workspace_id: config.workspace_id.clone(),
        extension_version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: Capabilities {
            extension_actor: true,
            cdp_available: true,
            cdp_enabled: false,
            cdp_attached: false,
        },
        metadata: Some(json!({
            "runtime_owner": "supervisor",
            "runtime": "rzn-browser supervisor",
            "pid": std::process::id()
        })),
    };
    socket
        .send(Message::Text(serde_json::to_string(&hello)?))
        .await?;
    let ready = wait_for_actor_ready(&mut socket).await?;
    if ready.actor_id != config.actor_id {
        bail!("Cloud actor.ready identity does not match the configured actor");
    }
    result_cache
        .lock()
        .await
        .activate_scope(receipt_scope(&config));
    mark_cloud_actor_ready(&status).await;

    let default_timeout = config
        .request_timeout_ms
        .unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS)
        .max(1);
    let (command_tx, command_rx) = mpsc::channel::<CloudCommandEnvelope>(MAX_QUEUED_CLOUD_MESSAGES);
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<String>(MAX_QUEUED_CLOUD_MESSAGES);
    // The socket worker ends with the session; dispatch ownership lives in the shared cache.
    let _worker = AbortOnDrop(tokio::spawn(run_cloud_command_worker(
        command_rx,
        outbound_tx,
        default_timeout,
        config.clone(),
        dispatch_tx,
        result_cache,
        status,
    )));

    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + ping_every, ping_every);
    let mut last_rx = tokio::time::Instant::now();
    loop {
        tokio::select! {
            message = socket.next() => {
                let Some(message) = message else { return Ok(()); };
                last_rx = tokio::time::Instant::now();
                match message {
                    Ok(Message::Text(text)) => {
                        let envelope: CloudCommandEnvelope =
                            serde_json::from_str(&text).context("Decode cloud command envelope")?;
                        command_tx.try_send(envelope).context("Cloud command queue full or closed; reconnecting")?;
                    }
                    Ok(Message::Ping(payload)) => {
                        send_with_timeout(&mut socket, Message::Pong(payload), idle_timeout).await?;
                    }
                    Ok(Message::Pong(_)) => {}
                    Ok(Message::Close(_)) => bail!("Cloud control plane closed websocket"),
                    Err(error) => return Err(error.into()),
                    _ => {}
                }
            }
            text = outbound_rx.recv() => {
                let Some(text) = text else { bail!("Cloud command worker stopped; reconnecting"); };
                send_with_timeout(&mut socket, Message::Text(text), idle_timeout).await?;
            }
            _ = ping.tick() => {
                send_with_timeout(&mut socket, Message::Ping(Vec::new()), idle_timeout).await?;
            }
            _ = tokio::time::sleep_until(last_rx + idle_timeout) => {
                bail!(
                    "Cloud control plane silent for {}ms; reconnecting",
                    idle_timeout.as_millis()
                );
            }
        }
    }
}

/// A blocked write (full send buffer on a dead connection) must not wedge the
/// loop past the idle deadline either.
async fn send_with_timeout<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    message: Message,
    timeout: Duration,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(timeout, socket.send(message))
        .await
        .context("Cloud websocket send timed out")??;
    Ok(())
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn run_cloud_command_worker(
    mut command_rx: mpsc::Receiver<CloudCommandEnvelope>,
    outbound_tx: mpsc::Sender<String>,
    default_timeout: u64,
    config: LocalCloudActorConfig,
    dispatch_tx: mpsc::UnboundedSender<CloudDispatchRequest>,
    result_cache: Arc<Mutex<CommandResultCache>>,
    status: Arc<Mutex<CloudActorRuntimeState>>,
) {
    // Overflow ends the session; a replay joins in-flight work or reads its cached result.
    let queue = |value: String| outbound_tx.try_send(value).is_ok();
    while let Some(envelope) = command_rx.recv().await {
        if envelope.actor_id != config.actor_id {
            if !queue(to_json(&error_command_result(
                &envelope,
                "Cloud command actor does not match the configured actor",
            ))) {
                return;
            }
            continue;
        }
        if envelope.payload.kind != CloudCommandKind::BrowserCommand {
            let result = error_command_result(
                &envelope,
                "Unsupported cloud command kind; only browser_command is implemented".to_string(),
            );
            if !queue(to_json(&result)) {
                return;
            }
            continue;
        }
        let admission = prepare_cloud_command(
            envelope,
            default_timeout,
            Some(&config),
            dispatch_tx.clone(),
            result_cache.clone(),
            status.clone(),
        )
        .await;
        let result = match admission {
            CommandAdmission::Rejected(result) => result,
            CommandAdmission::Accepted(prepared) => {
                if !queue(to_json(&command_ack(&prepared.envelope))) {
                    return;
                }
                wait_for_prepared_command(prepared).await
            }
        };
        if !queue(to_json(&result)) {
            return;
        }
    }
}

fn to_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

#[cfg(test)]
async fn dispatch_command_with_dedupe(
    envelope: CloudCommandEnvelope,
    default_request_timeout_ms: u64,
    dispatch_tx: mpsc::UnboundedSender<CloudDispatchRequest>,
    result_cache: Arc<Mutex<CommandResultCache>>,
    status: Arc<Mutex<CloudActorRuntimeState>>,
) -> CloudCommandResult {
    match prepare_cloud_command(
        envelope,
        default_request_timeout_ms,
        None,
        dispatch_tx,
        result_cache,
        status,
    )
    .await
    {
        CommandAdmission::Rejected(result) => result,
        CommandAdmission::Accepted(prepared) => wait_for_prepared_command(prepared).await,
    }
}

enum CommandAdmission {
    Accepted(PreparedCommand),
    Rejected(CloudCommandResult),
}

enum PreparedOutcome {
    Cached(CloudCommandResult),
    Waiting(watch::Receiver<Option<CloudCommandResult>>),
}

struct PreparedCommand {
    envelope: CloudCommandEnvelope,
    outcome: PreparedOutcome,
}

async fn wait_for_prepared_command(prepared: PreparedCommand) -> CloudCommandResult {
    let PreparedCommand { envelope, outcome } = prepared;
    let mut result = match outcome {
        PreparedOutcome::Cached(result) => result,
        PreparedOutcome::Waiting(mut result_rx) => loop {
            if let Some(result) = result_rx.borrow_and_update().clone() {
                break result;
            }
            if result_rx.changed().await.is_err() {
                break error_command_result(
                    &envelope,
                    "Cloud command dispatch ended without a result",
                );
            }
        },
    };
    // A server may renew the lease while retrying the same immutable command.
    // Route the stored outcome to the current lease without replaying effects.
    result.lease_id = envelope.lease_id;
    result
}

async fn prepare_cloud_command(
    envelope: CloudCommandEnvelope,
    default_request_timeout_ms: u64,
    config: Option<&LocalCloudActorConfig>,
    dispatch_tx: mpsc::UnboundedSender<CloudDispatchRequest>,
    result_cache: Arc<Mutex<CommandResultCache>>,
    status: Arc<Mutex<CloudActorRuntimeState>>,
) -> CommandAdmission {
    let fallback_envelope = envelope.clone();
    match tokio::spawn(prepare_cloud_command_inner(
        envelope,
        default_request_timeout_ms,
        config.cloned(),
        dispatch_tx,
        result_cache,
        status,
    ))
    .await
    {
        Ok(admission) => admission,
        Err(error) => CommandAdmission::Rejected(error_command_result(
            &fallback_envelope,
            format!("Cloud command admission task failed: {error}"),
        )),
    }
}

async fn prepare_cloud_command_inner(
    envelope: CloudCommandEnvelope,
    default_request_timeout_ms: u64,
    config: Option<LocalCloudActorConfig>,
    dispatch_tx: mpsc::UnboundedSender<CloudDispatchRequest>,
    result_cache: Arc<Mutex<CommandResultCache>>,
    status: Arc<Mutex<CloudActorRuntimeState>>,
) -> CommandAdmission {
    let scope = config.as_ref().map(receipt_scope);
    let config = config.as_ref();
    let fingerprint = command_fingerprint(&envelope);
    let result_rx;
    let generation;
    {
        let mut cache = result_cache.lock().await;
        if let Some(scope) = scope.as_deref() {
            if cache.active_scope.as_deref() != Some(scope) {
                return CommandAdmission::Rejected(error_command_result(
                    &envelope,
                    "Cloud actor config changed before command acceptance",
                ));
            }
        }
        if let Some(config) = config {
            if envelope.actor_id != config.actor_id {
                return CommandAdmission::Rejected(error_command_result(
                    &envelope,
                    "Cloud command actor does not match the configured actor",
                ));
            }
        }
        if let Some(cached) = cache.get(&envelope.command_id) {
            if cache
                .fingerprints
                .get(&envelope.command_id)
                .is_some_and(|cached_fingerprint| cached_fingerprint != &fingerprint)
            {
                return CommandAdmission::Rejected(command_id_conflict_result(&envelope));
            }
            return CommandAdmission::Accepted(PreparedCommand {
                envelope,
                outcome: PreparedOutcome::Cached(cached),
            });
        }
        if let Some(inflight) = cache.inflight.get(&envelope.command_id) {
            if cache
                .inflight_fingerprints
                .get(&envelope.command_id)
                .is_some_and(|active_fingerprint| active_fingerprint != &fingerprint)
            {
                return CommandAdmission::Rejected(command_id_conflict_result(&envelope));
            }
            return CommandAdmission::Accepted(PreparedCommand {
                envelope,
                outcome: PreparedOutcome::Waiting(inflight.clone()),
            });
        }
        if cache.persistence_error.is_some() {
            if !retry_receipt_journal(&mut cache).await {
                let error = cache
                    .persistence_error
                    .clone()
                    .unwrap_or_else(|| "receipt journal is unavailable".to_string());
                status.lock().await.command_receipt_error = Some(error.clone());
                return CommandAdmission::Rejected(error_command_result(
                    &envelope,
                    format!(
                        "Cloud command receipt journal unavailable; command not dispatched: {error}"
                    ),
                ));
            }
            status.lock().await.command_receipt_error = None;
        }

        if let (Some(scope), Some(journal)) = (scope.as_deref(), cache.receipts.as_mut()) {
            if let Some(receipt) = journal.find(scope, &envelope.command_id).cloned() {
                if receipt.fingerprint != fingerprint {
                    return CommandAdmission::Rejected(command_id_conflict_result(&envelope));
                }
                let result = match receipt.state {
                    CommandReceiptState::Completed(result) => result,
                    CommandReceiptState::Accepted => {
                        let result = indeterminate_receipt_result(
                            &receipt.identity,
                            "A prior accepted command has no live dispatcher; its outcome is indeterminate and it was not replayed.",
                        );
                        if let Err(error) = journal
                            .complete(scope, &envelope.command_id, result.clone())
                            .await
                        {
                            let message = error.to_string();
                            cache.persistence_error = Some(message.clone());
                            cache.journal_retry_after =
                                Some(Instant::now() + RECEIPT_REOPEN_BACKOFF);
                            status.lock().await.command_receipt_error = Some(message);
                            return CommandAdmission::Rejected(error_command_result(
                                &envelope,
                                "Cloud command receipt journal could not record an indeterminate outcome; command not dispatched",
                            ));
                        }
                        result
                    }
                };
                cache.insert(
                    envelope.command_id.clone(),
                    fingerprint.clone(),
                    result.clone(),
                );
                return CommandAdmission::Accepted(PreparedCommand {
                    envelope,
                    outcome: PreparedOutcome::Cached(result),
                });
            }
        }

        if envelope.deadline_ms <= now_ms() {
            return CommandAdmission::Rejected(expired_command_result(
                &envelope,
                "Cloud command expired before acceptance; command not dispatched",
            ));
        }
        if cache.inflight.len() >= MAX_CACHED_COMMAND_RESULTS {
            return CommandAdmission::Rejected(error_command_result(
                &envelope,
                "Cloud command overload: too many in-flight dispatches",
            ));
        }

        if let (Some(scope), Some(journal)) = (scope.as_deref(), cache.receipts.as_mut()) {
            journal.prune_expired_completed(now_ms());
            if journal.entries.len() >= MAX_CACHED_COMMAND_RESULTS {
                return CommandAdmission::Rejected(error_command_result(
                    &envelope,
                    "Cloud command receipt capacity reached; command not dispatched",
                ));
            }
            let receipt = CommandReceipt {
                scope: scope.to_string(),
                command_id: envelope.command_id.clone(),
                fingerprint: fingerprint.clone(),
                deadline_ms: envelope.deadline_ms,
                identity: receipt_identity(&envelope),
                state: CommandReceiptState::Accepted,
            };
            if let Err(error) = journal.accept(receipt).await {
                let message = error.to_string();
                cache.persistence_error = Some(message.clone());
                cache.journal_retry_after = Some(Instant::now() + RECEIPT_REOPEN_BACKOFF);
                status.lock().await.command_receipt_error = Some(message);
                return CommandAdmission::Rejected(error_command_result(
                    &envelope,
                    "Cloud command receipt journal could not durably accept command; command not dispatched",
                ));
            }
        }

        let (result_tx, receiver) = watch::channel(None);
        result_rx = receiver.clone();
        cache.inflight.insert(envelope.command_id.clone(), receiver);
        cache
            .inflight_fingerprints
            .insert(envelope.command_id.clone(), fingerprint.clone());
        generation = cache.generation;
        let cache = result_cache.clone();
        let status = status.clone();
        let dispatch_envelope = envelope.clone();
        let receipt_scope = scope.clone();
        let dispatch_fingerprint = fingerprint.clone();
        tokio::spawn(async move {
            if cache.lock().await.generation == generation {
                let mut guard = status.lock().await;
                guard.inflight_command_id = Some(dispatch_envelope.command_id.clone());
                guard.last_command_id = Some(dispatch_envelope.command_id.clone());
            }
            let config_changed = cache.lock().await.generation != generation;
            let mut result = if config_changed {
                error_command_result(
                    &dispatch_envelope,
                    "Cloud actor config changed before dispatch",
                )
            } else if dispatch_envelope.deadline_ms <= now_ms() {
                expired_command_result(
                    &dispatch_envelope,
                    "Cloud command expired before browser dispatch; command not dispatched",
                )
            } else {
                dispatch_command(
                    dispatch_envelope.clone(),
                    default_request_timeout_ms,
                    dispatch_tx,
                )
                .await
            };
            {
                let mut cache = cache.lock().await;
                if let (Some(scope), Some(journal)) =
                    (receipt_scope.as_deref(), cache.receipts.as_mut())
                {
                    if let Err(error) = journal
                        .complete(scope, &dispatch_envelope.command_id, result.clone())
                        .await
                    {
                        let message = error.to_string();
                        cache.persistence_error = Some(message.clone());
                        cache.journal_retry_after = Some(Instant::now() + RECEIPT_REOPEN_BACKOFF);
                        status.lock().await.command_receipt_error = Some(message);
                        result = indeterminate_receipt_result(
                            &receipt_identity(&dispatch_envelope),
                            "Command ran but its result could not be durably recorded; its outcome is indeterminate and it will not be replayed.",
                        );
                    }
                }
                if cache.generation == generation {
                    cache.inflight.remove(&dispatch_envelope.command_id);
                    cache
                        .inflight_fingerprints
                        .remove(&dispatch_envelope.command_id);
                    cache.insert(
                        dispatch_envelope.command_id.clone(),
                        dispatch_fingerprint,
                        result.clone(),
                    );
                    let mut guard = status.lock().await;
                    if guard.inflight_command_id.as_deref()
                        == Some(dispatch_envelope.command_id.as_str())
                    {
                        guard.inflight_command_id = None;
                    }
                    guard.last_result_at_ms = Some(now_ms());
                    guard.dedupe_cache_size = cache.len();
                }
            }
            let _ = result_tx.send(Some(result));
        });
    }

    CommandAdmission::Accepted(PreparedCommand {
        envelope,
        outcome: PreparedOutcome::Waiting(result_rx),
    })
}

async fn dispatch_command(
    envelope: CloudCommandEnvelope,
    default_request_timeout_ms: u64,
    dispatch_tx: mpsc::UnboundedSender<CloudDispatchRequest>,
) -> CloudCommandResult {
    let timeout_ms = compute_request_timeout_ms(
        envelope.deadline_ms,
        default_request_timeout_ms,
        DEFAULT_EXTENSION_RPC_GRACE_MS,
    );
    let (respond_to, response_rx) = oneshot::channel::<CloudCommandResult>();
    if dispatch_tx
        .send(CloudDispatchRequest {
            envelope: envelope.clone(),
            default_request_timeout_ms,
            respond_to,
        })
        .is_err()
    {
        return error_command_result(&envelope, "Supervisor cloud dispatcher is unavailable");
    }

    match tokio::time::timeout(Duration::from_millis(timeout_ms.max(1)), response_rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => error_command_result(
            &envelope,
            format!(
                "Supervisor cloud dispatch channel closed for {}",
                envelope.command_id
            ),
        ),
        Err(_) => error_command_result(
            &envelope,
            format!(
                "Timed out waiting for supervisor cloud dispatch {}",
                envelope.command_id
            ),
        ),
    }
}

async fn wait_for_actor_ready<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
) -> Result<ActorReady>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let maybe_message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .context("Timed out waiting for actor.ready")?;
        match maybe_message {
            Some(Ok(Message::Text(text))) => {
                let ready: ActorReady =
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

async fn apply_cloud_control_message(
    control: CloudActorControlMessage,
    status: &Arc<Mutex<CloudActorRuntimeState>>,
    result_cache: &Arc<Mutex<CommandResultCache>>,
) -> Option<LocalCloudActorConfig> {
    match control {
        CloudActorControlMessage::ApplyConfig {
            next_config,
            respond_to,
        } => {
            result_cache.lock().await.clear();
            let mut guard = status.lock().await;
            guard.config = next_config.clone();
            guard.connected = false;
            guard.last_error = None;
            guard.inflight_command_id = None;
            guard.dedupe_cache_size = 0;
            drop(guard);
            if let Some(respond_to) = respond_to {
                let _ = respond_to.send(());
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

async fn mark_cloud_actor_connected(status: &Arc<Mutex<CloudActorRuntimeState>>) {
    let mut guard = status.lock().await;
    guard.connected = true;
    guard.last_connected_at_ms = Some(now_ms());
    guard.last_error = None;
}

async fn mark_cloud_actor_ready(status: &Arc<Mutex<CloudActorRuntimeState>>) {
    let mut guard = status.lock().await;
    guard.connected = true;
    guard.last_ready_at_ms = Some(now_ms());
    guard.last_error = None;
}

async fn mark_cloud_actor_disconnected(status: &Arc<Mutex<CloudActorRuntimeState>>) {
    let mut guard = status.lock().await;
    guard.connected = false;
}

fn command_ack(envelope: &CloudCommandEnvelope) -> CloudCommandAck {
    CloudCommandAck {
        version: CLOUD_CONTRACT.to_string(),
        message_type: "command.ack".to_string(),
        actor_id: envelope.actor_id.clone(),
        run_id: envelope.run_id.clone(),
        session_id: envelope.session_id.clone(),
        command_id: envelope.command_id.clone(),
        lease_id: envelope.lease_id.clone(),
        accepted_at_ms: now_ms(),
        trace_id: envelope.trace_id.clone(),
    }
}

fn error_command_result(
    envelope: &CloudCommandEnvelope,
    error: impl Into<String>,
) -> CloudCommandResult {
    let error = error.into();
    CloudCommandResult {
        version: CLOUD_CONTRACT.to_string(),
        message_type: "command.result".to_string(),
        actor_id: envelope.actor_id.clone(),
        run_id: envelope.run_id.clone(),
        session_id: envelope.session_id.clone(),
        command_id: envelope.command_id.clone(),
        lease_id: envelope.lease_id.clone(),
        success: false,
        finished_at_ms: now_ms(),
        trace_id: envelope.trace_id.clone(),
        result: Some(BrowserActionResult {
            success: false,
            error_code: Some("SUPERVISOR_CLOUD_ERROR".to_string()),
            error: Some(error.clone()),
            current_url: None,
            current_tab_id: None,
            current_tab_ref: None,
            dom_hash: None,
            dom_snapshot: None,
            capabilities: None,
            raw: Some(json!({
                "runtime_owner": "supervisor",
                "error": error
            })),
        }),
        error: Some(error),
    }
}

pub(crate) fn compute_request_timeout_ms(deadline_ms: u64, default_ms: u64, grace_ms: u64) -> u64 {
    let now = now_ms();
    if deadline_ms > now {
        deadline_ms.saturating_sub(now).saturating_add(grace_ms)
    } else {
        default_ms.saturating_add(grace_ms)
    }
}

fn load_local_actor_config() -> Result<Option<LocalCloudActorConfig>> {
    let path = resolve_cloud_actor_config_path(None);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("Read cloud actor config {}", path.display()))?;
    let config: LocalCloudActorConfig =
        serde_json::from_slice(&bytes).context("Parse cloud actor config JSON")?;
    normalize_local_actor_config(config).map(Some)
}

fn normalize_local_actor_config(
    mut config: LocalCloudActorConfig,
) -> Result<LocalCloudActorConfig> {
    if config.version != CLOUD_ACTOR_CONFIG_CONTRACT {
        bail!("Unsupported cloud actor config version {}", config.version);
    }
    config.server_url = normalize_server_url(&config.server_url)?;
    if config.websocket_url.trim().is_empty() {
        config.websocket_url = websocket_url_from_server(&config.server_url)?;
    } else {
        config.websocket_url = normalize_websocket_url(&config.websocket_url)?;
    }
    Ok(config)
}

fn persist_local_actor_config(path: &Path, config: &LocalCloudActorConfig) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(config)?;
    write_secret_file(path, bytes)
        .with_context(|| format!("Write cloud actor config {}", path.display()))
}

fn remove_local_actor_config(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path)
            .with_context(|| format!("Remove cloud actor config {}", path.display()))?;
    }
    Ok(())
}

fn normalize_server_url(server_url: &str) -> Result<String> {
    let mut url = Url::parse(server_url.trim())
        .with_context(|| format!("Invalid cloud server URL {}", server_url))?;
    validate_server_url(&url)?;
    url.set_query(None);
    url.set_fragment(None);
    let normalized = url.to_string().trim_end_matches('/').to_string();
    Ok(normalized)
}

fn websocket_url_from_server(server_url: &str) -> Result<String> {
    let mut url = Url::parse(server_url)?;
    match url.scheme() {
        "http" if is_loopback_url(&url) => url
            .set_scheme("ws")
            .map_err(|_| anyhow!("Set websocket scheme"))?,
        "http" => bail!("Cloud server URL must use https unless it targets loopback"),
        "https" => url
            .set_scheme("wss")
            .map_err(|_| anyhow!("Set websocket scheme"))?,
        other => bail!("Unsupported cloud server URL scheme {}", other),
    }
    url.set_path("/v1/actors/connect");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

fn normalize_websocket_url(websocket_url: &str) -> Result<String> {
    let mut url = Url::parse(websocket_url.trim())
        .with_context(|| format!("Invalid cloud actor websocket URL {}", websocket_url))?;
    validate_websocket_url(&url)?;
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

fn build_actor_ws_url(config: &LocalCloudActorConfig) -> Result<Url> {
    let mut url = Url::parse(&config.websocket_url)
        .with_context(|| format!("Invalid cloud actor websocket URL {}", config.websocket_url))?;
    validate_websocket_url(&url)?;
    url.query_pairs_mut()
        .append_pair("actor_id", &config.actor_id)
        .append_pair("workspace_id", &config.workspace_id);
    Ok(url)
}

fn build_actor_ws_request(config: &LocalCloudActorConfig) -> Result<Request<()>> {
    let url = build_actor_ws_url(config)?;
    let mut request = url.as_str().into_client_request()?;
    let header_value = HeaderValue::from_str(&format!("Bearer {}", config.actor_token))
        .context("Build cloud actor authorization header")?;
    request.headers_mut().insert(AUTHORIZATION, header_value);
    Ok(request)
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

fn next_backoff(current: Duration) -> Duration {
    (current * 2).min(Duration::from_secs(30))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rzn_contracts::browser::{CloudBrowserCommand, CloudCommandPayload};

    fn receipt_path(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rzn-cloud-receipts-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&path).expect("create test receipt dir");
        path.join(COMMAND_RECEIPT_FILE)
    }

    fn sample_envelope(command_id: &str) -> CloudCommandEnvelope {
        CloudCommandEnvelope {
            version: CLOUD_CONTRACT.to_string(),
            message_type: "command.execute".to_string(),
            actor_id: "actor-1".to_string(),
            run_id: "run-1".to_string(),
            session_id: "session-1".to_string(),
            command_id: command_id.to_string(),
            lease_id: "lease-1".to_string(),
            deadline_ms: now_ms() + 10_000,
            trace_id: Some("trace-1".to_string()),
            parent_command_id: None,
            planner_step_index: Some(0),
            payload: CloudCommandPayload {
                kind: CloudCommandKind::BrowserCommand,
                command: Some(CloudBrowserCommand {
                    cmd: "execute_step".to_string(),
                    payload: Some(json!({
                        "session_id": "session-1",
                        "step": { "type": "get_current_url" }
                    })),
                    data: None,
                }),
                side_effecting: Some(false),
                idempotency_policy: Some("single_delivery".to_string()),
                metadata: None,
            },
        }
    }

    fn sample_result(envelope: &CloudCommandEnvelope) -> CloudCommandResult {
        CloudCommandResult {
            version: CLOUD_CONTRACT.to_string(),
            message_type: "command.result".to_string(),
            actor_id: envelope.actor_id.clone(),
            run_id: envelope.run_id.clone(),
            session_id: envelope.session_id.clone(),
            command_id: envelope.command_id.clone(),
            lease_id: envelope.lease_id.clone(),
            success: true,
            finished_at_ms: now_ms(),
            trace_id: envelope.trace_id.clone(),
            result: Some(BrowserActionResult {
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

    fn sample_actor_config() -> LocalCloudActorConfig {
        LocalCloudActorConfig {
            version: CLOUD_ACTOR_CONFIG_CONTRACT.to_string(),
            actor_id: "actor-1".to_string(),
            actor_token: "token-1".to_string(),
            workspace_id: "workspace-1".to_string(),
            server_url: "https://cloud.example.test".to_string(),
            websocket_url: "wss://cloud.example.test/v1/actors/connect".to_string(),
            paired_at_ms: 123,
            connect_timeout_ms: Some(500),
            request_timeout_ms: Some(1_000),
        }
    }

    #[test]
    fn fingerprint_allows_new_lease_but_ignores_stale_nested_session() {
        let first = sample_envelope("same-command");
        let mut redelivery = first.clone();
        redelivery.lease_id = "lease-renewed".to_string();
        redelivery
            .payload
            .command
            .as_mut()
            .unwrap()
            .payload
            .as_mut()
            .unwrap()["session_id"] = json!("stale-nested-session");
        assert_eq!(
            command_fingerprint(&first),
            command_fingerprint(&redelivery)
        );
    }

    async fn activate_test_scope(
        cache: &Arc<Mutex<CommandResultCache>>,
        config: &LocalCloudActorConfig,
    ) {
        cache.lock().await.activate_scope(receipt_scope(config));
    }

    #[tokio::test]
    async fn accepted_receipt_survives_restart_as_indeterminate_without_redispatch() {
        let path = receipt_path("restart");
        let config = sample_actor_config();
        let envelope = sample_envelope("command-crashed");
        let scope = receipt_scope(&config);
        {
            let mut journal = CommandReceiptJournal::open(path.clone()).unwrap();
            journal
                .accept(CommandReceipt {
                    scope: scope.clone(),
                    command_id: envelope.command_id.clone(),
                    fingerprint: command_fingerprint(&envelope),
                    deadline_ms: envelope.deadline_ms,
                    identity: receipt_identity(&envelope),
                    state: CommandReceiptState::Accepted,
                })
                .await
                .unwrap();
        }

        let journal = CommandReceiptJournal::open(path.clone()).unwrap();
        let Some(CommandReceipt {
            state: CommandReceiptState::Completed(result),
            ..
        }) = journal.find(&scope, &envelope.command_id)
        else {
            panic!("restart must durably finish an accepted receipt as indeterminate");
        };
        assert_eq!(
            result.result.as_ref().unwrap().error_code.as_deref(),
            Some("COMMAND_INDETERMINATE")
        );
        assert!(result.error.as_deref().unwrap().contains("not replayed"));

        let cache = Arc::new(Mutex::new(CommandResultCache {
            receipts: Some(journal),
            ..CommandResultCache::default()
        }));
        activate_test_scope(&cache, &config).await;
        let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel();
        let admission = prepare_cloud_command(
            envelope.clone(),
            1_000,
            Some(&config),
            dispatch_tx,
            cache,
            Arc::new(Mutex::new(CloudActorRuntimeState::default())),
        )
        .await;
        let CommandAdmission::Accepted(prepared) = admission else {
            panic!("replayed receipt should return its stored outcome");
        };
        let replay = wait_for_prepared_command(prepared).await;
        assert_eq!(
            replay.result.as_ref().unwrap().error_code.as_deref(),
            Some("COMMAND_INDETERMINATE")
        );
        assert!(
            dispatch_rx.try_recv().is_err(),
            "restart replay must not dispatch"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn completed_receipt_rejects_mutated_payload_after_restart() {
        let path = receipt_path("completed");
        let config = sample_actor_config();
        let envelope = sample_envelope("command-completed");
        let expected = sample_result(&envelope);
        let scope = receipt_scope(&config);
        {
            let mut journal = CommandReceiptJournal::open(path.clone()).unwrap();
            journal
                .accept(CommandReceipt {
                    scope: scope.clone(),
                    command_id: envelope.command_id.clone(),
                    fingerprint: command_fingerprint(&envelope),
                    deadline_ms: envelope.deadline_ms,
                    identity: receipt_identity(&envelope),
                    state: CommandReceiptState::Accepted,
                })
                .await
                .unwrap();
            journal
                .complete(&scope, &envelope.command_id, expected.clone())
                .await
                .unwrap();
        }
        let journal = CommandReceiptJournal::open(path.clone()).unwrap();
        let cache = Arc::new(Mutex::new(CommandResultCache {
            receipts: Some(journal),
            ..CommandResultCache::default()
        }));
        activate_test_scope(&cache, &config).await;
        let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel();
        let mut replay_envelope = envelope.clone();
        replay_envelope.lease_id = "lease-renewed".to_string();
        let replay = prepare_cloud_command(
            replay_envelope.clone(),
            1_000,
            Some(&config),
            dispatch_tx.clone(),
            cache.clone(),
            Arc::new(Mutex::new(CloudActorRuntimeState::default())),
        )
        .await;
        let CommandAdmission::Accepted(prepared) = replay else {
            panic!("completed receipt should replay unchanged payload");
        };
        let replay_result = wait_for_prepared_command(prepared).await;
        let mut expected_replay = expected.clone();
        expected_replay.lease_id = replay_envelope.lease_id.clone();
        assert_eq!(replay_result, expected_replay);
        assert!(dispatch_rx.try_recv().is_err(), "replay must not dispatch");

        replay_envelope = envelope.clone();
        replay_envelope.payload.command.as_mut().unwrap().cmd = "close_tab".to_string();
        let admission = prepare_cloud_command(
            replay_envelope,
            1_000,
            Some(&config),
            dispatch_tx,
            cache,
            Arc::new(Mutex::new(CloudActorRuntimeState::default())),
        )
        .await;
        let CommandAdmission::Rejected(result) = admission else {
            panic!("reused command id with different payload must be rejected");
        };
        assert_eq!(
            result.result.unwrap().error_code.as_deref(),
            Some("COMMAND_ID_CONFLICT")
        );
        assert!(
            dispatch_rx.try_recv().is_err(),
            "conflicting payload must not dispatch"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn receipt_journal_lock_excludes_second_supervisor_and_recovers_after_release() {
        let path = receipt_path("journal-lock");
        let config = sample_actor_config();
        let envelope = sample_envelope("command-lock-owner");
        let scope = receipt_scope(&config);
        let mut owner = CommandReceiptJournal::open(path.clone()).unwrap();
        owner
            .accept(CommandReceipt {
                scope,
                command_id: envelope.command_id.clone(),
                fingerprint: command_fingerprint(&envelope),
                deadline_ms: envelope.deadline_ms,
                identity: receipt_identity(&envelope),
                state: CommandReceiptState::Accepted,
            })
            .await
            .unwrap();

        assert!(
            CommandReceiptJournal::open(path.clone()).is_err(),
            "a second supervisor must not open or rewrite the active journal"
        );
        let persisted: CommandReceiptFile =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(matches!(
            persisted.entries[0].state,
            CommandReceiptState::Accepted
        ));

        drop(owner);
        let recovered = CommandReceiptJournal::open(path.clone()).unwrap();
        let Some(CommandReceipt {
            state: CommandReceiptState::Completed(result),
            ..
        }) = recovered.find(&receipt_scope(&config), &envelope.command_id)
        else {
            panic!("new owner must recover the accepted receipt as terminal");
        };
        assert_eq!(
            result.result.as_ref().unwrap().error_code.as_deref(),
            Some("COMMAND_INDETERMINATE")
        );
        drop(recovered);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn canceled_receipt_writer_keeps_journal_lock_until_write_finishes() {
        let path = receipt_path("journal-lock-cancel");
        let config = sample_actor_config();
        let envelope = sample_envelope("command-lock-cancel");
        let scope = receipt_scope(&config);
        let started = Arc::new(AtomicBool::new(false));
        let mut journal = CommandReceiptJournal::open(path.clone()).unwrap();
        journal.persist_test_hook = Some((
            started.clone(),
            Arc::new(AtomicU64::new(0)),
            Duration::from_millis(250),
        ));
        let writer = tokio::spawn(async move {
            journal
                .accept(CommandReceipt {
                    scope,
                    command_id: envelope.command_id.clone(),
                    fingerprint: command_fingerprint(&envelope),
                    deadline_ms: envelope.deadline_ms,
                    identity: receipt_identity(&envelope),
                    state: CommandReceiptState::Accepted,
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !started.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("blocking receipt writer started");
        writer.abort();
        assert!(writer
            .await
            .err()
            .expect("journal owner task was cancelled")
            .is_cancelled());

        assert!(
            CommandReceiptJournal::open(path.clone()).is_err(),
            "detached blocking write must retain the exclusive lock"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        let recovered = CommandReceiptJournal::open(path.clone()).unwrap();
        let Some(CommandReceipt {
            state: CommandReceiptState::Completed(result),
            ..
        }) = recovered.find(&receipt_scope(&config), "command-lock-cancel")
        else {
            panic!("completed blocking write must be recovered without replay");
        };
        assert_eq!(
            result.result.as_ref().unwrap().error_code.as_deref(),
            Some("COMMAND_INDETERMINATE")
        );
        drop(recovered);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn receipt_write_failure_fails_closed_before_dispatch() {
        let root = receipt_path("write-failure")
            .parent()
            .unwrap()
            .to_path_buf();
        let blocker = root.join("not-a-directory");
        fs::write(&blocker, b"file").unwrap();
        let config = sample_actor_config();
        let envelope = sample_envelope("command-no-write");
        let cache = Arc::new(Mutex::new(CommandResultCache {
            receipts: Some(CommandReceiptJournal {
                path: blocker.join(COMMAND_RECEIPT_FILE),
                entries: Vec::new(),
                lock: Arc::new(CommandReceiptJournalLock {
                    _file: fs::File::open(&blocker).unwrap(),
                }),
                persist_test_hook: None,
            }),
            ..CommandResultCache::default()
        }));
        activate_test_scope(&cache, &config).await;
        let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel();
        let admission = prepare_cloud_command(
            envelope,
            1_000,
            Some(&config),
            dispatch_tx,
            cache,
            Arc::new(Mutex::new(CloudActorRuntimeState::default())),
        )
        .await;
        let CommandAdmission::Rejected(result) = admission else {
            panic!("failed receipt write must reject the command");
        };
        assert!(result.error.as_deref().unwrap().contains("durably accept"));
        assert!(dispatch_rx.try_recv().is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn receipt_scope_partitions_actor_workspace_and_server() {
        let original = sample_actor_config();
        let mut changed = original.clone();
        changed.actor_id.push_str("-other");
        assert_ne!(receipt_scope(&original), receipt_scope(&changed));
        changed = original.clone();
        changed.workspace_id.push_str("-other");
        assert_ne!(receipt_scope(&original), receipt_scope(&changed));
        changed = original.clone();
        changed.server_url = "https://other.example.test".to_string();
        assert_ne!(receipt_scope(&original), receipt_scope(&changed));

        let path = receipt_path("namespace-change");
        let cache = Arc::new(Mutex::new(CommandResultCache {
            receipts: Some(CommandReceiptJournal::open(path.clone()).unwrap()),
            ..CommandResultCache::default()
        }));
        activate_test_scope(&cache, &original).await;
        let status = Arc::new(Mutex::new(CloudActorRuntimeState::default()));
        let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel();
        let envelope = sample_envelope("same-command-id");
        let first = prepare_cloud_command(
            envelope.clone(),
            1_000,
            Some(&original),
            dispatch_tx.clone(),
            cache.clone(),
            status.clone(),
        )
        .await;
        let CommandAdmission::Accepted(first) = first else {
            panic!("first config should accept command");
        };
        let request = dispatch_rx.recv().await.unwrap();
        request.respond_to.send(sample_result(&envelope)).unwrap();
        assert!(wait_for_prepared_command(first).await.success);

        cache.lock().await.clear();
        let mut next_config = original.clone();
        next_config.actor_id = "actor-2".to_string();
        next_config.workspace_id = "workspace-2".to_string();
        activate_test_scope(&cache, &next_config).await;
        let mut next_envelope = envelope.clone();
        next_envelope.actor_id = next_config.actor_id.clone();
        let second = prepare_cloud_command(
            next_envelope.clone(),
            1_000,
            Some(&next_config),
            dispatch_tx,
            cache,
            status,
        )
        .await;
        let CommandAdmission::Accepted(second) = second else {
            panic!("new namespace should accept the reused command id");
        };
        let request = dispatch_rx.recv().await.unwrap();
        request
            .respond_to
            .send(sample_result(&next_envelope))
            .unwrap();
        assert!(wait_for_prepared_command(second).await.success);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn expired_command_is_rejected_without_a_receipt_or_dispatch() {
        let path = receipt_path("expired");
        let config = sample_actor_config();
        let mut envelope = sample_envelope("command-expired");
        envelope.deadline_ms = now_ms().saturating_sub(1);
        let cache = Arc::new(Mutex::new(CommandResultCache {
            receipts: Some(CommandReceiptJournal::open(path.clone()).unwrap()),
            ..CommandResultCache::default()
        }));
        activate_test_scope(&cache, &config).await;
        let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel();
        let admission = prepare_cloud_command(
            envelope,
            1_000,
            Some(&config),
            dispatch_tx,
            cache.clone(),
            Arc::new(Mutex::new(CloudActorRuntimeState::default())),
        )
        .await;
        let CommandAdmission::Rejected(result) = admission else {
            panic!("expired command must be rejected");
        };
        assert_eq!(
            result.result.unwrap().error_code.as_deref(),
            Some("COMMAND_EXPIRED")
        );
        assert!(cache
            .lock()
            .await
            .receipts
            .as_ref()
            .unwrap()
            .entries
            .is_empty());
        assert!(dispatch_rx.try_recv().is_err());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn canceled_receipt_admission_keeps_later_snapshot_and_dispatch() {
        let path = receipt_path("cancel-admission");
        let config = sample_actor_config();
        let started = Arc::new(AtomicBool::new(false));
        let writes = Arc::new(AtomicU64::new(0));
        let mut journal = CommandReceiptJournal::open(path.clone()).unwrap();
        journal.persist_test_hook = Some((started.clone(), writes, Duration::from_millis(200)));
        let cache = Arc::new(Mutex::new(CommandResultCache {
            receipts: Some(journal),
            ..CommandResultCache::default()
        }));
        activate_test_scope(&cache, &config).await;
        let status = Arc::new(Mutex::new(CloudActorRuntimeState::default()));
        let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel();
        let first_envelope = sample_envelope("command-first");
        let first = tokio::spawn({
            let envelope = first_envelope.clone();
            let config = config.clone();
            let dispatch_tx = dispatch_tx.clone();
            let cache = cache.clone();
            let status = status.clone();
            async move {
                prepare_cloud_command(envelope, 1_000, Some(&config), dispatch_tx, cache, status)
                    .await
            }
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !started.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("first atomic receipt write started");
        first.abort();
        assert!(first
            .await
            .err()
            .expect("outer admission task was cancelled")
            .is_cancelled());

        let second_envelope = sample_envelope("command-second");
        let second = tokio::spawn({
            let envelope = second_envelope.clone();
            let config = config.clone();
            let cache = cache.clone();
            async move {
                prepare_cloud_command(envelope, 1_000, Some(&config), dispatch_tx, cache, status)
                    .await
            }
        });
        let first_request = tokio::time::timeout(Duration::from_secs(2), dispatch_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first_request.envelope.command_id, first_envelope.command_id);
        first_request
            .respond_to
            .send(sample_result(&first_envelope))
            .unwrap();
        let second_request = tokio::time::timeout(Duration::from_secs(2), dispatch_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            second_request.envelope.command_id,
            second_envelope.command_id
        );
        second_request
            .respond_to
            .send(sample_result(&second_envelope))
            .unwrap();
        let CommandAdmission::Accepted(prepared) = second.await.unwrap() else {
            panic!("second command should be admitted after first write finishes");
        };
        assert!(wait_for_prepared_command(prepared).await.success);

        // Inspect the persisted snapshot while the live cache still owns the journal lock.
        let journal: CommandReceiptFile =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(journal.entries.len(), 2);
        assert!(journal
            .entries
            .iter()
            .all(|receipt| { matches!(receipt.state, CommandReceiptState::Completed(_)) }));
        assert!(dispatch_rx.try_recv().is_err());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn receipt_journal_reopens_after_storage_recovers_and_backoff_expires() {
        let path = receipt_path("reopen");
        persist_receipt_file(
            &path,
            &CommandReceiptFile {
                version: COMMAND_RECEIPT_VERSION,
                entries: Vec::new(),
            },
        )
        .unwrap();
        let config = sample_actor_config();
        let cache = Arc::new(Mutex::new(CommandResultCache {
            receipts: Some(CommandReceiptJournal::open(path.clone()).unwrap()),
            receipt_path: Some(path.clone()),
            persistence_error: Some("transient disk error".to_string()),
            journal_retry_after: Some(Instant::now() - Duration::from_secs(1)),
            ..CommandResultCache::default()
        }));
        activate_test_scope(&cache, &config).await;
        let status = Arc::new(Mutex::new(CloudActorRuntimeState {
            command_receipt_error: Some("transient disk error".to_string()),
            ..CloudActorRuntimeState::default()
        }));
        let envelope = sample_envelope("command-after-recovery");
        let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel();
        let admission = prepare_cloud_command(
            envelope.clone(),
            1_000,
            Some(&config),
            dispatch_tx,
            cache.clone(),
            status.clone(),
        )
        .await;
        let CommandAdmission::Accepted(prepared) = admission else {
            panic!("recovered journal should admit a new command");
        };
        let request = dispatch_rx.recv().await.unwrap();
        request.respond_to.send(sample_result(&envelope)).unwrap();
        assert!(wait_for_prepared_command(prepared).await.success);
        assert!(status.lock().await.command_receipt_error.is_none());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn cloud_actor_connection_gate_waits_for_journal_owner() {
        let path = receipt_path("connection-gate");
        let owner = CommandReceiptJournal::open(path.clone()).unwrap();
        let error = CommandReceiptJournal::open(path.clone())
            .err()
            .expect("competing cloud actor must not acquire the journal")
            .to_string();
        let cache = Arc::new(Mutex::new(CommandResultCache {
            receipt_path: Some(path.clone()),
            persistence_error: Some(error.clone()),
            journal_retry_after: Some(Instant::now() - Duration::from_secs(1)),
            ..CommandResultCache::default()
        }));
        let status = Arc::new(Mutex::new(CloudActorRuntimeState {
            command_receipt_error: Some(error.clone()),
            ..CloudActorRuntimeState::default()
        }));
        let (_control_tx, mut control_rx) = mpsc::unbounded_channel();

        assert!(matches!(
            wait_for_receipt_journal_or_control(&cache, &status, &mut control_rx).await,
            ReceiptJournalGateOutcome::RetryLater
        ));
        assert_eq!(
            cache.lock().await.persistence_error.as_deref(),
            Some(error.as_str())
        );
        assert_eq!(
            status.lock().await.command_receipt_error.as_deref(),
            Some(error.as_str())
        );

        drop(owner);
        cache.lock().await.journal_retry_after = Some(Instant::now() - Duration::from_secs(1));
        assert!(matches!(
            wait_for_receipt_journal_or_control(&cache, &status, &mut control_rx).await,
            ReceiptJournalGateOutcome::Ready
        ));
        assert!(cache.lock().await.persistence_error.is_none());
        assert!(status.lock().await.command_receipt_error.is_none());
        drop(cache);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn corrupt_receipt_journal_stays_visible_and_fails_closed() {
        let path = receipt_path("corrupt");
        let corrupt = b"{broken";
        fs::write(&path, corrupt).unwrap();
        let error = CommandReceiptJournal::open(path.clone())
            .err()
            .expect("corrupt journal must not open")
            .to_string();
        let config = sample_actor_config();
        let cache = Arc::new(Mutex::new(CommandResultCache {
            persistence_error: Some(error.clone()),
            receipt_path: Some(path.clone()),
            journal_retry_after: Some(Instant::now() + RECEIPT_REOPEN_BACKOFF),
            ..CommandResultCache::default()
        }));
        activate_test_scope(&cache, &config).await;
        let status = Arc::new(Mutex::new(CloudActorRuntimeState {
            command_receipt_error: Some(error.clone()),
            ..CloudActorRuntimeState::default()
        }));
        let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel();
        let admission = prepare_cloud_command(
            sample_envelope("command-corrupt-journal"),
            1_000,
            Some(&config),
            dispatch_tx,
            cache.clone(),
            status.clone(),
        )
        .await;
        let CommandAdmission::Rejected(result) = admission else {
            panic!("corrupt journal must reject command acceptance");
        };
        assert!(result
            .error
            .as_deref()
            .unwrap()
            .contains("journal unavailable"));
        assert_eq!(
            cache.lock().await.persistence_error.as_deref(),
            Some(error.as_str())
        );
        assert_eq!(
            status.lock().await.command_receipt_error.as_deref(),
            Some(error.as_str())
        );
        assert!(dispatch_rx.try_recv().is_err());
        assert_eq!(
            fs::read(&path).unwrap(),
            corrupt,
            "do not erase corrupt receipts"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn actor_ws_request_sends_token_in_header_not_url() {
        let config = sample_actor_config();
        let request = build_actor_ws_request(&config).expect("build request");

        let uri = request.uri().to_string();
        assert!(uri.contains("actor_id=actor-1"));
        assert!(uri.contains("workspace_id=workspace-1"));
        assert!(!uri.contains("token-1"));
        assert!(!uri.contains("token="));
        assert_eq!(
            request
                .headers()
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer token-1")
        );
    }

    #[test]
    fn websocket_url_uses_wss_except_loopback() {
        assert_eq!(
            websocket_url_from_server("http://127.0.0.1:8787").unwrap(),
            "ws://127.0.0.1:8787/v1/actors/connect"
        );
        assert_eq!(
            websocket_url_from_server("https://cloud.example.test").unwrap(),
            "wss://cloud.example.test/v1/actors/connect"
        );
        assert!(websocket_url_from_server("http://cloud.example.test").is_err());
    }

    #[tokio::test]
    async fn apply_config_control_message_updates_status_before_ack() {
        let cache = Arc::new(Mutex::new(CommandResultCache::default()));
        let status = Arc::new(Mutex::new(CloudActorRuntimeState::default()));
        let (ack_tx, ack_rx) = oneshot::channel();

        let next = apply_cloud_control_message(
            CloudActorControlMessage::ApplyConfig {
                next_config: Some(sample_actor_config()),
                respond_to: Some(ack_tx),
            },
            &status,
            &cache,
        )
        .await;

        assert!(next.is_some());
        assert!(ack_rx.await.is_ok());
        let guard = status.lock().await;
        assert_eq!(
            guard.config.as_ref().map(|config| config.actor_id.as_str()),
            Some("actor-1")
        );
        assert!(!guard.connected);
        assert_eq!(guard.dedupe_cache_size, 0);
    }

    #[tokio::test]
    async fn clear_config_control_message_updates_status_before_ack() {
        let cache = Arc::new(Mutex::new(CommandResultCache::default()));
        let status = Arc::new(Mutex::new(CloudActorRuntimeState {
            config: Some(sample_actor_config()),
            connected: true,
            dedupe_cache_size: 1,
            ..CloudActorRuntimeState::default()
        }));
        let (ack_tx, ack_rx) = oneshot::channel();

        let next = apply_cloud_control_message(
            CloudActorControlMessage::ApplyConfig {
                next_config: None,
                respond_to: Some(ack_tx),
            },
            &status,
            &cache,
        )
        .await;

        assert!(next.is_none());
        assert!(ack_rx.await.is_ok());
        let guard = status.lock().await;
        assert!(guard.config.is_none());
        assert!(!guard.connected);
        assert_eq!(guard.dedupe_cache_size, 0);
    }

    #[tokio::test]
    async fn connect_phase_control_message_acks_before_connect_timeout() {
        let (control_tx, mut control_rx) = mpsc::unbounded_channel::<CloudActorControlMessage>();
        let cache = Arc::new(Mutex::new(CommandResultCache::default()));
        let status = Arc::new(Mutex::new(CloudActorRuntimeState {
            config: Some(sample_actor_config()),
            ..CloudActorRuntimeState::default()
        }));
        let (ack_tx, ack_rx) = oneshot::channel();

        control_tx
            .send(CloudActorControlMessage::ApplyConfig {
                next_config: None,
                respond_to: Some(ack_tx),
            })
            .expect("send control");

        let outcome = tokio::time::timeout(
            Duration::from_millis(50),
            wait_for_cloud_connection_or_control(
                Duration::from_secs(60),
                std::future::pending::<std::result::Result<(), &'static str>>(),
                &mut control_rx,
            ),
        )
        .await
        .expect("control should interrupt pending connect");

        let ConnectPhaseOutcome::Control(control) = outcome else {
            panic!("expected control message to win connect race");
        };
        let next = apply_cloud_control_message(control, &status, &cache).await;

        assert!(next.is_none());
        assert!(ack_rx.await.is_ok());
        assert!(status.lock().await.config.is_none());
    }

    #[tokio::test]
    async fn invalid_url_backoff_control_message_acks_before_sleep_timeout() {
        let (control_tx, mut control_rx) = mpsc::unbounded_channel::<CloudActorControlMessage>();
        let cache = Arc::new(Mutex::new(CommandResultCache::default()));
        let status = Arc::new(Mutex::new(CloudActorRuntimeState {
            config: Some(sample_actor_config()),
            ..CloudActorRuntimeState::default()
        }));
        let (ack_tx, ack_rx) = oneshot::channel();

        control_tx
            .send(CloudActorControlMessage::ApplyConfig {
                next_config: None,
                respond_to: Some(ack_tx),
            })
            .expect("send control");

        let outcome = tokio::time::timeout(
            Duration::from_millis(50),
            wait_for_backoff_or_control(Duration::from_secs(60), &mut control_rx),
        )
        .await
        .expect("control should interrupt invalid-url backoff");

        let BackoffOutcome::Control(control) = outcome else {
            panic!("expected control message to win backoff race");
        };
        let next = apply_cloud_control_message(control, &status, &cache).await;

        assert!(next.is_none());
        assert!(ack_rx.await.is_ok());
        assert!(status.lock().await.config.is_none());
    }

    #[test]
    fn normalize_local_actor_config_rejects_invalid_non_empty_websocket_url() {
        let mut config = sample_actor_config();
        config.websocket_url = "not a websocket url".to_string();

        let error = normalize_local_actor_config(config).expect_err("invalid websocket URL");

        assert!(
            error
                .to_string()
                .contains("Invalid cloud actor websocket URL"),
            "{error}"
        );
    }

    #[test]
    fn normalize_local_actor_config_rejects_unsupported_websocket_scheme() {
        let mut config = sample_actor_config();
        config.websocket_url = "https://cloud.example.test/v1/actors/connect".to_string();

        let error = normalize_local_actor_config(config).expect_err("unsupported websocket scheme");

        assert!(
            error
                .to_string()
                .contains("Unsupported cloud websocket URL scheme"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn duplicate_command_id_replays_cached_result_before_dispatch() {
        let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel::<CloudDispatchRequest>();
        let cache = Arc::new(Mutex::new(CommandResultCache::default()));
        let status = Arc::new(Mutex::new(CloudActorRuntimeState::default()));
        let envelope = sample_envelope("command-1");

        let first_envelope = envelope.clone();
        let first_dispatch = tokio::spawn(dispatch_command_with_dedupe(
            first_envelope.clone(),
            1_000,
            dispatch_tx.clone(),
            cache.clone(),
            status.clone(),
        ));

        let request = dispatch_rx.recv().await.expect("first dispatch request");
        assert_eq!(request.envelope.command_id, "command-1");
        request
            .respond_to
            .send(sample_result(&first_envelope))
            .expect("send dispatch result");
        let first = first_dispatch.await.unwrap();
        assert!(first.success);

        let second = dispatch_command_with_dedupe(
            envelope.clone(),
            1_000,
            dispatch_tx.clone(),
            cache.clone(),
            status,
        )
        .await;
        assert!(second.success);
        assert_eq!(second.command_id, "command-1");
        assert!(
            tokio::time::timeout(Duration::from_millis(50), dispatch_rx.recv())
                .await
                .is_err(),
            "duplicate command should not reach extension dispatch"
        );

        let mut conflict = envelope;
        conflict.payload.command.as_mut().unwrap().cmd = "close_tab".to_string();
        let conflict = dispatch_command_with_dedupe(
            conflict,
            1_000,
            dispatch_tx,
            cache,
            Arc::new(Mutex::new(CloudActorRuntimeState::default())),
        )
        .await;
        assert_eq!(
            conflict.result.unwrap().error_code.as_deref(),
            Some("COMMAND_ID_CONFLICT")
        );
        assert!(dispatch_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn in_flight_limit_returns_overload_without_dispatch() {
        let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel::<CloudDispatchRequest>();
        let cache = Arc::new(Mutex::new(CommandResultCache::default()));
        for index in 0..MAX_CACHED_COMMAND_RESULTS {
            let (_, receiver) = watch::channel(None);
            cache
                .lock()
                .await
                .inflight
                .insert(format!("pending-{index}"), receiver);
        }
        let result = dispatch_command_with_dedupe(
            sample_envelope("over-limit"),
            1_000,
            dispatch_tx.clone(),
            cache,
            Arc::new(Mutex::new(CloudActorRuntimeState::default())),
        )
        .await;
        assert!(!result.success);
        assert!(result.error.unwrap().contains("overload"));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), dispatch_rx.recv())
                .await
                .is_err(),
            "overloaded command must not reach extension dispatch"
        );
    }

    #[tokio::test]
    async fn abort_waiter_while_status_locked_keeps_dispatch_owned() {
        let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel::<CloudDispatchRequest>();
        let cache = Arc::new(Mutex::new(CommandResultCache::default()));
        let status = Arc::new(Mutex::new(CloudActorRuntimeState::default()));
        let envelope = sample_envelope("command-aborted-waiter");
        let status_guard = status.lock().await;
        let first = tokio::spawn(dispatch_command_with_dedupe(
            envelope.clone(),
            1_000,
            dispatch_tx.clone(),
            cache.clone(),
            status.clone(),
        ));
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(guard) = cache.try_lock() {
                    if guard.inflight.contains_key(&envelope.command_id) {
                        break;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("command registered before dispatch");
        first.abort();
        assert!(first
            .await
            .err()
            .expect("outer admission task was cancelled")
            .is_cancelled());
        drop(status_guard);

        let replay = tokio::spawn(dispatch_command_with_dedupe(
            envelope.clone(),
            1_000,
            dispatch_tx.clone(),
            cache.clone(),
            status,
        ));
        let request = tokio::time::timeout(Duration::from_secs(2), dispatch_rx.recv())
            .await
            .unwrap()
            .expect("owned dispatch survives waiter abort");
        let expected = sample_result(&envelope);
        request.respond_to.send(expected.clone()).unwrap();
        assert_eq!(replay.await.unwrap(), expected);
        assert_eq!(cache.lock().await.get(&envelope.command_id), Some(expected));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), dispatch_rx.recv())
                .await
                .is_err(),
            "replay must not dispatch again"
        );
    }

    #[tokio::test]
    async fn disconnect_mid_dispatch_replays_one_result_without_dispatching_twice() {
        let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel::<CloudDispatchRequest>();
        let cache = Arc::new(Mutex::new(CommandResultCache::default()));
        let status = Arc::new(Mutex::new(CloudActorRuntimeState::default()));
        let envelope = sample_envelope("command-reconnect");
        let (client, mut server) = ws_pair().await;
        let session = tokio::spawn(run_cloud_actor_session_with(
            client,
            sample_actor_config(),
            dispatch_tx.clone(),
            cache.clone(),
            status.clone(),
            Duration::from_secs(60),
            Duration::from_secs(60),
        ));
        server
            .send(Message::Text(serde_json::to_string(&envelope).unwrap()))
            .await
            .unwrap();
        let request = tokio::time::timeout(Duration::from_secs(2), dispatch_rx.recv())
            .await
            .unwrap()
            .expect("first dispatch");
        server.close(None).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), session)
            .await
            .expect("first session ended")
            .unwrap()
            .expect_err("closed socket reconnects");

        let (client, mut server) = ws_pair().await;
        let session = tokio::spawn(run_cloud_actor_session_with(
            client,
            sample_actor_config(),
            dispatch_tx,
            cache.clone(),
            status,
            Duration::from_secs(60),
            Duration::from_secs(60),
        ));
        server
            .send(Message::Text(serde_json::to_string(&envelope).unwrap()))
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), dispatch_rx.recv())
                .await
                .is_err(),
            "replayed command must await the original dispatch"
        );
        let expected = sample_result(&envelope);
        request.respond_to.send(expected.clone()).unwrap();
        let actual = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let Some(Ok(Message::Text(text))) = server.next().await else {
                    panic!("socket closed before replay result");
                };
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "command.result" {
                    break serde_json::from_value::<CloudCommandResult>(value).unwrap();
                }
            }
        })
        .await
        .expect("replayed result");
        assert_eq!(actual, expected);
        assert_eq!(cache.lock().await.get(&envelope.command_id), Some(expected));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), dispatch_rx.recv())
                .await
                .is_err(),
            "only one command should reach extension dispatch"
        );
        session.abort();
    }

    type Ws = tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>;

    /// In-memory client/server websocket pair; the server has sent actor.ready.
    async fn ws_pair() -> (Ws, Ws) {
        use tokio_tungstenite::tungstenite::protocol::Role;
        let (a, b) = tokio::io::duplex(64 * 1024);
        let client = Ws::from_raw_socket(a, Role::Client, None).await;
        let mut server = Ws::from_raw_socket(b, Role::Server, None).await;
        let ready = ActorReady {
            version: CLOUD_CONTRACT.to_string(),
            message_type: "actor.ready".to_string(),
            actor_id: "actor-1".to_string(),
            heartbeat_interval_ms: 0,
            resume_token: None,
            config: None,
        };
        server
            .send(Message::Text(serde_json::to_string(&ready).unwrap()))
            .await
            .unwrap();
        (client, server)
    }

    #[tokio::test]
    async fn session_pings_and_reconnects_on_silent_peer() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio_tungstenite::tungstenite::protocol::Role;
        // Raw server end: a websocket library would auto-pong, and a half-open
        // peer never answers anything.
        let (a, mut raw) = tokio::io::duplex(64 * 1024);
        let client = Ws::from_raw_socket(a, Role::Client, None).await;
        let ready = serde_json::to_vec(&json!({
            "version": CLOUD_CONTRACT, "type": "actor.ready",
            "actor_id": "actor-1", "heartbeat_interval_ms": 0
        }))
        .unwrap();
        assert!(ready.len() < 126);
        let mut frame = vec![0x81, ready.len() as u8]; // FIN|text, unmasked
        frame.extend_from_slice(&ready);
        raw.write_all(&frame).await.unwrap();

        let (dispatch_tx, _dispatch_rx) = mpsc::unbounded_channel();
        let err = tokio::time::timeout(
            Duration::from_secs(2),
            run_cloud_actor_session_with(
                client,
                sample_actor_config(),
                dispatch_tx,
                Arc::new(Mutex::new(CommandResultCache::default())),
                Arc::new(Mutex::new(CloudActorRuntimeState::default())),
                Duration::from_millis(20),
                Duration::from_millis(150),
            ),
        )
        .await
        .expect("idle timeout must end the session")
        .expect_err("silent peer is an error so the caller reconnects");
        assert!(err.to_string().contains("silent"), "{err}");

        // Parse the client's (masked) frames and count pings.
        let mut bytes = Vec::new();
        raw.read_to_end(&mut bytes).await.unwrap();
        let (mut i, mut pings) = (0, 0);
        while i + 2 <= bytes.len() {
            let opcode = bytes[i] & 0x0f;
            let mut len = (bytes[i + 1] & 0x7f) as usize;
            i += 2;
            if len == 126 {
                len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
                i += 2;
            }
            i += 4 + len; // mask key + payload
            pings += usize::from(opcode == 0x9);
        }
        assert!(pings >= 2, "expected keepalive pings, got {pings}");
    }

    #[tokio::test]
    async fn long_command_does_not_stall_pongs() {
        let (client, mut server) = ws_pair().await;
        let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel::<CloudDispatchRequest>();
        let status = Arc::new(Mutex::new(CloudActorRuntimeState::default()));
        let session = tokio::spawn(run_cloud_actor_session_with(
            client,
            sample_actor_config(),
            dispatch_tx,
            Arc::new(Mutex::new(CommandResultCache::default())),
            status,
            Duration::from_secs(60),
            Duration::from_secs(60),
        ));

        server
            .send(Message::Text(
                serde_json::to_string(&sample_envelope("command-slow")).unwrap(),
            ))
            .await
            .unwrap();
        // The command reaches dispatch and is never answered (a long command).
        let _pending = dispatch_rx.recv().await.expect("dispatched");
        server.send(Message::Ping(b"alive".to_vec())).await.unwrap();

        let mut saw_ack = false;
        let pong = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match server.next().await {
                    Some(Ok(Message::Pong(payload))) => return payload,
                    Some(Ok(Message::Text(text))) if text.contains("command.ack") => saw_ack = true,
                    Some(Ok(_)) => {}
                    other => panic!("unexpected {other:?}"),
                }
            }
        })
        .await
        .expect("pong while the command is still running");
        assert_eq!(pong, b"alive".to_vec());
        assert!(saw_ack, "ack is sent before the command finishes");
        session.abort();
    }

    #[test]
    fn deadline_timeout_includes_grace() {
        let timeout_ms = compute_request_timeout_ms(now_ms().saturating_add(250), 1_000, 5_000);
        assert!((5_000..=5_250).contains(&timeout_ms));
    }
}
