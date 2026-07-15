use anyhow::{anyhow, Context, Result};
use rzn_contracts::v2::{RunResultV2, RunStatusV2};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const INDEX_FILE: &str = "index.jsonl";
pub const DEFAULT_RETENTION_COUNT: usize = 500;
pub const DEFAULT_RETENTION_DAYS: i64 = 30;
const MAX_ERROR_BYTES: usize = 2 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunRecord {
    pub run_id: String,
    pub workflow_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_hash: Option<String>,
    pub origin: String,
    pub started_at: i64,
    pub ended_at: i64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failing_step_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    pub error_message: String,
    pub step_count: usize,
    pub params_digest: String,
    pub result_ref: String,
}

#[derive(Debug, Clone)]
pub struct AppendRun<'a> {
    pub origin: &'a str,
    pub workflow_hash: Option<&'a str>,
    pub started_at: i64,
    pub ended_at: i64,
    pub params: &'a Value,
    pub result: &'a RunResultV2,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RunListFilter {
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
    pub workflow_id: Option<String>,
    pub status: Option<String>,
    pub origin: Option<String>,
}

fn default_limit() -> usize {
    50
}

#[derive(Debug, Clone, Copy)]
pub struct GcPolicy {
    pub max_count: usize,
    pub max_age_days: i64,
    pub now_ms: i64,
}

impl Default for GcPolicy {
    fn default() -> Self {
        Self {
            max_count: DEFAULT_RETENTION_COUNT,
            max_age_days: DEFAULT_RETENTION_DAYS,
            now_ms: now_ms(),
        }
    }
}

#[derive(Clone)]
pub struct RunStore {
    root: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl RunStore {
    pub fn open(app_base: impl AsRef<Path>) -> Result<Self> {
        let root = app_base.as_ref().join("runs");
        fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
        Ok(Self {
            root,
            lock: Arc::new(Mutex::new(())),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn append(&self, run: AppendRun<'_>) -> Result<RunRecord> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow!("run store lock poisoned"))?;
        let result_name = format!("{}.json", safe_id(&run.result.run_id));
        let params_name = format!("{}.params.json", safe_id(&run.result.run_id));
        write_json_atomic(&self.root.join(&result_name), run.result)?;
        write_params_0600(&self.root.join(&params_name), run.params)?;
        if run.result.status != RunStatusV2::Succeeded {
            let failure_name = format!("{}.failure.json", safe_id(&run.result.run_id));
            let context = failure_context_from_result(run.result);
            let _ = write_json_atomic(&self.root.join(failure_name), &context);
        }

        let failure = run.result.failure_summary.as_ref();
        let error_message = failure
            .map(|f| f.message.as_str())
            .or_else(|| run.result.error.as_ref().map(|e| e.message.as_str()))
            .unwrap_or("");
        let record = RunRecord {
            run_id: run.result.run_id.clone(),
            workflow_id: run.result.workflow_id.clone(),
            workflow_hash: run.workflow_hash.map(str::to_string),
            origin: run.origin.to_string(),
            started_at: run.started_at,
            ended_at: run.ended_at,
            status: status_str(&run.result.status).to_string(),
            failing_step_index: failure.and_then(|f| f.failing_step_index),
            error_class: failure.map(|f| f.error_class.clone()),
            fingerprint: failure.map(|f| f.fingerprint.clone()),
            error_message: truncate_utf8(error_message, MAX_ERROR_BYTES),
            step_count: run.result.steps.len(),
            params_digest: params_digest(run.params),
            result_ref: result_name,
        };
        let mut index = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join(INDEX_FILE))?;
        let mut line = serde_json::to_vec(&record)?;
        line.push(b'\n');
        index.write_all(&line)?;
        Ok(record)
    }

    pub fn list(&self, mut filter: RunListFilter) -> Result<(usize, Vec<RunRecord>)> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow!("run store lock poisoned"))?;
        filter.limit = filter.limit.clamp(1, 200);
        let mut rows = load_index(&self.root.join(INDEX_FILE))?;
        rows.retain(|row| {
            filter
                .workflow_id
                .as_ref()
                .map_or(true, |v| &row.workflow_id == v)
                && filter.status.as_ref().map_or(true, |v| &row.status == v)
                && filter.origin.as_ref().map_or(true, |v| {
                    if v == "fleet" {
                        row.origin.starts_with("fleet:")
                    } else {
                        &row.origin == v
                    }
                })
        });
        rows.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        let total = rows.len();
        Ok((
            total,
            rows.into_iter()
                .skip(filter.offset)
                .take(filter.limit)
                .collect(),
        ))
    }

    pub fn get(&self, run_id: &str) -> Result<Option<(RunRecord, RunResultV2)>> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow!("run store lock poisoned"))?;
        let record = load_index(&self.root.join(INDEX_FILE))?
            .into_iter()
            .rev()
            .find(|row| row.run_id == run_id);
        let Some(record) = record else {
            return Ok(None);
        };
        let result = serde_json::from_slice(&fs::read(self.root.join(&record.result_ref))?)?;
        Ok(Some((record, result)))
    }

    pub fn read_params(&self, run_id: &str) -> Result<Option<Value>> {
        let path = self.root.join(format!("{}.params.json", safe_id(run_id)));
        match fs::read(path) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn gc(&self, policy: GcPolicy) -> Result<usize> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow!("run store lock poisoned"))?;
        let mut rows = load_index(&self.root.join(INDEX_FILE))?;
        rows.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        let cutoff = policy.now_ms - policy.max_age_days.max(1) * 86_400_000;
        let mut kept = Vec::new();
        let mut removed = 0;
        for (idx, row) in rows.into_iter().enumerate() {
            if idx >= policy.max_count || row.started_at < cutoff {
                remove_run_files(&self.root, &row);
                removed += 1;
            } else {
                kept.push(row);
            }
        }
        rewrite_index(&self.root.join(INDEX_FILE), &kept)?;
        Ok(removed)
    }
}

fn failure_context_from_result(result: &RunResultV2) -> Value {
    let encoded = serde_json::to_value(result).unwrap_or(Value::Null);
    let mut context = serde_json::Map::new();
    let console = result
        .error
        .as_ref()
        .map(|e| truncate_utf8(&e.message, 4 * 1024))
        .filter(|s| !s.is_empty());
    if let Some(value) = console {
        context.insert("console_tail".into(), Value::String(value));
    }
    let mut captured = false;
    for key in ["screenshot_b64", "dom_excerpt"] {
        if let Some(value) = find_string_field(&encoded, key) {
            let limit = if key == "dom_excerpt" {
                4 * 1024
            } else {
                2 * 1024 * 1024
            };
            context.insert(key.into(), Value::String(truncate_utf8(value, limit)));
            captured = true;
        }
    }
    if !captured {
        context.insert(
            "capture_unavailable".into(),
            Value::String("snapshot transport did not provide screenshot or DOM context".into()),
        );
    }
    Value::Object(context)
}

fn find_string_field<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    match value {
        Value::Object(map) => map
            .get(name)
            .and_then(Value::as_str)
            .or_else(|| map.values().find_map(|v| find_string_field(v, name))),
        Value::Array(values) => values.iter().find_map(|v| find_string_field(v, name)),
        _ => None,
    }
}

fn load_index(path: &Path) -> Result<Vec<RunRecord>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let mut rows = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str(&line) {
            rows.push(record);
        }
    }
    Ok(rows)
}

fn rewrite_index(path: &Path, rows: &[RunRecord]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp)?;
    for row in rows {
        serde_json::to_writer(&mut file, row)?;
        file.write_all(b"\n")?;
    }
    fs::rename(tmp, path)?;
    Ok(())
}

fn remove_run_files(root: &Path, row: &RunRecord) {
    let id = safe_id(&row.run_id);
    let _ = fs::remove_file(root.join(&row.result_ref));
    let _ = fs::remove_file(root.join(format!("{id}.params.json")));
    let _ = fs::remove_file(root.join(format!("{id}.failure.json")));
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn write_params_0600(path: &Path, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(&bytes)?;
    }
    #[cfg(not(unix))]
    fs::write(path, bytes)?;
    Ok(())
}

fn status_str(status: &RunStatusV2) -> &'static str {
    match status {
        RunStatusV2::Succeeded => "succeeded",
        RunStatusV2::Failed | RunStatusV2::PolicyBlocked => "failed",
        RunStatusV2::Cancelled => "cancelled",
        RunStatusV2::TimedOut => "timed_out",
    }
}

fn params_digest(value: &Value) -> String {
    hex::encode(Sha256::digest(
        serde_json::to_vec(value).unwrap_or_default(),
    ))
}

fn safe_id(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn truncate_utf8(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rzn_contracts::v2::{DebugBundleV1, FailureSummaryV1, RunErrorV1, RUN_RESULT_VERSION};
    use uuid::Uuid;

    fn temp() -> PathBuf {
        std::env::temp_dir().join(format!("rzn-run-store-{}", Uuid::new_v4()))
    }
    fn result(id: &str, status: RunStatusV2) -> RunResultV2 {
        RunResultV2 {
            version: RUN_RESULT_VERSION.into(),
            run_id: id.into(),
            workflow_id: "wf".into(),
            status,
            output: None,
            artifacts: vec![],
            warnings: vec![],
            steps: vec![],
            debug: None,
            error: None,
            failure_summary: None,
        }
    }

    #[test]
    fn run_store_filters_reload_and_keeps_params_private() {
        let base = temp();
        let store = RunStore::open(&base).unwrap();
        let params = serde_json::json!({"secret":"value"});
        let mut failed = result("r2", RunStatusV2::Failed);
        failed.error = Some(RunErrorV1 {
            code: "x".into(),
            message: "boom".into(),
            step_id: None,
            retry_hint: None,
        });
        failed.failure_summary = Some(FailureSummaryV1 {
            error_class: "engine_error".into(),
            failing_step_index: Some(2),
            fingerprint: "abc".into(),
            message: "boom".into(),
        });
        store
            .append(AppendRun {
                origin: "local_cli",
                workflow_hash: None,
                started_at: 1,
                ended_at: 2,
                params: &params,
                result: &result("r1", RunStatusV2::Succeeded),
            })
            .unwrap();
        store
            .append(AppendRun {
                origin: "fleet:j1",
                workflow_hash: Some("h"),
                started_at: 3,
                ended_at: 4,
                params: &params,
                result: &failed,
            })
            .unwrap();
        let (total, rows) = RunStore::open(&base)
            .unwrap()
            .list(RunListFilter {
                origin: Some("fleet".into()),
                status: Some("failed".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(rows[0].run_id, "r2");
        let index = fs::read_to_string(base.join("runs/index.jsonl")).unwrap();
        assert!(!index.contains("secret"));
        assert!(!index.contains("value"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(base.join("runs/r2.params.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn run_store_gc_and_truncated_tail() {
        let base = temp();
        let store = RunStore::open(&base).unwrap();
        let params = serde_json::json!({});
        for i in 0..3 {
            let r = result(&format!("r{i}"), RunStatusV2::Succeeded);
            store
                .append(AppendRun {
                    origin: "mcp",
                    workflow_hash: None,
                    started_at: i,
                    ended_at: i,
                    params: &params,
                    result: &r,
                })
                .unwrap();
        }
        OpenOptions::new()
            .append(true)
            .open(base.join("runs/index.jsonl"))
            .unwrap()
            .write_all(b"{truncated")
            .unwrap();
        assert_eq!(
            store
                .gc(GcPolicy {
                    max_count: 2,
                    max_age_days: 365,
                    now_ms: 3
                })
                .unwrap(),
            1
        );
        assert!(store.get("r0").unwrap().is_none());
        assert!(store.get("r2").unwrap().is_some());
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn concurrent_store_instances_append_complete_json_lines() {
        let base = temp();
        let threads: Vec<_> = (0..8)
            .map(|worker| {
                let base = base.clone();
                std::thread::spawn(move || {
                    let store = RunStore::open(base).unwrap();
                    let params = serde_json::json!({});
                    for item in 0..25 {
                        let run = result(
                            &format!("worker-{worker}-run-{item}"),
                            RunStatusV2::Succeeded,
                        );
                        store
                            .append(AppendRun {
                                origin: "mcp",
                                workflow_hash: None,
                                started_at: item,
                                ended_at: item,
                                params: &params,
                                result: &run,
                            })
                            .unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let (total, _) = RunStore::open(&base)
            .unwrap()
            .list(RunListFilter {
                limit: 200,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(total, 200);
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn supervisor_control_replay_round_trip_and_bounded_failure_context() {
        let base = temp();
        let store = RunStore::open(&base).unwrap();
        let params = serde_json::json!({"query":"rust", "page":2});
        let mut failed = result("replay-source", RunStatusV2::Failed);
        failed.error = Some(RunErrorV1 {
            code: "timeout".into(),
            message: "x".repeat(8 * 1024),
            step_id: None,
            retry_hint: None,
        });
        failed.debug = Some(DebugBundleV1 {
            trace_id: None,
            events: vec![],
            raw: Some(serde_json::json!({
                "screenshot_b64": "s".repeat(2 * 1024 * 1024 + 100),
                "dom_excerpt": "d".repeat(8 * 1024)
            })),
        });
        store
            .append(AppendRun {
                origin: "local_cli",
                workflow_hash: Some("hash"),
                started_at: 1,
                ended_at: 2,
                params: &params,
                result: &failed,
            })
            .unwrap();
        assert_eq!(store.read_params("replay-source").unwrap(), Some(params));
        let context: Value = serde_json::from_slice(
            &fs::read(base.join("runs/replay-source.failure.json")).unwrap(),
        )
        .unwrap();
        assert!(context["console_tail"].as_str().unwrap().len() <= 4 * 1024);
        assert_eq!(
            context["screenshot_b64"].as_str().unwrap().len(),
            2 * 1024 * 1024
        );
        assert_eq!(context["dom_excerpt"].as_str().unwrap().len(), 4 * 1024);
        assert!(context.get("capture_unavailable").is_none());

        let mut capture_failed = result("capture-failed", RunStatusV2::Failed);
        capture_failed.error = Some(RunErrorV1 {
            code: "snapshot_failed".into(),
            message: "run failure remains primary".into(),
            step_id: None,
            retry_hint: None,
        });
        store
            .append(AppendRun {
                origin: "local_cli",
                workflow_hash: Some("hash"),
                started_at: 3,
                ended_at: 4,
                params: &serde_json::json!({}),
                result: &capture_failed,
            })
            .unwrap();
        let unavailable: Value = serde_json::from_slice(
            &fs::read(base.join("runs/capture-failed.failure.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(unavailable["console_tail"], "run failure remains primary");
        assert!(unavailable.get("capture_unavailable").is_some());
        fs::remove_dir_all(base).ok();
    }
}
