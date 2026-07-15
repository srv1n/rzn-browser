//! FLA-T-0003 tests: journal + reconciliation + poll-loop behavior.
//!
//! The control-plane HTTP path is covered both with a real axum mock server
//! (`HttpFleetApi` wire round-trip + 403 revoked parsing) and with an in-memory
//! `MockApi` so the loop tests are deterministic. Job execution uses a mock
//! runner so no browser is required.

use super::*;

use std::collections::VecDeque;
use std::sync::atomic::AtomicUsize;

use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};

use rzn_contracts::fleet_v1::FleetErrorV1;

// ---------------------------------------------------------------------------
// Fixtures / helpers
// ---------------------------------------------------------------------------

fn temp_dir(tag: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rzn-fleet-loop-{tag}-{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn assignment(job_id: &str) -> FleetJobAssignmentV1 {
    FleetJobAssignmentV1 {
        job_id: job_id.to_string(),
        workflow_id: "wf_demo".to_string(),
        workflow_hash: "a".repeat(64),
        params: json!({ "query": "shoes" }),
        priority: 0,
        single_delivery: true,
        execution_deadline_seconds: 30,
        lease_expires_at_ms: 0,
    }
}

fn poll_active_empty() -> FleetPollResponseV1 {
    FleetPollResponseV1 {
        jobs: Vec::new(),
        cancellations: Vec::new(),
        poll_interval_seconds: 0,
        device_status: FleetDeviceStatusV1::Active,
    }
}

fn poll_with_job(a: FleetJobAssignmentV1) -> FleetPollResponseV1 {
    FleetPollResponseV1 {
        jobs: vec![a],
        ..poll_active_empty()
    }
}

fn poll_with_cancel(job_id: &str) -> FleetPollResponseV1 {
    FleetPollResponseV1 {
        cancellations: vec![job_id.to_string()],
        ..poll_active_empty()
    }
}

fn poll_revoked() -> FleetPollResponseV1 {
    FleetPollResponseV1 {
        device_status: FleetDeviceStatusV1::Revoked,
        ..poll_active_empty()
    }
}

fn succeeded_run_result(run_id: &str, workflow_id: &str) -> RunResultV2 {
    RunResultV2 {
        version: RUN_RESULT_VERSION.to_string(),
        run_id: run_id.to_string(),
        workflow_id: workflow_id.to_string(),
        status: RunStatusV2::Succeeded,
        output: Some(json!({ "ok": true })),
        artifacts: Vec::new(),
        warnings: Vec::new(),
        steps: Vec::new(),
        debug: None,
        error: None,
        failure_summary: None,
    }
}

async fn wait_until<F: Fn() -> bool>(pred: F, label: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if pred() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for: {label}");
}

// ---------------------------------------------------------------------------
// Mock control-plane API
// ---------------------------------------------------------------------------

enum MockPoll {
    Ok(FleetPollResponseV1),
    Stop(String),
    Network(String),
}

struct MockInner {
    script: VecDeque<MockPoll>,
    default: FleetPollResponseV1,
    posts: Vec<FleetResultPostV1>,
    poll_count: usize,
    ack_deduped: bool,
    fail_next_posts: usize,
}

struct MockApi {
    inner: Mutex<MockInner>,
}

impl MockApi {
    fn new(script: Vec<MockPoll>, default: FleetPollResponseV1) -> Self {
        Self {
            inner: Mutex::new(MockInner {
                script: script.into_iter().collect(),
                default,
                posts: Vec::new(),
                poll_count: 0,
                ack_deduped: false,
                fail_next_posts: 0,
            }),
        }
    }

    fn with_deduped(self, deduped: bool) -> Self {
        self.inner.lock().unwrap().ack_deduped = deduped;
        self
    }

    fn with_failing_posts(self, n: usize) -> Self {
        self.inner.lock().unwrap().fail_next_posts = n;
        self
    }

    fn posts(&self) -> Vec<FleetResultPostV1> {
        self.inner.lock().unwrap().posts.clone()
    }

    fn poll_count(&self) -> usize {
        self.inner.lock().unwrap().poll_count
    }
}

#[async_trait]
impl FleetApi for MockApi {
    async fn poll(&self, _req: &FleetPollRequestV1) -> Result<FleetPollResponseV1, FleetCallError> {
        let mut inner = self.inner.lock().unwrap();
        inner.poll_count += 1;
        match inner.script.pop_front() {
            Some(MockPoll::Ok(resp)) => Ok(resp),
            Some(MockPoll::Stop(code)) => Err(FleetCallError::Stop {
                code: code.clone(),
                message: format!("stop: {code}"),
            }),
            Some(MockPoll::Network(msg)) => Err(FleetCallError::Network(msg)),
            None => Ok(inner.default.clone()),
        }
    }

    async fn post_result(
        &self,
        _job_id: &str,
        post: &FleetResultPostV1,
    ) -> Result<FleetResultAckV1, FleetCallError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.fail_next_posts > 0 {
            inner.fail_next_posts -= 1;
            return Err(FleetCallError::Network("mock post failure".to_string()));
        }
        inner.posts.push(post.clone());
        let deduped = inner.ack_deduped;
        Ok(FleetResultAckV1 {
            ok: true,
            job_status: "accepted".to_string(),
            deduped,
        })
    }
}

// ---------------------------------------------------------------------------
// Mock executor + health
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum ExecBehavior {
    Succeed,
    RunUntilCancelled,
}

struct MockExecutor {
    behavior: ExecBehavior,
    executed: Mutex<Vec<String>>,
    observed_cancel: AtomicBool,
}

impl MockExecutor {
    fn new(behavior: ExecBehavior) -> Self {
        Self {
            behavior,
            executed: Mutex::new(Vec::new()),
            observed_cancel: AtomicBool::new(false),
        }
    }

    fn executed(&self) -> Vec<String> {
        self.executed.lock().unwrap().clone()
    }
}

#[async_trait]
impl FleetJobExecutor for MockExecutor {
    async fn execute(
        &self,
        a: &FleetJobAssignmentV1,
        cancel: Arc<AtomicBool>,
        _shared: Arc<FleetShared>,
    ) -> RunResultV2 {
        self.executed.lock().unwrap().push(a.job_id.clone());
        let run_id = format!("run-{}", a.job_id);
        match self.behavior {
            ExecBehavior::Succeed => succeeded_run_result(&run_id, &a.workflow_id),
            ExecBehavior::RunUntilCancelled => {
                for _ in 0..500 {
                    if cancel.load(Ordering::SeqCst) {
                        self.observed_cancel.store(true, Ordering::SeqCst);
                        return succeeded_run_result(&run_id, &a.workflow_id);
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                succeeded_run_result(&run_id, &a.workflow_id)
            }
        }
    }
}

struct StubHealth;

#[async_trait]
impl HealthProbe for StubHealth {
    async fn probe(&self) -> HealthSnapshot {
        HealthSnapshot {
            browser_running: true,
            extension_bridge_up: true,
            readiness_cause: None,
            extension_version: Some("test-ext".to_string()),
        }
    }
}

fn build_loop(
    dir: &Path,
    api: Arc<dyn FleetApi>,
    executor: Arc<dyn FleetJobExecutor>,
    shared: Arc<FleetShared>,
    journal: Arc<Journal>,
) -> FleetLoop {
    FleetLoop {
        api,
        executor,
        health: Arc::new(StubHealth),
        shared,
        journal,
        results_dir: dir.join("results"),
        config_interval_secs: None,
        interval_ms_override: Some(15),
        started_at_ms: now_ms(),
        cli_version: "test".to_string(),
        state: Arc::new(crate::supervisor::SupervisorState::new(
            crate::supervisor::SupervisorConfig {
                app_base: Some(dir.to_path_buf()),
            },
        )),
    }
}

fn journal_states(journal: &Journal, job_id: &str) -> Vec<JournalState> {
    journal
        .entries
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.job_id == job_id)
        .map(|e| e.state)
        .collect()
}

#[test]
fn paused_fleet_loop_does_not_claim() {
    assert!(!should_claim_job(true, true));
    assert!(!should_claim_job(false, false));
    assert!(should_claim_job(true, false));
}

// ---------------------------------------------------------------------------
// Unit tests: journal, helpers
// ---------------------------------------------------------------------------

#[test]
fn journal_append_latest_and_compact() {
    let dir = temp_dir("journal");
    let journal = Journal::open(dir.join("j.jsonl")).unwrap();
    let a = assignment("job_1");
    journal
        .append(JournalEntry::from_assignment(
            &a,
            JournalState::Accepted,
            None,
        ))
        .unwrap();
    journal
        .append(JournalEntry::from_assignment(
            &a,
            JournalState::Running,
            None,
        ))
        .unwrap();
    let b = assignment("job_2");
    journal
        .append(JournalEntry::from_assignment(
            &b,
            JournalState::Accepted,
            None,
        ))
        .unwrap();
    journal
        .append(JournalEntry::from_assignment(
            &b,
            JournalState::Finished,
            Some(FleetJobTerminalStatusV1::Succeeded),
        ))
        .unwrap();
    journal
        .append(JournalEntry::marker(
            "job_2",
            "wf_demo",
            JournalState::Posted,
            Some(FleetJobTerminalStatusV1::Succeeded),
        ))
        .unwrap();

    assert_eq!(
        journal.latest("job_1").unwrap().state,
        JournalState::Running
    );
    assert_eq!(journal.latest("job_2").unwrap().state, JournalState::Posted);

    // Re-open from disk: entries survive.
    let reopened = Journal::open(dir.join("j.jsonl")).unwrap();
    assert_eq!(
        reopened.latest("job_2").unwrap().state,
        JournalState::Posted
    );

    // Compaction drops the posted job, keeps the in-flight one (latest only).
    reopened.compact().unwrap();
    let after: Vec<_> = reopened.latest_per_job();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].job_id, "job_1");
    assert_eq!(after[0].state, JournalState::Running);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn params_conversion_passes_strings_and_serializes_others() {
    let params = json!({ "s": "text", "n": 7, "b": true, "arr": [1, 2] });
    let map = params_to_string_map(&params);
    assert_eq!(map.get("s").unwrap(), "text");
    assert_eq!(map.get("n").unwrap(), "7");
    assert_eq!(map.get("b").unwrap(), "true");
    assert_eq!(map.get("arr").unwrap(), "[1,2]");
}

#[test]
fn hash_prefix_is_stripped_and_lowercased() {
    let h = "A".repeat(64);
    assert_eq!(strip_hash_prefix(&format!("sha256:{h}")), "a".repeat(64));
    assert_eq!(strip_hash_prefix(&h), "a".repeat(64));
}

#[test]
fn terminal_status_and_backoff_and_jitter() {
    let ok = succeeded_run_result("r", "w");
    assert_eq!(
        terminal_status(&ok, false),
        FleetJobTerminalStatusV1::Succeeded
    );
    assert_eq!(
        terminal_status(&ok, true),
        FleetJobTerminalStatusV1::Cancelled
    );
    let failed = failed_result("r", "w", "boom".to_string());
    assert_eq!(
        terminal_status(&failed, false),
        FleetJobTerminalStatusV1::Failed
    );
    let timed = timed_out_result("r", "w");
    assert_eq!(
        terminal_status(&timed, false),
        FleetJobTerminalStatusV1::TimedOut
    );

    // Backoff grows then caps at 5 minutes.
    assert_eq!(backoff_ms(1_000, 1), 1_000);
    assert_eq!(backoff_ms(1_000, 2), 2_000);
    assert_eq!(backoff_ms(1_000, 3), 4_000);
    assert_eq!(backoff_ms(1_000, 30), MAX_BACKOFF_MS);

    // Jitter stays within ±33% of the base.
    for _ in 0..50 {
        let j = jittered_ms(1_000);
        assert!(j >= 670 && j <= 1_330, "jitter out of bounds: {j}");
    }
}

#[test]
fn fleet_status_json_shape() {
    let shared = FleetShared::new();
    shared.set_state(
        LoopState::StoppedRevoked,
        Some("device revoked".to_string()),
    );
    shared.set_last_poll(1_720_000_000_000);
    shared.set_active_job(Some("job_1".to_string()));

    let value = shared.status_json();
    assert_eq!(value["state"], json!("stopped_revoked"));
    assert_eq!(value["reason"], json!("device revoked"));
    assert_eq!(value["last_poll_ms"], json!(1_720_000_000_000i64));
    assert_eq!(value["active_job_id"], json!("job_1"));
    assert!(value["journal_tail"].is_array());
}

// ---------------------------------------------------------------------------
// A1: happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a1_happy_path_journals_before_execution_and_posts() {
    let dir = temp_dir("a1");
    let journal = Arc::new(Journal::open(dir.join("j.jsonl")).unwrap());
    let shared = Arc::new(FleetShared::new());
    let api = Arc::new(MockApi::new(
        vec![MockPoll::Ok(poll_with_job(assignment("job_a1")))],
        poll_active_empty(),
    ));
    let executor = Arc::new(MockExecutor::new(ExecBehavior::Succeed));

    let fleet = build_loop(
        &dir,
        api.clone(),
        executor.clone(),
        shared.clone(),
        journal.clone(),
    );
    let handle = tokio::spawn(fleet.run());

    wait_until(|| !api.posts().is_empty(), "result posted").await;
    // Heartbeats keep polling between/after jobs.
    wait_until(|| api.poll_count() >= 2, "poll heartbeats").await;
    shared.request_disable();
    handle.await.unwrap();

    // Exactly one succeeded result posted.
    let posts = api.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].job_id, "job_a1");
    assert_eq!(posts[0].status, FleetJobTerminalStatusV1::Succeeded);
    assert_eq!(executor.executed(), vec!["job_a1".to_string()]);

    // accepted was journaled BEFORE execution (running/finished), and posted last.
    let states = journal_states(&journal, "job_a1");
    let accepted = states.iter().position(|s| *s == JournalState::Accepted);
    let running = states.iter().position(|s| *s == JournalState::Running);
    let finished = states.iter().position(|s| *s == JournalState::Finished);
    let posted = states.iter().position(|s| *s == JournalState::Posted);
    assert!(accepted.is_some() && running.is_some());
    assert!(accepted < running, "accepted must precede running");
    assert!(running < finished, "running must precede finished");
    assert!(finished < posted, "finished must precede posted");

    let _ = fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// A2: crash recovery + dedupe
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a2_reconcile_running_entry_posts_aborted() {
    let dir = temp_dir("a2-abort");
    let journal = Arc::new(Journal::open(dir.join("j.jsonl")).unwrap());
    // Synthetic crash: a job left at `running` with no finished, no result file.
    let a = assignment("job_crash");
    journal
        .append(JournalEntry::from_assignment(
            &a,
            JournalState::Accepted,
            None,
        ))
        .unwrap();
    journal
        .append(JournalEntry::from_assignment(
            &a,
            JournalState::Running,
            None,
        ))
        .unwrap();

    let api = Arc::new(MockApi::new(vec![], poll_active_empty()));
    let executor = Arc::new(MockExecutor::new(ExecBehavior::Succeed));
    let shared = Arc::new(FleetShared::new());
    let fleet = build_loop(&dir, api.clone(), executor.clone(), shared, journal.clone());

    fleet.reconcile_startup().await;
    fleet.flush_pending().await;

    let posts = api.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].job_id, "job_crash");
    assert_eq!(posts[0].status, FleetJobTerminalStatusV1::Aborted);
    // Never executed; journaled through to posted.
    assert!(executor.executed().is_empty());
    assert_eq!(
        journal.latest("job_crash").unwrap().state,
        JournalState::Posted
    );

    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a2_finished_not_posted_reposts_stored_result() {
    let dir = temp_dir("a2-repost");
    let journal = Arc::new(Journal::open(dir.join("j.jsonl")).unwrap());
    let a = assignment("job_stored");
    journal
        .append(JournalEntry::from_assignment(
            &a,
            JournalState::Finished,
            Some(FleetJobTerminalStatusV1::Succeeded),
        ))
        .unwrap();
    // A persisted result awaiting post.
    let stored = FleetResultPostV1 {
        job_id: "job_stored".to_string(),
        status: FleetJobTerminalStatusV1::Succeeded,
        run_result: succeeded_run_result("run-job_stored", "wf_demo"),
        error: None,
        started_at_ms: 1,
        finished_at_ms: 2,
    };
    persist_result(&dir.join("results"), "job_stored", &stored).unwrap();

    let api = Arc::new(MockApi::new(vec![], poll_active_empty()).with_deduped(true));
    let executor = Arc::new(MockExecutor::new(ExecBehavior::Succeed));
    let shared = Arc::new(FleetShared::new());
    let fleet = build_loop(&dir, api.clone(), executor.clone(), shared, journal.clone());

    fleet.reconcile_startup().await;
    fleet.flush_pending().await;

    let posts = api.posts();
    assert_eq!(posts.len(), 1, "stored result re-posted exactly once");
    assert_eq!(posts[0].job_id, "job_stored");
    assert!(executor.executed().is_empty(), "must not execute");
    assert_eq!(
        journal.latest("job_stored").unwrap().state,
        JournalState::Posted
    );
    // Result file cleaned up after posting.
    assert!(!dir.join("results").join("job_stored.json").exists());

    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a2_already_completed_redelivery_is_deduped_not_reexecuted() {
    let dir = temp_dir("a2-dedupe");
    let journal = Arc::new(Journal::open(dir.join("j.jsonl")).unwrap());
    let a = assignment("job_done");
    // Already finished with a persisted result (completed on a prior run).
    journal
        .append(JournalEntry::from_assignment(
            &a,
            JournalState::Finished,
            Some(FleetJobTerminalStatusV1::Succeeded),
        ))
        .unwrap();
    let stored = FleetResultPostV1 {
        job_id: "job_done".to_string(),
        status: FleetJobTerminalStatusV1::Succeeded,
        run_result: succeeded_run_result("run-job_done", "wf_demo"),
        error: None,
        started_at_ms: 1,
        finished_at_ms: 2,
    };
    persist_result(&dir.join("results"), "job_done", &stored).unwrap();

    let shared = Arc::new(FleetShared::new());
    // Server re-delivers the already-completed job on every poll.
    let api = Arc::new(
        MockApi::new(
            vec![MockPoll::Ok(poll_with_job(a.clone()))],
            poll_with_job(a),
        )
        .with_deduped(true),
    );
    let executor = Arc::new(MockExecutor::new(ExecBehavior::Succeed));
    let fleet = build_loop(
        &dir,
        api.clone(),
        executor.clone(),
        shared.clone(),
        journal.clone(),
    );
    let handle = tokio::spawn(fleet.run());

    wait_until(|| !api.posts().is_empty(), "deduped re-post").await;
    shared.request_disable();
    handle.await.unwrap();

    // Deduped, never re-executed.
    assert!(
        executor.executed().is_empty(),
        "completed job must not re-execute"
    );
    assert!(api.posts().iter().all(|p| p.job_id == "job_done"));
    assert_eq!(
        journal.latest("job_done").unwrap().state,
        JournalState::Posted
    );

    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a2_result_reposts_after_failed_post() {
    let dir = temp_dir("a2-retry");
    let journal = Arc::new(Journal::open(dir.join("j.jsonl")).unwrap());
    let stored = FleetResultPostV1 {
        job_id: "job_retry".to_string(),
        status: FleetJobTerminalStatusV1::Succeeded,
        run_result: succeeded_run_result("run-job_retry", "wf_demo"),
        error: None,
        started_at_ms: 1,
        finished_at_ms: 2,
    };
    persist_result(&dir.join("results"), "job_retry", &stored).unwrap();

    // First post attempt fails (network); second succeeds.
    let api = Arc::new(MockApi::new(vec![], poll_active_empty()).with_failing_posts(1));
    let executor = Arc::new(MockExecutor::new(ExecBehavior::Succeed));
    let shared = Arc::new(FleetShared::new());
    let fleet = build_loop(&dir, api.clone(), executor.clone(), shared, journal.clone());

    fleet.flush_pending().await; // fails; result file stays
    assert!(api.posts().is_empty());
    assert!(dir.join("results").join("job_retry.json").exists());

    fleet.flush_pending().await; // succeeds
    assert_eq!(api.posts().len(), 1);
    assert!(!dir.join("results").join("job_retry.json").exists());

    let _ = fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// A3: revoked stop + network backoff recovery
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a3_network_error_recovers_then_revoked_stops() {
    let dir = temp_dir("a3");
    let journal = Arc::new(Journal::open(dir.join("j.jsonl")).unwrap());
    let shared = Arc::new(FleetShared::new());
    // Network failure (must NOT exit), then healthy, then revoked (must stop).
    let api = Arc::new(MockApi::new(
        vec![
            MockPoll::Network("boom".to_string()),
            MockPoll::Ok(poll_active_empty()),
            MockPoll::Ok(poll_revoked()),
        ],
        poll_active_empty(),
    ));
    let executor = Arc::new(MockExecutor::new(ExecBehavior::Succeed));
    let fleet = build_loop(&dir, api.clone(), executor, shared.clone(), journal);
    let handle = tokio::spawn(fleet.run());

    handle.await.unwrap();

    // Survived the network error (polled past it) and stopped on revoked.
    assert!(
        api.poll_count() >= 3,
        "loop must poll past the network error"
    );
    let status = shared.status_json();
    assert_eq!(status["state"], json!("stopped_revoked"));

    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a3_403_dormant_stop_error_path() {
    let dir = temp_dir("a3-dormant");
    let journal = Arc::new(Journal::open(dir.join("j.jsonl")).unwrap());
    let shared = Arc::new(FleetShared::new());
    let api = Arc::new(MockApi::new(
        vec![MockPoll::Stop(error_codes::DEVICE_DORMANT.to_string())],
        poll_active_empty(),
    ));
    let executor = Arc::new(MockExecutor::new(ExecBehavior::Succeed));
    let fleet = build_loop(&dir, api, executor, shared.clone(), journal);
    tokio::spawn(fleet.run()).await.unwrap();

    assert_eq!(shared.status_json()["state"], json!("stopped_dormant"));

    let _ = fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// A4: cancellation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a4_cancellation_produces_cancelled_result() {
    let dir = temp_dir("a4");
    let journal = Arc::new(Journal::open(dir.join("j.jsonl")).unwrap());
    let shared = Arc::new(FleetShared::new());
    let executor = Arc::new(MockExecutor::new(ExecBehavior::RunUntilCancelled));
    // Poll #1 assigns a long-running job; every later poll cancels it.
    let api = Arc::new(MockApi::new(
        vec![MockPoll::Ok(poll_with_job(assignment("job_cancel")))],
        poll_with_cancel("job_cancel"),
    ));

    let fleet = build_loop(
        &dir,
        api.clone(),
        executor.clone(),
        shared.clone(),
        journal.clone(),
    );
    let handle = tokio::spawn(fleet.run());

    wait_until(|| !api.posts().is_empty(), "cancelled result posted").await;
    shared.request_disable();
    handle.await.unwrap();

    let posts = api.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].job_id, "job_cancel");
    assert_eq!(posts[0].status, FleetJobTerminalStatusV1::Cancelled);
    assert!(
        executor.observed_cancel.load(Ordering::SeqCst),
        "executor must observe the cooperative cancel"
    );
    assert_eq!(
        journal.latest("job_cancel").unwrap().state,
        JournalState::Posted
    );

    let _ = fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// HttpFleetApi wire round-trip against an axum mock server
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct HttpMockState {
    result_posts: Arc<AtomicUsize>,
}

async fn spawn_http_mock(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn http_poll_and_result_round_trip() {
    async fn poll_ok() -> Json<FleetPollResponseV1> {
        Json(poll_with_job(assignment("job_http")))
    }
    async fn result_ok(
        State(state): State<HttpMockState>,
        AxPath(_job_id): AxPath<String>,
        Json(_post): Json<FleetResultPostV1>,
    ) -> Json<FleetResultAckV1> {
        state.result_posts.fetch_add(1, Ordering::SeqCst);
        Json(FleetResultAckV1 {
            ok: true,
            job_status: "accepted".to_string(),
            deduped: false,
        })
    }

    let state = HttpMockState::default();
    let router = Router::new()
        .route("/v1/fleet/poll", post(poll_ok))
        .route("/v1/fleet/jobs/:job_id/result", post(result_ok))
        .with_state(state.clone());
    let base = spawn_http_mock(router).await;

    let api = HttpFleetApi::new(base, "fld_secret");
    let req = FleetPollRequestV1 {
        health: DeviceHealthV1 {
            browser_running: true,
            extension_bridge_up: true,
            readiness_cause: None,
            cli_version: "test".to_string(),
            extension_version: None,
            uptime_seconds: 1,
            running_job_ids: Vec::new(),
        },
        active_job_ids: Vec::new(),
        max_jobs: 1,
    };
    let resp = api.poll(&req).await.expect("poll ok");
    assert_eq!(resp.jobs.len(), 1);
    assert_eq!(resp.jobs[0].job_id, "job_http");

    let post = FleetResultPostV1 {
        job_id: "job_http".to_string(),
        status: FleetJobTerminalStatusV1::Succeeded,
        run_result: succeeded_run_result("run-job_http", "wf_demo"),
        error: None,
        started_at_ms: 1,
        finished_at_ms: 2,
    };
    let ack = api.post_result("job_http", &post).await.expect("result ok");
    assert!(ack.ok);
    assert_eq!(state.result_posts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn http_poll_403_revoked_maps_to_stop() {
    async fn poll_revoked_handler() -> (StatusCode, Json<FleetErrorV1>) {
        (
            StatusCode::FORBIDDEN,
            Json(FleetErrorV1 {
                code: error_codes::DEVICE_REVOKED.to_string(),
                message: "device token revoked".to_string(),
            }),
        )
    }
    let router = Router::new().route("/v1/fleet/poll", post(poll_revoked_handler));
    let base = spawn_http_mock(router).await;

    let api = HttpFleetApi::new(base, "fld_secret");
    let req = FleetPollRequestV1 {
        health: DeviceHealthV1 {
            browser_running: false,
            extension_bridge_up: false,
            readiness_cause: Some("bridge_down".to_string()),
            cli_version: "test".to_string(),
            extension_version: None,
            uptime_seconds: 0,
            running_job_ids: Vec::new(),
        },
        active_job_ids: Vec::new(),
        max_jobs: 1,
    };
    match api.poll(&req).await {
        Err(FleetCallError::Stop { code, .. }) => {
            assert_eq!(code, error_codes::DEVICE_REVOKED);
        }
        other => panic!("expected Stop, got {other:?}"),
    }
}
