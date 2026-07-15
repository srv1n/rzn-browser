//! FLA-T-0003: supervisor fleet poll loop with journal + crash-safe result post.
//!
//! When `fleet_config.json` (written by `fleet_cli`, FLA-T-0005) is present, the
//! supervisor starts a jittered poll loop that:
//!   - heartbeats to the backend and claims at most one job per poll,
//!   - journals every job to disk (`fleet_journal.jsonl`) *before* executing it,
//!   - runs the manifest through the shared `workflow_runner` in-process (its
//!     `StepTransport` drives `SupervisorState::dispatch` directly — it never
//!     dials the local socket),
//!   - persists the terminal result to `fleet_results/<job_id>.json` and posts it
//!     with retry until the server acks (a `deduped` ack counts as posted),
//!   - reconciles on restart: accepted/running-without-finished jobs post an
//!     `aborted` result, finished-not-posted results are re-posted, and an
//!     already-completed re-delivered job is deduped instead of re-executed.
//!
//! The device token is never logged. All timestamps are epoch milliseconds.

use anyhow::anyhow;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::task::JoinHandle;

use crate::run_store::{AppendRun, RunStore};
use rzn_contracts::fleet_v1::{
    error_codes, DeviceHealthV1, FleetDeviceStatusV1, FleetJobAssignmentV1,
    FleetJobTerminalStatusV1, FleetPollRequestV1, FleetPollResponseV1, FleetResultAckV1,
    FleetResultPostV1,
};
use rzn_contracts::v2::{RunErrorV1, RunResultV2, RunStatusV2, RUN_RESULT_VERSION};
use rzn_core::runtime_paths::{default_app_base_dir, env_trimmed};

use crate::supervisor::SupervisorState;
use crate::workflow_cache::{HttpManifestFetcher, WorkflowCache};
use crate::workflow_runner::{
    execute_workflow, load_workflow_for_run, RunEventSink, RunOptions, SessionSpec, SnapshotMode,
    StepTransport, TransportError,
};

// ---------------------------------------------------------------------------
// Constants + env overrides
// ---------------------------------------------------------------------------

const FLEET_CONFIG_FILENAME: &str = "fleet_config.json";
const FLEET_CONFIG_PATH_ENV: &str = "RZN_FLEET_CONFIG_PATH";
/// Test/smoke override (FLA-T-0006): forces the base poll interval, in ms.
const FLEET_POLL_INTERVAL_MS_ENV: &str = "RZN_FLEET_POLL_INTERVAL_MS";
/// Test override: disable jitter for deterministic timing.
const FLEET_DISABLE_JITTER_ENV: &str = "RZN_FLEET_DISABLE_JITTER";

const JOURNAL_FILENAME: &str = "fleet_journal.jsonl";
const RESULTS_DIRNAME: &str = "fleet_results";
const WORKFLOW_CACHE_DIRNAME: &str = "workflow_cache";

const DEFAULT_POLL_INTERVAL_SECS: u64 = 45;
/// Backoff ceiling for network errors: 5 minutes.
const MAX_BACKOFF_MS: u64 = 5 * 60 * 1_000;
/// ±33% client-side jitter on the poll cadence.
const JITTER_FRACTION: f64 = 0.33;
const MAX_JOURNAL_TAIL: usize = 5;
/// Fallback job deadline when the assignment omits `execution_deadline_seconds`.
const DEFAULT_JOB_DEADLINE_SECS: u64 = 600;
/// How often the manifest cache is garbage-collected (once daily).
const CACHE_GC_INTERVAL_SECS: u64 = 24 * 60 * 60;

// ---------------------------------------------------------------------------
// fleet.status / fleet.disable local RPC bridge
// ---------------------------------------------------------------------------

/// Shared handle the running loop publishes so `dispatch` can serve the local
/// `fleet.status` / `fleet.disable` RPCs the CLI calls.
static FLEET_RUNTIME: OnceLock<Arc<FleetShared>> = OnceLock::new();

/// `fleet.status` RPC. Returns the live loop state (CLI reads `state`/`reason`).
pub(crate) fn fleet_status_rpc() -> Value {
    match FLEET_RUNTIME.get() {
        Some(shared) => shared.status_json(),
        None => json!({ "state": "disabled", "reason": "fleet not enrolled" }),
    }
}

/// `fleet.disable` RPC. Stops the loop (and cancels any running job) if present.
pub(crate) fn fleet_disable_rpc() -> Value {
    if let Some(shared) = FLEET_RUNTIME.get() {
        shared.request_disable();
    }
    json!({ "ok": true })
}

// ---------------------------------------------------------------------------
// Device config (shared shape with fleet_cli / FLA-T-0005)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct FleetDeviceConfig {
    version: String,
    server_url: String,
    device_id: String,
    device_token: String,
    tenant_id: String,
    #[serde(default)]
    poll_interval_seconds: Option<u64>,
}

/// Resolve `fleet_config.json` exactly as the CLI does (env override wins, else
/// the runtime dir) so the supervisor reads the identical file the CLI wrote.
fn fleet_config_path() -> PathBuf {
    if let Some(path) = env_trimmed(FLEET_CONFIG_PATH_ENV) {
        return PathBuf::from(path);
    }
    default_app_base_dir().join(FLEET_CONFIG_FILENAME)
}

fn load_fleet_config() -> Option<FleetDeviceConfig> {
    let path = fleet_config_path();
    let bytes = fs::read(&path).ok()?;
    match serde_json::from_slice::<FleetDeviceConfig>(&bytes) {
        Ok(config) => Some(config),
        Err(err) => {
            tracing::warn!("fleet: ignoring unparseable {}: {}", path.display(), err);
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Startup hook (called from supervisor::serve)
// ---------------------------------------------------------------------------

/// Start the fleet poll loop when this device is enrolled. No-op otherwise, so
/// local-only supervisors are entirely unchanged. Never logs the device token.
pub(crate) fn maybe_spawn_fleet_loop(state: Arc<SupervisorState>) {
    let Some(config) = load_fleet_config() else {
        return;
    };

    let base = default_app_base_dir();
    let journal = match Journal::open(base.join(JOURNAL_FILENAME)) {
        Ok(journal) => Arc::new(journal),
        Err(err) => {
            tracing::warn!("fleet: could not open journal, fleet mode disabled: {err}");
            return;
        }
    };
    let results_dir = base.join(RESULTS_DIRNAME);
    let cache = Arc::new(WorkflowCache::new(base.join(WORKFLOW_CACHE_DIRNAME)));
    let fetcher = Arc::new(HttpManifestFetcher::new(
        config.server_url.clone(),
        config.device_token.clone(),
    ));
    let api: Arc<dyn FleetApi> = Arc::new(HttpFleetApi::new(
        config.server_url.clone(),
        config.device_token.clone(),
    ));
    let executor: Arc<dyn FleetJobExecutor> = Arc::new(InProcessJobExecutor {
        state: state.clone(),
        cache: cache.clone(),
        fetcher,
        default_deadline_secs: DEFAULT_JOB_DEADLINE_SECS,
    });
    let health: Arc<dyn HealthProbe> = Arc::new(StateHealthProbe {
        state: state.clone(),
    });

    let shared = Arc::new(FleetShared::new());
    // The first serve() wins the global; a second call (should not happen) is a
    // no-op so the CLI keeps reading the original loop's status.
    let _ = FLEET_RUNTIME.set(shared.clone());

    // Daily best-effort manifest cache GC (keep newest 3 per id, drop >30d).
    tokio::spawn(async move {
        loop {
            cache.gc(3, 30);
            tokio::time::sleep(Duration::from_secs(CACHE_GC_INTERVAL_SECS)).await;
        }
    });

    let fleet = FleetLoop {
        api,
        executor,
        health,
        shared,
        journal,
        results_dir,
        config_interval_secs: config.poll_interval_seconds,
        interval_ms_override: None,
        started_at_ms: now_ms(),
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
        state,
    };

    tracing::info!(
        server = %config.server_url,
        device_id = %config.device_id,
        "fleet mode enabled; starting poll loop"
    );
    tokio::spawn(fleet.run());
}

// ---------------------------------------------------------------------------
// Shared loop status (published for fleet.status)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopState {
    Polling,
    StoppedRevoked,
    StoppedDormant,
    Disabled,
}

impl LoopState {
    fn as_str(self) -> &'static str {
        match self {
            LoopState::Polling => "polling",
            LoopState::StoppedRevoked => "stopped_revoked",
            LoopState::StoppedDormant => "stopped_dormant",
            LoopState::Disabled => "disabled",
        }
    }
}

struct FleetLoopStatus {
    state: LoopState,
    reason: Option<String>,
    last_poll_ms: Option<i64>,
    active_job_id: Option<String>,
    journal_tail: Vec<JournalTailEntry>,
}

/// Loop-owned shared state: the published status plus the disable/cancel signals.
struct FleetShared {
    status: Mutex<FleetLoopStatus>,
    disabled: AtomicBool,
    cancel_current: Mutex<Option<Arc<AtomicBool>>>,
    steps_seen: AtomicU64,
}

impl FleetShared {
    fn new() -> Self {
        Self {
            status: Mutex::new(FleetLoopStatus {
                state: LoopState::Polling,
                reason: None,
                last_poll_ms: None,
                active_job_id: None,
                journal_tail: Vec::new(),
            }),
            disabled: AtomicBool::new(false),
            cancel_current: Mutex::new(None),
            steps_seen: AtomicU64::new(0),
        }
    }

    fn request_disable(&self) {
        self.disabled.store(true, Ordering::SeqCst);
        if let Some(cancel) = self.cancel_current.lock().unwrap().as_ref() {
            cancel.store(true, Ordering::SeqCst);
        }
    }

    fn set_cancel(&self, cancel: Arc<AtomicBool>) {
        *self.cancel_current.lock().unwrap() = Some(cancel);
    }

    fn clear_cancel(&self) {
        *self.cancel_current.lock().unwrap() = None;
    }

    fn set_state(&self, state: LoopState, reason: Option<String>) {
        let mut status = self.status.lock().unwrap();
        status.state = state;
        status.reason = reason;
    }

    fn set_last_poll(&self, ms: i64) {
        self.status.lock().unwrap().last_poll_ms = Some(ms);
    }

    fn set_active_job(&self, id: Option<String>) {
        self.status.lock().unwrap().active_job_id = id;
    }

    fn refresh_tail(&self, journal: &Journal) {
        let tail = journal.tail(MAX_JOURNAL_TAIL);
        self.status.lock().unwrap().journal_tail = tail;
    }

    /// Record that the runner drove a step (in-memory liveness signal).
    fn note_step(&self) {
        self.steps_seen.fetch_add(1, Ordering::SeqCst);
    }

    fn status_json(&self) -> Value {
        let status = self.status.lock().unwrap();
        let mut map = Map::new();
        map.insert("state".to_string(), json!(status.state.as_str()));
        if let Some(reason) = &status.reason {
            map.insert("reason".to_string(), json!(reason));
        }
        if let Some(ms) = status.last_poll_ms {
            map.insert("last_poll_ms".to_string(), json!(ms));
        }
        if let Some(job) = &status.active_job_id {
            map.insert("active_job_id".to_string(), json!(job));
        }
        map.insert(
            "journal_tail".to_string(),
            serde_json::to_value(&status.journal_tail).unwrap_or_else(|_| json!([])),
        );
        Value::Object(map)
    }
}

// ---------------------------------------------------------------------------
// Journal (append-only, fsync'd)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JournalState {
    Accepted,
    Running,
    Finished,
    Posted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JournalEntry {
    job_id: String,
    state: JournalState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_status: Option<FleetJobTerminalStatusV1>,
    ts_ms: i64,
    #[serde(default)]
    workflow_id: String,
    #[serde(default)]
    workflow_hash: String,
    #[serde(default)]
    single_delivery: bool,
}

/// The compact `{job_id, state, ts_ms}` view surfaced in `fleet.status`.
#[derive(Debug, Clone, Serialize)]
struct JournalTailEntry {
    job_id: String,
    state: JournalState,
    ts_ms: i64,
}

impl JournalEntry {
    fn from_assignment(
        assignment: &FleetJobAssignmentV1,
        state: JournalState,
        terminal_status: Option<FleetJobTerminalStatusV1>,
    ) -> Self {
        Self {
            job_id: assignment.job_id.clone(),
            state,
            terminal_status,
            ts_ms: now_ms(),
            workflow_id: assignment.workflow_id.clone(),
            workflow_hash: assignment.workflow_hash.clone(),
            single_delivery: assignment.single_delivery,
        }
    }

    fn carry(
        prev: &JournalEntry,
        state: JournalState,
        terminal_status: Option<FleetJobTerminalStatusV1>,
    ) -> Self {
        Self {
            job_id: prev.job_id.clone(),
            state,
            terminal_status,
            ts_ms: now_ms(),
            workflow_id: prev.workflow_id.clone(),
            workflow_hash: prev.workflow_hash.clone(),
            single_delivery: prev.single_delivery,
        }
    }

    fn marker(
        job_id: &str,
        workflow_id: &str,
        state: JournalState,
        terminal_status: Option<FleetJobTerminalStatusV1>,
    ) -> Self {
        Self {
            job_id: job_id.to_string(),
            state,
            terminal_status,
            ts_ms: now_ms(),
            workflow_id: workflow_id.to_string(),
            workflow_hash: String::new(),
            single_delivery: false,
        }
    }

    fn tail_view(&self) -> JournalTailEntry {
        JournalTailEntry {
            job_id: self.job_id.clone(),
            state: self.state,
            ts_ms: self.ts_ms,
        }
    }
}

/// Append-only journal held in memory and mirrored to a fsync'd JSONL file.
struct Journal {
    path: PathBuf,
    entries: Mutex<Vec<JournalEntry>>,
}

impl Journal {
    fn open(path: PathBuf) -> io::Result<Self> {
        let mut entries = Vec::new();
        match fs::read_to_string(&path) {
            Ok(text) => {
                for line in text.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if let Ok(entry) = serde_json::from_str::<JournalEntry>(line) {
                        entries.push(entry);
                    }
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        Ok(Self {
            path,
            entries: Mutex::new(entries),
        })
    }

    fn append(&self, entry: JournalEntry) -> io::Result<()> {
        let mut guard = self.entries.lock().unwrap();
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut line = serde_json::to_vec(&entry).map_err(io_error)?;
        line.push(b'\n');
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(&line)?;
        file.flush()?;
        file.sync_all()?;
        guard.push(entry);
        Ok(())
    }

    /// Latest recorded state for a job, if any.
    fn latest(&self, job_id: &str) -> Option<JournalEntry> {
        let guard = self.entries.lock().unwrap();
        guard.iter().rev().find(|e| e.job_id == job_id).cloned()
    }

    /// Latest entry per job (insertion order preserved by the file).
    fn latest_per_job(&self) -> Vec<JournalEntry> {
        let guard = self.entries.lock().unwrap();
        let mut order: Vec<String> = Vec::new();
        let mut map: HashMap<String, JournalEntry> = HashMap::new();
        for entry in guard.iter() {
            if !map.contains_key(&entry.job_id) {
                order.push(entry.job_id.clone());
            }
            map.insert(entry.job_id.clone(), entry.clone());
        }
        order.into_iter().filter_map(|id| map.remove(&id)).collect()
    }

    /// Rewrite the journal keeping only the latest entry of each non-`posted` job.
    fn compact(&self) -> io::Result<()> {
        let retained: Vec<JournalEntry> = self
            .latest_per_job()
            .into_iter()
            .filter(|entry| entry.state != JournalState::Posted)
            .collect();

        let mut buf = Vec::new();
        for entry in &retained {
            buf.extend_from_slice(&serde_json::to_vec(entry).map_err(io_error)?);
            buf.push(b'\n');
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("jsonl.tmp");
        {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(&buf)?;
            file.flush()?;
            file.sync_all()?;
        }
        fs::rename(&tmp, &self.path)?;

        let mut guard = self.entries.lock().unwrap();
        *guard = retained;
        Ok(())
    }

    fn tail(&self, n: usize) -> Vec<JournalTailEntry> {
        let guard = self.entries.lock().unwrap();
        let start = guard.len().saturating_sub(n);
        guard[start..].iter().map(JournalEntry::tail_view).collect()
    }
}

// ---------------------------------------------------------------------------
// Fleet control-plane API
// ---------------------------------------------------------------------------

/// A poll/post failure the loop must distinguish.
#[derive(Debug)]
enum FleetCallError {
    /// 403 with a `device_revoked` / `device_dormant` code: STOP polling.
    Stop { code: String, message: String },
    /// Transport error or a retryable non-2xx: back off and retry.
    Network(String),
}

#[async_trait]
trait FleetApi: Send + Sync {
    async fn poll(&self, req: &FleetPollRequestV1) -> Result<FleetPollResponseV1, FleetCallError>;
    async fn post_result(
        &self,
        job_id: &str,
        post: &FleetResultPostV1,
    ) -> Result<FleetResultAckV1, FleetCallError>;
}

/// Real reqwest-backed fleet API. The device token is only ever sent as a bearer
/// credential; it is never logged.
struct HttpFleetApi {
    client: reqwest::Client,
    server_url: String,
    device_token: String,
}

impl HttpFleetApi {
    fn new(server_url: impl Into<String>, device_token: impl Into<String>) -> Self {
        let server_url = server_url.into().trim_end_matches('/').to_string();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self {
            client,
            server_url,
            device_token: device_token.into(),
        }
    }
}

#[async_trait]
impl FleetApi for HttpFleetApi {
    async fn poll(&self, req: &FleetPollRequestV1) -> Result<FleetPollResponseV1, FleetCallError> {
        let url = format!("{}/v1/fleet/poll", self.server_url);
        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.device_token)
            .json(req)
            .send()
            .await
            .map_err(|err| FleetCallError::Network(format!("poll request failed: {err}")))?;
        let status = response.status();
        if status.as_u16() == 403 {
            let body = response.text().await.unwrap_or_default();
            if let Ok(err) = serde_json::from_str::<rzn_contracts::fleet_v1::FleetErrorV1>(&body) {
                if err.code == error_codes::DEVICE_REVOKED
                    || err.code == error_codes::DEVICE_DORMANT
                {
                    return Err(FleetCallError::Stop {
                        code: err.code,
                        message: err.message,
                    });
                }
            }
            return Err(FleetCallError::Network(format!("poll 403: {body}")));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(FleetCallError::Network(format!(
                "poll status {status}: {body}"
            )));
        }
        response
            .json::<FleetPollResponseV1>()
            .await
            .map_err(|err| FleetCallError::Network(format!("poll decode failed: {err}")))
    }

    async fn post_result(
        &self,
        job_id: &str,
        post: &FleetResultPostV1,
    ) -> Result<FleetResultAckV1, FleetCallError> {
        let url = format!("{}/v1/fleet/jobs/{}/result", self.server_url, job_id);
        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.device_token)
            .json(post)
            .send()
            .await
            .map_err(|err| FleetCallError::Network(format!("result post failed: {err}")))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(FleetCallError::Network(format!(
                "result post status {status}: {body}"
            )));
        }
        response
            .json::<FleetResultAckV1>()
            .await
            .map_err(|err| FleetCallError::Network(format!("result ack decode failed: {err}")))
    }
}

// ---------------------------------------------------------------------------
// Health probe
// ---------------------------------------------------------------------------

struct HealthSnapshot {
    browser_running: bool,
    extension_bridge_up: bool,
    readiness_cause: Option<String>,
    extension_version: Option<String>,
}

#[async_trait]
trait HealthProbe: Send + Sync {
    async fn probe(&self) -> HealthSnapshot;
}

/// Builds a device-health snapshot from the supervisor's `runtime.status`.
struct StateHealthProbe {
    state: Arc<SupervisorState>,
}

#[async_trait]
impl HealthProbe for StateHealthProbe {
    async fn probe(&self) -> HealthSnapshot {
        let status = self
            .state
            .dispatch("runtime.status", json!({}))
            .await
            .unwrap_or_else(|_| json!({}));
        let connected = status
            .pointer("/native_host_bridge/connected")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        HealthSnapshot {
            browser_running: connected,
            extension_bridge_up: connected,
            readiness_cause: if connected {
                None
            } else {
                Some("bridge_down".to_string())
            },
            extension_version: find_string_field(&status, "extension_version"),
        }
    }
}

// ---------------------------------------------------------------------------
// Job executor
// ---------------------------------------------------------------------------

#[async_trait]
trait FleetJobExecutor: Send + Sync {
    /// Run the assignment to a terminal `RunResultV2`. `cancel` is checked
    /// between steps; a set flag means later steps are skipped.
    async fn execute(
        &self,
        assignment: &FleetJobAssignmentV1,
        cancel: Arc<AtomicBool>,
        shared: Arc<FleetShared>,
    ) -> RunResultV2;
}

/// Production executor: resolve the manifest via the content-hash cache and run
/// it through the shared runner with an in-process transport.
struct InProcessJobExecutor {
    state: Arc<SupervisorState>,
    cache: Arc<WorkflowCache>,
    fetcher: Arc<HttpManifestFetcher>,
    default_deadline_secs: u64,
}

#[async_trait]
impl FleetJobExecutor for InProcessJobExecutor {
    async fn execute(
        &self,
        assignment: &FleetJobAssignmentV1,
        cancel: Arc<AtomicBool>,
        shared: Arc<FleetShared>,
    ) -> RunResultV2 {
        let run_id = format!("fleet-{}", assignment.job_id);
        let content_hash = strip_hash_prefix(&assignment.workflow_hash);

        // Resolve the exact bytes the server dispatched, verified by content hash.
        if let Err(err) = self
            .cache
            .get(&assignment.workflow_id, &content_hash, &*self.fetcher)
            .await
        {
            return failed_result(
                &run_id,
                &assignment.workflow_id,
                format!("workflow cache/fetch failed: {err}"),
            );
        }
        let path = self
            .cache
            .root()
            .join(&assignment.workflow_id)
            .join(format!("{content_hash}.json"));

        let params = params_to_string_map(&assignment.params);
        let workflow = match load_workflow_for_run(&path.to_string_lossy(), &params) {
            Ok(workflow) => workflow,
            Err(err) => {
                return failed_result(
                    &run_id,
                    &assignment.workflow_id,
                    format!("workflow load failed: {err}"),
                )
            }
        };

        let deadline_secs = if assignment.execution_deadline_seconds > 0 {
            assignment.execution_deadline_seconds
        } else {
            self.default_deadline_secs
        };
        let deadline = Duration::from_secs(deadline_secs);

        let transport = InProcessTransport {
            state: self.state.clone(),
            cancel: cancel.clone(),
        };
        let sink = FleetRunSink { shared };
        let opts = RunOptions {
            run_id: run_id.clone(),
            workflow_hash: Some(assignment.workflow_hash.clone()),
            params,
            deadline: Some(deadline),
            session: SessionSpec {
                origin: Some("fleet".to_string()),
                job_id: Some(assignment.job_id.clone()),
                ..SessionSpec::default()
            },
            snapshot_mode: SnapshotMode::None,
            workflow_path: path.to_string_lossy().into_owned(),
        };

        // The runner does not enforce a global deadline itself; wrap it here.
        match tokio::time::timeout(
            deadline,
            execute_workflow(&transport, &sink, workflow, opts),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => timed_out_result(&run_id, &assignment.workflow_id),
        }
    }
}

/// In-process `StepTransport`: drives `SupervisorState::dispatch` directly (never
/// the local socket). A set cancel flag short-circuits the *next* step so the
/// in-flight step completes and subsequent steps are skipped.
struct InProcessTransport {
    state: Arc<SupervisorState>,
    cancel: Arc<AtomicBool>,
}

#[async_trait]
impl StepTransport for InProcessTransport {
    async fn call(
        &self,
        method: &str,
        params: Value,
        timeout_ms: u64,
    ) -> Result<Value, TransportError> {
        if method == "browser.execute_step" && self.cancel.load(Ordering::SeqCst) {
            return Err(TransportError::Call(anyhow!(
                "fleet job cancelled before step"
            )));
        }
        let fut = self.state.dispatch(method, params);
        if timeout_ms == 0 {
            fut.await.map_err(TransportError::Call)
        } else {
            match tokio::time::timeout(Duration::from_millis(timeout_ms), fut).await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(err)) => Err(TransportError::Call(err)),
                Err(_) => Err(TransportError::Timeout),
            }
        }
    }
}

/// Records step progress into the shared in-memory state (liveness signal).
struct FleetRunSink {
    shared: Arc<FleetShared>,
}

impl RunEventSink for FleetRunSink {
    fn on_step_start(&self, _idx: usize, _total: usize, _step_id: &str, _step_type: &str) {
        self.shared.note_step();
    }
}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

struct RunningJob {
    job_id: String,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

struct FleetLoop {
    api: Arc<dyn FleetApi>,
    executor: Arc<dyn FleetJobExecutor>,
    health: Arc<dyn HealthProbe>,
    shared: Arc<FleetShared>,
    journal: Arc<Journal>,
    results_dir: PathBuf,
    config_interval_secs: Option<u64>,
    /// Direct base-interval override (ms) — takes precedence over the env var.
    /// Used by tests for deterministic cadence without touching process env.
    interval_ms_override: Option<u64>,
    started_at_ms: i64,
    cli_version: String,
    state: Arc<SupervisorState>,
}

impl FleetLoop {
    async fn run(self) {
        // Crash recovery + housekeeping before the first poll.
        self.reconcile_startup().await;
        let _ = self.journal.compact();
        self.shared.refresh_tail(&self.journal);
        self.flush_pending().await;

        let mut failures: u32 = 0;
        let mut server_interval: Option<u64> = None;
        let mut current: Option<RunningJob> = None;

        loop {
            if self.shared.disabled.load(Ordering::SeqCst) {
                self.shared.set_state(
                    LoopState::Disabled,
                    Some("disabled via fleet.disable".to_string()),
                );
                if let Some(job) = &current {
                    job.cancel.store(true, Ordering::SeqCst);
                }
                break;
            }

            // Reap a finished job and post its result.
            if current
                .as_ref()
                .map(|j| j.handle.is_finished())
                .unwrap_or(false)
            {
                if let Some(job) = current.take() {
                    let _ = job.handle.await;
                    self.shared.clear_cancel();
                    self.shared.refresh_tail(&self.journal);
                }
            }
            // Retry any unposted results every tick.
            self.flush_pending().await;

            let active_ids: Vec<String> = current
                .as_ref()
                .map(|job| vec![job.job_id.clone()])
                .unwrap_or_default();
            self.shared.set_active_job(active_ids.first().cloned());

            let health = self.build_health(&active_ids).await;
            let request = FleetPollRequestV1 {
                health,
                active_job_ids: active_ids.clone(),
                max_jobs: 1,
            };
            self.shared.set_last_poll(now_ms());

            match self.api.poll(&request).await {
                Ok(response) => {
                    failures = 0;
                    server_interval = (response.poll_interval_seconds > 0)
                        .then_some(response.poll_interval_seconds);

                    match response.device_status {
                        FleetDeviceStatusV1::Revoked => {
                            self.shared.set_state(
                                LoopState::StoppedRevoked,
                                Some("device revoked".to_string()),
                            );
                            if let Some(job) = &current {
                                job.cancel.store(true, Ordering::SeqCst);
                            }
                            break;
                        }
                        FleetDeviceStatusV1::Dormant => {
                            self.shared.set_state(
                                LoopState::StoppedDormant,
                                Some("device dormant".to_string()),
                            );
                            if let Some(job) = &current {
                                job.cancel.store(true, Ordering::SeqCst);
                            }
                            break;
                        }
                        FleetDeviceStatusV1::Active => {}
                    }

                    // Cooperative cancellation of the running job.
                    if let Some(job) = current.as_ref() {
                        if response.cancellations.iter().any(|id| id == &job.job_id) {
                            job.cancel.store(true, Ordering::SeqCst);
                        }
                    }

                    // Claim a new job only when idle (one job at a time).
                    if should_claim_job(current.is_none(), self.state.automation_paused()) {
                        if let Some(assignment) = response.jobs.into_iter().next() {
                            current = self.maybe_start_job(assignment).await;
                            if let Some(job) = &current {
                                self.shared.set_cancel(job.cancel.clone());
                            }
                        }
                    }

                    self.shared.set_state(LoopState::Polling, None);
                    let base = self.base_interval_ms(server_interval);
                    sleep_ms(jittered_ms(base)).await;
                }
                Err(FleetCallError::Stop { code, message }) => {
                    let state = if code == error_codes::DEVICE_DORMANT {
                        LoopState::StoppedDormant
                    } else {
                        LoopState::StoppedRevoked
                    };
                    self.shared.set_state(state, Some(message));
                    if let Some(job) = &current {
                        job.cancel.store(true, Ordering::SeqCst);
                    }
                    break;
                }
                Err(FleetCallError::Network(message)) => {
                    failures = failures.saturating_add(1);
                    tracing::debug!("fleet poll network error: {message}");
                    let base = self.base_interval_ms(server_interval);
                    sleep_ms(backoff_ms(base, failures)).await;
                }
            }
        }

        // Give any in-flight job a brief window to land its result, then flush.
        if let Some(job) = current.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), job.handle).await;
            self.flush_pending().await;
        }
        self.shared.refresh_tail(&self.journal);
    }

    async fn build_health(&self, active_ids: &[String]) -> DeviceHealthV1 {
        let snapshot = self.health.probe().await;
        let uptime_seconds = ((now_ms() - self.started_at_ms).max(0) as u64) / 1_000;
        DeviceHealthV1 {
            browser_running: snapshot.browser_running,
            extension_bridge_up: snapshot.extension_bridge_up,
            readiness_cause: snapshot.readiness_cause,
            cli_version: self.cli_version.clone(),
            extension_version: snapshot.extension_version,
            uptime_seconds,
            running_job_ids: active_ids.to_vec(),
        }
    }

    fn base_interval_ms(&self, server_interval: Option<u64>) -> u64 {
        if let Some(ms) = self.interval_ms_override {
            return ms.max(1);
        }
        if let Some(ms) =
            env_trimmed(FLEET_POLL_INTERVAL_MS_ENV).and_then(|v| v.parse::<u64>().ok())
        {
            return ms.max(1);
        }
        let secs = server_interval
            .filter(|s| *s > 0)
            .or_else(|| self.config_interval_secs.filter(|s| *s > 0))
            .unwrap_or(DEFAULT_POLL_INTERVAL_SECS);
        secs.saturating_mul(1_000).max(1)
    }

    /// Accept + spawn a job, journaling `accepted` BEFORE any execution. Returns
    /// `None` (no execution) when the job is already finished/posted (dedupe).
    async fn maybe_start_job(&self, assignment: FleetJobAssignmentV1) -> Option<RunningJob> {
        if let Some(entry) = self.journal.latest(&assignment.job_id) {
            if matches!(entry.state, JournalState::Finished | JournalState::Posted) {
                // Already completed: never re-execute; a persisted result (if any)
                // will be re-posted (server dedupes) on the next flush.
                self.flush_pending().await;
                return None;
            }
        }

        // Journal `accepted` on disk before doing any execution work.
        let _ = self.journal.append(JournalEntry::from_assignment(
            &assignment,
            JournalState::Accepted,
            None,
        ));
        self.shared.refresh_tail(&self.journal);

        let cancel = Arc::new(AtomicBool::new(false));
        let executor = self.executor.clone();
        let journal = self.journal.clone();
        let results_dir = self.results_dir.clone();
        let shared = self.shared.clone();
        let assignment_task = assignment.clone();
        let cancel_task = cancel.clone();
        let started_at_ms = now_ms();
        let state = self.state.clone();

        let handle = tokio::spawn(async move {
            let _ = journal.append(JournalEntry::from_assignment(
                &assignment_task,
                JournalState::Running,
                None,
            ));
            shared.refresh_tail(&journal);
            state
                .emit_fleet_run_notice(
                    &assignment_task.job_id,
                    &assignment_task.workflow_id,
                    "started",
                    None,
                )
                .await;

            let mut result = executor
                .execute(&assignment_task, cancel_task.clone(), shared.clone())
                .await;
            if result.status != RunStatusV2::Succeeded && result.failure_summary.is_none() {
                let error = result.error.as_ref();
                let code = error.map_or("unknown", |e| e.code.as_str());
                let message = error.map_or("workflow failed", |e| e.message.as_str());
                result.failure_summary = Some(crate::workflow_health::failure_summary(
                    &assignment_task.workflow_hash,
                    None,
                    code,
                    message,
                ));
            }
            let cancelled = cancel_task.load(Ordering::SeqCst);
            let terminal = terminal_status(&result, cancelled);
            let phase = match &terminal {
                FleetJobTerminalStatusV1::Succeeded => "succeeded",
                _ => "failed",
            };
            let error_class = result
                .failure_summary
                .as_ref()
                .map(|f| f.error_class.as_str());
            state
                .emit_fleet_run_notice(
                    &assignment_task.job_id,
                    &assignment_task.workflow_id,
                    phase,
                    error_class,
                )
                .await;
            let error = terminal_error(&terminal, &result);
            let finished_at_ms = now_ms();
            let _ = state.append_run_and_refresh(AppendRun {
                origin: &format!("fleet:{}", assignment_task.job_id),
                workflow_hash: Some(&assignment_task.workflow_hash),
                started_at: started_at_ms as i64,
                ended_at: finished_at_ms as i64,
                params: &assignment_task.params,
                result: &result,
            });

            let post = FleetResultPostV1 {
                job_id: assignment_task.job_id.clone(),
                status: terminal.clone(),
                run_result: result,
                error,
                started_at_ms,
                finished_at_ms,
            };
            // Persist BEFORE journaling finished so a crash never loses the result.
            let _ = persist_result(&results_dir, &assignment_task.job_id, &post);
            let _ = journal.append(JournalEntry::from_assignment(
                &assignment_task,
                JournalState::Finished,
                Some(terminal),
            ));
            shared.refresh_tail(&journal);
        });

        Some(RunningJob {
            job_id: assignment.job_id,
            cancel,
            handle,
        })
    }

    /// Post every persisted-but-unposted result; on ack, journal `posted` and
    /// delete the stored result. A `deduped` ack counts as posted.
    async fn flush_pending(&self) {
        let read_dir = match fs::read_dir(&self.results_dir) {
            Ok(read_dir) => read_dir,
            Err(_) => return,
        };
        let mut files: Vec<PathBuf> = Vec::new();
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                files.push(path);
            }
        }

        for path in files {
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            let post: FleetResultPostV1 = match serde_json::from_slice(&bytes) {
                Ok(post) => post,
                Err(_) => {
                    let _ = fs::remove_file(&path);
                    continue;
                }
            };
            match self.api.post_result(&post.job_id, &post).await {
                Ok(_ack) => {
                    let _ = self.journal.append(JournalEntry::marker(
                        &post.job_id,
                        &post.run_result.workflow_id,
                        JournalState::Posted,
                        Some(post.status.clone()),
                    ));
                    let _ = fs::remove_file(&path);
                    self.shared.refresh_tail(&self.journal);
                }
                Err(_) => { /* keep the file; retried next tick */ }
            }
        }
    }

    /// Reconcile the journal on startup:
    ///   accepted/running w/o finished  → persist an Aborted result (unless a real
    ///                                     result was already persisted) + journal finished,
    ///   finished w/o result file       → persist an Aborted fallback,
    ///   posted                         → drop any stale result file.
    async fn reconcile_startup(&self) {
        for entry in self.journal.latest_per_job() {
            let result_path = self.results_dir.join(result_filename(&entry.job_id));
            match entry.state {
                JournalState::Accepted | JournalState::Running => {
                    if let Ok(bytes) = fs::read(&result_path) {
                        if let Ok(post) = serde_json::from_slice::<FleetResultPostV1>(&bytes) {
                            // A real result survived the crash; mark finished so it posts.
                            let _ = self.journal.append(JournalEntry::carry(
                                &entry,
                                JournalState::Finished,
                                Some(post.status),
                            ));
                            continue;
                        }
                    }
                    let post = aborted_post(&entry.job_id, &entry.workflow_id);
                    let _ = persist_result(&self.results_dir, &entry.job_id, &post);
                    let _ = self.journal.append(JournalEntry::carry(
                        &entry,
                        JournalState::Finished,
                        Some(FleetJobTerminalStatusV1::Aborted),
                    ));
                }
                JournalState::Finished => {
                    if !result_path.exists() {
                        let post = aborted_post(&entry.job_id, &entry.workflow_id);
                        let _ = persist_result(&self.results_dir, &entry.job_id, &post);
                    }
                }
                JournalState::Posted => {
                    let _ = fs::remove_file(&result_path);
                }
            }
        }
    }
}

fn should_claim_job(idle: bool, paused: bool) -> bool {
    idle && !paused
}

// ---------------------------------------------------------------------------
// Result persistence + helpers
// ---------------------------------------------------------------------------

fn persist_result(dir: &Path, job_id: &str, post: &FleetResultPostV1) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let bytes = serde_json::to_vec(post).map_err(io_error)?;
    let name = result_filename(job_id);
    let tmp = dir.join(format!(".{name}.tmp"));
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, dir.join(&name))?;
    Ok(())
}

/// Storage-safe filename for a job's persisted result (content carries the real
/// `job_id`; the filename is only a key).
fn result_filename(job_id: &str) -> String {
    let sanitized: String = job_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{sanitized}.json")
}

fn terminal_status(result: &RunResultV2, cancelled: bool) -> FleetJobTerminalStatusV1 {
    if cancelled {
        return FleetJobTerminalStatusV1::Cancelled;
    }
    match result.status {
        RunStatusV2::Succeeded => FleetJobTerminalStatusV1::Succeeded,
        RunStatusV2::TimedOut => FleetJobTerminalStatusV1::TimedOut,
        RunStatusV2::Cancelled => FleetJobTerminalStatusV1::Cancelled,
        RunStatusV2::Failed | RunStatusV2::PolicyBlocked => FleetJobTerminalStatusV1::Failed,
    }
}

fn terminal_error(terminal: &FleetJobTerminalStatusV1, result: &RunResultV2) -> Option<String> {
    match terminal {
        FleetJobTerminalStatusV1::Succeeded => None,
        FleetJobTerminalStatusV1::Cancelled => Some("cancelled by control plane".to_string()),
        FleetJobTerminalStatusV1::TimedOut => Some("execution deadline exceeded".to_string()),
        FleetJobTerminalStatusV1::Aborted => Some("supervisor restarted mid-run".to_string()),
        FleetJobTerminalStatusV1::Failed => Some(
            result
                .error
                .as_ref()
                .map(|e| e.message.clone())
                .unwrap_or_else(|| "workflow failed".to_string()),
        ),
    }
}

fn params_to_string_map(params: &Value) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Some(object) = params.as_object() {
        for (key, value) in object {
            let text = match value {
                Value::String(s) => s.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            map.insert(key.clone(), text);
        }
    }
    map
}

/// The cache wants a bare 64-char hex digest; assignments may prefix `sha256:`.
fn strip_hash_prefix(hash: &str) -> String {
    hash.trim()
        .strip_prefix("sha256:")
        .unwrap_or(hash.trim())
        .to_ascii_lowercase()
}

fn aborted_post(job_id: &str, workflow_id: &str) -> FleetResultPostV1 {
    let now = now_ms();
    FleetResultPostV1 {
        job_id: job_id.to_string(),
        status: FleetJobTerminalStatusV1::Aborted,
        run_result: RunResultV2 {
            version: RUN_RESULT_VERSION.to_string(),
            run_id: format!("fleet-abort-{job_id}"),
            workflow_id: workflow_id.to_string(),
            status: RunStatusV2::Failed,
            output: None,
            artifacts: Vec::new(),
            warnings: Vec::new(),
            steps: Vec::new(),
            debug: None,
            error: Some(RunErrorV1 {
                code: "supervisor_restarted".to_string(),
                message: "supervisor restarted mid-run".to_string(),
                step_id: None,
                retry_hint: None,
            }),
            failure_summary: None,
        },
        error: Some("supervisor restarted mid-run".to_string()),
        started_at_ms: now,
        finished_at_ms: now,
    }
}

fn failed_result(run_id: &str, workflow_id: &str, message: String) -> RunResultV2 {
    RunResultV2 {
        version: RUN_RESULT_VERSION.to_string(),
        run_id: run_id.to_string(),
        workflow_id: workflow_id.to_string(),
        status: RunStatusV2::Failed,
        output: None,
        artifacts: Vec::new(),
        warnings: Vec::new(),
        steps: Vec::new(),
        debug: None,
        error: Some(RunErrorV1 {
            code: "fleet_execution_error".to_string(),
            message,
            step_id: None,
            retry_hint: None,
        }),
        failure_summary: None,
    }
}

fn timed_out_result(run_id: &str, workflow_id: &str) -> RunResultV2 {
    RunResultV2 {
        version: RUN_RESULT_VERSION.to_string(),
        run_id: run_id.to_string(),
        workflow_id: workflow_id.to_string(),
        status: RunStatusV2::TimedOut,
        output: None,
        artifacts: Vec::new(),
        warnings: Vec::new(),
        steps: Vec::new(),
        debug: None,
        error: Some(RunErrorV1 {
            code: "execution_deadline_exceeded".to_string(),
            message: "execution deadline exceeded".to_string(),
            step_id: None,
            retry_hint: None,
        }),
        failure_summary: None,
    }
}

fn find_string_field(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(found)) = map.get(key) {
                if !found.is_empty() {
                    return Some(found.clone());
                }
            }
            map.values()
                .find_map(|nested| find_string_field(nested, key))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|nested| find_string_field(nested, key)),
        _ => None,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Apply ±`JITTER_FRACTION` jitter to a base interval, unless jitter is disabled.
fn jittered_ms(base_ms: u64) -> u64 {
    if base_ms == 0 {
        return 0;
    }
    if env_trimmed(FLEET_DISABLE_JITTER_ENV).is_some() {
        return base_ms;
    }
    let span = (base_ms as f64 * JITTER_FRACTION) as i64;
    if span <= 0 {
        return base_ms;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as i64)
        .unwrap_or(0);
    let offset = (nanos % (2 * span + 1)) - span;
    (base_ms as i64 + offset).max(1) as u64
}

/// Exponential backoff: `base * 2^(failures-1)`, capped at 5 minutes.
fn backoff_ms(base_ms: u64, failures: u32) -> u64 {
    let shift = failures.saturating_sub(1).min(20);
    let factor = 1u64 << shift;
    base_ms.saturating_mul(factor).min(MAX_BACKOFF_MS).max(1)
}

async fn sleep_ms(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

fn io_error<E: std::fmt::Display>(err: E) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err.to_string())
}

#[cfg(test)]
mod tests;
