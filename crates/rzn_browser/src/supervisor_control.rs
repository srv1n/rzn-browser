use crate::log_buffer::LogBuffer;
use crate::run_store::{RunListFilter, RunStore};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug, Clone, Serialize)]
pub struct RunningRun {
    pub run_id: String,
    pub workflow_id: String,
    pub origin: String,
    pub step_index: usize,
    pub step_total: usize,
    /// Set for runs claimed by an out-of-process client (CLI). A dead owner frees the slot.
    #[serde(skip)]
    pub(crate) owner_pid: Option<u32>,
    #[serde(skip)]
    pub(crate) started: Instant,
}

fn owner_is_live(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // EPERM means the pid exists but belongs to someone else: treat as live.
        let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
        alive || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}
#[derive(Serialize, Deserialize, Default)]
struct Persisted {
    paused: bool,
}
pub struct SupervisorControl {
    path: PathBuf,
    paused: AtomicBool,
    cancel: AtomicBool,
    running: Mutex<Option<RunningRun>>,
    snapshot_cache: Mutex<(Vec<crate::run_store::RunRecord>, usize)>,
    pub logs: LogBuffer,
}
impl SupervisorControl {
    pub fn open(base: &Path) -> Self {
        let path = base.join("automation_state.json");
        let paused = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Persisted>(&b).ok())
            .is_some_and(|s| s.paused);
        Self {
            path,
            paused: AtomicBool::new(paused),
            cancel: AtomicBool::new(false),
            running: Mutex::new(None),
            snapshot_cache: Mutex::new((Vec::new(), 0)),
            logs: LogBuffer::new(2000),
        }
    }
    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
    pub fn pause(&self, cancel: bool) -> Result<Value> {
        {
            let running = self.running.lock().unwrap();
            self.paused.store(true, Ordering::SeqCst);
            if cancel && running.is_some() {
                self.cancel.store(true, Ordering::SeqCst);
            }
        }
        self.persist()?;
        Ok(json!({"ok":true,"paused":true,"cancel_current":cancel}))
    }
    pub fn resume(&self) -> Result<Value> {
        self.paused.store(false, Ordering::SeqCst);
        self.persist()?;
        Ok(json!({"ok":true,"paused":false}))
    }
    pub fn cancel(&self, requested_run_id: Option<&str>) -> Value {
        let running = self.running.lock().unwrap();
        let Some(current) = running.as_ref() else {
            return json!({"ok":false,"error":"no run is in progress"});
        };
        if requested_run_id.is_some_and(|id| id != current.run_id) {
            return json!({"ok":false,"error":"requested run is not in progress","run_id":current.run_id});
        }
        self.cancel.store(true, Ordering::SeqCst);
        json!({"ok":true,"cancel_requested":true,"run_id":current.run_id})
    }
    pub fn cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
    pub fn cancel_requested_for(&self, run_id: &str) -> bool {
        self.running
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|run| run.run_id == run_id && self.cancel_requested())
    }
    pub fn try_begin_run(
        &self,
        run_id: String,
        workflow_id: String,
        origin: String,
        total: usize,
    ) -> std::result::Result<(), RunningRun> {
        self.try_begin_run_owned(run_id, workflow_id, origin, total, None)
    }
    pub fn try_begin_run_owned(
        &self,
        run_id: String,
        workflow_id: String,
        origin: String,
        total: usize,
        owner_pid: Option<u32>,
    ) -> std::result::Result<(), RunningRun> {
        let mut running = self.running.lock().unwrap();
        if let Some(current) = running.as_ref() {
            if !current.owner_pid.is_some_and(|pid| !owner_is_live(pid)) {
                return Err(current.clone());
            }
            log::warn!(
                "reclaiming run slot {} from exited client pid {:?}",
                current.run_id,
                current.owner_pid
            );
        }
        self.cancel.store(false, Ordering::SeqCst);
        *running = Some(RunningRun {
            run_id,
            workflow_id,
            origin,
            step_index: 0,
            step_total: total,
            owner_pid,
            started: Instant::now(),
        });
        Ok(())
    }
    pub fn begin_run(&self, run_id: String, workflow_id: String, origin: String, total: usize) {
        let _ = self.try_begin_run(run_id, workflow_id, origin, total);
    }
    pub fn step(&self, index: usize, total: usize) {
        if let Some(run) = self.running.lock().unwrap().as_mut() {
            run.step_index = index + 1;
            run.step_total = total;
        }
    }
    pub fn end_run(&self) {
        self.end_run_if(None);
    }
    pub fn end_run_if(&self, run_id: Option<&str>) {
        let mut running = self.running.lock().unwrap();
        if run_id.is_none_or(|id| running.as_ref().is_some_and(|run| run.run_id == id)) {
            *running = None;
            self.cancel.store(false, Ordering::SeqCst);
        }
    }
    pub fn now_running(&self) -> Value {
        self.running
            .lock()
            .unwrap()
            .as_ref()
            .map(|r| {
                json!({
                    "run_id":r.run_id,"workflow_id":r.workflow_id,"origin":r.origin,
                    "step_index":r.step_index,"step_total":r.step_total,
                    "elapsed_ms":r.started.elapsed().as_millis() as u64
                })
            })
            .unwrap_or(Value::Null)
    }
    pub fn refresh_snapshot_cache(&self, store: &RunStore) -> Result<()> {
        let (_, recent) = store.list(RunListFilter {
            limit: 5,
            ..Default::default()
        })?;
        let (_, all) = store.list(RunListFilter {
            limit: 200,
            ..Default::default()
        })?;
        let flagged = crate::workflow_health::snapshots(all)
            .iter()
            .filter(|x| x.pointer("/health/flag").and_then(Value::as_str) != Some("healthy"))
            .count();
        *self.snapshot_cache.lock().unwrap() = (recent, flagged);
        Ok(())
    }
    fn persist(&self) -> Result<()> {
        if let Some(p) = self.path.parent() {
            fs::create_dir_all(p)?
        }
        fs::write(
            &self.path,
            serde_json::to_vec(&Persisted {
                paused: self.paused(),
            })?,
        )
        .context("persist automation pause")
    }
}
pub fn status_snapshot(
    store: &RunStore,
    control: &SupervisorControl,
    native: bool,
    extension: bool,
    fleet: Value,
) -> Result<Value> {
    let (recent, flagged) = control.snapshot_cache.lock().unwrap().clone();
    Ok(
        json!({"supervisor_version":env!("CARGO_PKG_VERSION"),"native_host_connected":native,"extension_connected":extension,"paused":control.paused(),"now_running":control.now_running(),"fleet":fleet,"recent_runs":recent,"flagged_workflows":flagged}),
    )
}
pub fn failure_context(base: &Path, run_id: &str) -> Result<Value> {
    let safe: String = run_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let p = base.join("runs").join(format!("{safe}.failure.json"));
    match fs::read(p) {
        Ok(b) => Ok(serde_json::from_slice(&b)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(json!({"capture_unavailable":"no failure context captured"}))
        }
        Err(e) => Err(e.into()),
    }
}
pub fn export_diagnostics(base: &Path, store: &RunStore, logs: &LogBuffer) -> Result<Value> {
    let dir = base.join("diagnostics");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.zip", chrono::Utc::now().timestamp_millis()));
    let file = fs::File::create(&path)?;
    let mut zip = zip::ZipWriter::new(file);
    let opt = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let (_, runs) = store.list(RunListFilter {
        limit: 20,
        ..Default::default()
    })?;
    zip.start_file("runs/index.json", opt)?;
    zip.write_all(&serde_json::to_vec(&runs)?)?;
    for row in &runs {
        if let Some((_, result)) = store.get(&row.run_id)? {
            zip.start_file(format!("runs/{}", row.result_ref), opt)?;
            zip.write_all(&serde_json::to_vec(&result)?)?;
        }
    }
    zip.start_file("logs.json", opt)?;
    zip.write_all(&serde_json::to_vec(&logs.tail(500, None, None, None))?)?;
    zip.start_file("versions.json", opt)?;
    zip.write_all(
        serde_json::to_string(&json!({"rzn_browser":env!("CARGO_PKG_VERSION")}))?.as_bytes(),
    )?;
    let config = std::env::var("RZN_FLEET_CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| base.join("fleet_config.json"));
    if let Ok(bytes) = fs::read(config) {
        if let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) {
            if let Some(o) = value.as_object_mut() {
                if o.contains_key("device_token") {
                    o.insert("device_token".into(), Value::String("REDACTED".into()));
                }
            }
            zip.start_file("fleet_config.json", opt)?;
            zip.write_all(&serde_json::to_vec(&value)?)?;
        }
    }
    zip.finish()?;
    Ok(json!({"path":path}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp() -> PathBuf {
        std::env::temp_dir().join(format!("rzn-supervisor-control-{}", Uuid::new_v4()))
    }

    #[test]
    fn supervisor_control_pause_persists_across_restart() {
        let base = temp();
        let control = SupervisorControl::open(&base);
        control.pause(false).unwrap();
        assert!(SupervisorControl::open(&base).paused());
        SupervisorControl::open(&base).resume().unwrap();
        assert!(!SupervisorControl::open(&base).paused());
    }

    #[test]
    fn supervisor_control_snapshot_contains_now_running_and_cached_counts() {
        let base = temp();
        let store = RunStore::open(&base).unwrap();
        let control = SupervisorControl::open(&base);
        control.refresh_snapshot_cache(&store).unwrap();
        control.begin_run("run-1".into(), "wf".into(), "local_cli".into(), 3);
        control.step(0, 3);
        let value = status_snapshot(&store, &control, true, false, Value::Null).unwrap();
        assert_eq!(value.pointer("/now_running/run_id"), Some(&json!("run-1")));
        assert_eq!(value["native_host_connected"], true);
        assert_eq!(value["extension_connected"], false);
    }

    #[test]
    fn supervisor_control_accepts_exactly_one_concurrent_run() {
        let control = SupervisorControl::open(&temp());
        assert!(control
            .try_begin_run("run-1".into(), "wf".into(), "local_cli".into(), 1)
            .is_ok());
        let current = control
            .try_begin_run("run-2".into(), "wf".into(), "local_cli".into(), 1)
            .expect_err("second run must be rejected atomically");
        assert_eq!(current.run_id, "run-1");
        control.end_run();
        assert!(control
            .try_begin_run("run-2".into(), "wf".into(), "local_cli".into(), 1)
            .is_ok());
    }

    #[test]
    fn slot_claimed_by_exited_client_is_reclaimed() {
        let control = SupervisorControl::open(&temp());
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead_pid = child.id();
        child.wait().unwrap();
        control
            .try_begin_run_owned(
                "cli-run".into(),
                "wf".into(),
                "cli".into(),
                1,
                Some(dead_pid),
            )
            .unwrap();
        control
            .try_begin_run("next-run".into(), "wf".into(), "dashboard".into(), 1)
            .expect("slot held by an exited client is reclaimed");
        assert_eq!(control.now_running()["run_id"], "next-run");

        control
            .try_begin_run_owned(
                "live-cli".into(),
                "wf".into(),
                "cli".into(),
                1,
                Some(std::process::id()),
            )
            .expect_err("slot is still held by next-run");
    }

    #[test]
    fn releasing_stale_run_does_not_free_current_slot() {
        let control = SupervisorControl::open(&temp());
        control
            .try_begin_run("run-1".into(), "wf".into(), "cli".into(), 1)
            .unwrap();
        control.end_run_if(Some("stale-run"));
        assert_eq!(control.now_running()["run_id"], "run-1");
        control.end_run_if(Some("run-1"));
        assert!(control.now_running().is_null());
    }

    #[test]
    fn cancellation_status_is_scoped_to_current_run() {
        let control = SupervisorControl::open(&temp());
        control
            .try_begin_run("run-1".into(), "wf".into(), "cli".into(), 1)
            .unwrap();
        assert!(!control.cancel_requested_for("run-1"));
        assert_eq!(control.cancel(Some("run-1"))["ok"], true);
        assert!(control.cancel_requested_for("run-1"));
        assert!(!control.cancel_requested_for("run-2"));
        control.end_run_if(Some("run-1"));
        assert!(!control.cancel_requested_for("run-1"));
    }

    #[test]
    fn supervisor_control_diagnostics_excludes_token_and_params_members() {
        let base = temp();
        fs::create_dir_all(base.join("runs")).unwrap();
        fs::write(
            base.join("runs/secret.params.json"),
            br#"{"password":"raw"}"#,
        )
        .unwrap();
        fs::write(
            base.join("fleet_config.json"),
            br#"{"device_token":"token-that-must-not-leak","device_id":"d1"}"#,
        )
        .unwrap();
        let store = RunStore::open(&base).unwrap();
        let output = export_diagnostics(&base, &store, &LogBuffer::new(10)).unwrap();
        let bytes = fs::read(output["path"].as_str().unwrap()).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("token-that-must-not-leak"));
        assert!(!text.contains(".params.json"));
        let file = fs::File::open(output["path"].as_str().unwrap()).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        for i in 0..archive.len() {
            assert!(!archive
                .by_index(i)
                .unwrap()
                .name()
                .ends_with(".params.json"));
        }
    }

    #[tokio::test]
    async fn supervisor_control_paused_refuses_runs_start() {
        let base = temp();
        let state = crate::supervisor::SupervisorState::new(crate::supervisor::SupervisorConfig {
            app_base: Some(base),
        });
        state.dispatch("automation.pause", json!({})).await.unwrap();
        let error = state
            .dispatch("runs.start", json!({"workflow_id":"missing"}))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("paused"));
        let error = state
            .dispatch("runs.claim", json!({"run_id":"claimed-during-pause"}))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("paused"));
        assert!(state.control.now_running().is_null());
    }

    #[tokio::test]
    async fn supervisor_control_native_host_passthrough_enforces_allowlist() {
        let state = crate::supervisor::SupervisorState::new(crate::supervisor::SupervisorConfig {
            app_base: Some(temp()),
        });
        let error = state
            .dispatch(
                "native_host.rpc",
                json!({"method":"runtime.shutdown","params":{}}),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not allowed"));
    }
}
