use crate::run_store::RunRecord;
use rzn_contracts::v2::FailureSummaryV1;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    SelectorNotFound,
    Timeout,
    Navigation,
    AuthWall,
    PopupBlocked,
    EngineError,
    Cancelled,
    Unknown,
}

impl ErrorClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelectorNotFound => "selector_not_found",
            Self::Timeout => "timeout",
            Self::Navigation => "navigation",
            Self::AuthWall => "auth_wall",
            Self::PopupBlocked => "popup_blocked",
            Self::EngineError => "engine_error",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }
}

pub fn classify_error(code: &str, message: &str) -> ErrorClass {
    let text = format!("{code} {message}").to_ascii_lowercase();
    if text.contains("selector") && (text.contains("not found") || text.contains("missing")) {
        ErrorClass::SelectorNotFound
    } else if text.contains("timeout") || text.contains("timed out") || text.contains("deadline") {
        ErrorClass::Timeout
    } else if text.contains("navigation") || text.contains("navigate") || text.contains("net::err")
    {
        ErrorClass::Navigation
    } else if text.contains("auth wall")
        || text.contains("login required")
        || text.contains("unauthorized")
    {
        ErrorClass::AuthWall
    } else if text.contains("popup") && text.contains("block") {
        ErrorClass::PopupBlocked
    } else if text.contains("cancel") || text.contains("abort") {
        ErrorClass::Cancelled
    } else if text.contains("engine") || text.contains("javascript") || text.contains("browser") {
        ErrorClass::EngineError
    } else {
        ErrorClass::Unknown
    }
}

pub fn fingerprint(workflow_hash: &str, step: Option<usize>, class: ErrorClass) -> String {
    let raw = format!(
        "{workflow_hash}:{}:{}",
        step.map(|v| v.to_string()).unwrap_or_default(),
        class.as_str()
    );
    hex::encode(Sha256::digest(raw.as_bytes()))[..16].to_string()
}

pub fn failure_summary(
    workflow_hash: &str,
    step: Option<usize>,
    code: &str,
    message: &str,
) -> FailureSummaryV1 {
    let class = classify_error(code, message);
    FailureSummaryV1 {
        error_class: class.as_str().into(),
        failing_step_index: step,
        fingerprint: fingerprint(workflow_hash, step, class),
        message: message.chars().take(2048).collect(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DominantFingerprint {
    pub step_index: Option<usize>,
    pub error_class: String,
    pub count: usize,
    pub first_seen_at: i64,
}
#[derive(Debug, Clone, Serialize)]
pub struct WorkflowHealth {
    pub flag: &'static str,
    pub consecutive_failures: usize,
    pub success_rate_last_20: f64,
    pub dominant_fingerprint: Option<DominantFingerprint>,
}

pub fn compute_health(records: &[RunRecord]) -> WorkflowHealth {
    let mut rows: Vec<_> = records.iter().collect();
    rows.sort_by_key(|r| r.started_at);
    rows.reverse();
    let consecutive_failures = rows.iter().take_while(|r| r.status != "succeeded").count();
    let recent: Vec<_> = rows.iter().take(20).collect();
    let successes = recent.iter().filter(|r| r.status == "succeeded").count();
    let success_rate_last_20 = if recent.is_empty() {
        1.0
    } else {
        successes as f64 / recent.len() as f64
    };
    let broken_after = env_threshold("RZN_HEALTH_BROKEN_AFTER", 3);
    let degraded_after = env_threshold("RZN_HEALTH_DEGRADED_AFTER", 3);
    let leading = rows
        .iter()
        .take(consecutive_failures)
        .filter_map(|r| r.fingerprint.as_ref())
        .next()
        .cloned();
    let same = leading.as_ref().map_or(0, |fp| {
        rows.iter()
            .take(consecutive_failures)
            .take_while(|r| r.fingerprint.as_ref() == Some(fp))
            .count()
    });
    let dominant_fingerprint = if same >= broken_after {
        leading.and_then(|fp| {
            let matching: Vec<_> = rows
                .iter()
                .take(same)
                .filter(|r| r.fingerprint.as_ref() == Some(&fp))
                .collect();
            matching.first().map(|latest| DominantFingerprint {
                step_index: latest.failing_step_index,
                error_class: latest
                    .error_class
                    .clone()
                    .unwrap_or_else(|| "unknown".into()),
                count: same,
                first_seen_at: matching
                    .iter()
                    .map(|r| r.started_at)
                    .min()
                    .unwrap_or(latest.started_at),
            })
        })
    } else {
        None
    };
    let flag = if dominant_fingerprint.is_some() {
        "broken"
    } else if consecutive_failures >= degraded_after {
        "degraded"
    } else {
        "healthy"
    };
    WorkflowHealth {
        flag,
        consecutive_failures,
        success_rate_last_20,
        dominant_fingerprint,
    }
}

pub fn snapshots(records: Vec<RunRecord>) -> Vec<serde_json::Value> {
    let mut grouped: HashMap<(String, Option<String>), Vec<RunRecord>> = HashMap::new();
    for row in records {
        grouped
            .entry((row.workflow_id.clone(), row.workflow_hash.clone()))
            .or_default()
            .push(row);
    }
    grouped.into_iter().map(|((workflow_id, workflow_hash), rows)| serde_json::json!({
        "workflow_id": workflow_id, "workflow_hash": workflow_hash,
        "last_run_at": rows.iter().map(|r| r.started_at).max(), "health": compute_health(&rows)
    })).collect()
}

fn env_threshold(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(id: &str, status: &str, fp: Option<&str>, at: i64) -> RunRecord {
        RunRecord {
            run_id: id.into(),
            workflow_id: "wf".into(),
            workflow_hash: Some("hash".into()),
            origin: "cli".into(),
            started_at: at,
            ended_at: at,
            status: status.into(),
            failing_step_index: Some(6),
            error_class: Some("selector_not_found".into()),
            fingerprint: fp.map(str::to_string),
            error_message: String::new(),
            step_count: 1,
            params_digest: String::new(),
            result_ref: String::new(),
        }
    }
    #[test]
    fn classifications_and_health() {
        assert_eq!(
            classify_error("", "selector not found"),
            ErrorClass::SelectorNotFound
        );
        assert_eq!(classify_error("odd", "wat"), ErrorClass::Unknown);
        let rows = vec![
            row("1", "failed", Some("x"), 1),
            row("2", "failed", Some("x"), 2),
            row("3", "failed", Some("x"), 3),
        ];
        assert_eq!(compute_health(&rows).flag, "broken");
        let mut reset = rows;
        reset.push(row("4", "succeeded", None, 4));
        assert_eq!(compute_health(&reset).consecutive_failures, 0);
    }

    #[test]
    fn fingerprint_bytes_match_backend_canon() {
        assert_eq!(
            fingerprint("h", Some(6), ErrorClass::SelectorNotFound),
            "333a6f8f918a2812"
        );
    }
}
