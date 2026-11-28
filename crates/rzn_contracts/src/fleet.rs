//! Fleet control-plane wire contracts (`rzn.fleet`).
//!
//! These types are the cross-repo contract between the backend fleet control
//! plane and the laptop supervisor. The backend consumes this crate via a
//! sibling path dependency and codes against exactly these field names, so the
//! shapes here are load-bearing and must stay stable.
//!
//! Design is a pull model: the device polls with a health snapshot, the response
//! carries job assignments, cancellations, and the device's own status. Results
//! post back a full [`crate::workflow::RunResult`]. Enrollment redeems a one-time
//! code for an `fld_`-prefixed device token.
//!
//! Every field that can safely default carries `#[serde(default)]` so old peers
//! tolerate contract additions. `skip_serializing_if` is intentionally omitted so
//! every wire field name is always present on the wire and pinned by the snapshot
//! tests below.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Fleet protocol version negotiated between the control plane and the device.
pub const FLEET_PROTOCOL: &str = "rzn.fleet";

/// Known `code` values carried by [`FleetError`] across all fleet endpoints.
pub mod error_codes {
    /// The device token has been revoked; the device must stop and re-enroll.
    pub const DEVICE_REVOKED: &str = "device_revoked";
    /// The device is dormant and should back off until reactivated.
    pub const DEVICE_DORMANT: &str = "device_dormant";
    /// A result post referenced a job the device does not currently hold a lease for.
    pub const JOB_NOT_CLAIMED_BY_DEVICE: &str = "job_not_claimed_by_device";
    /// The one-time enrollment code was invalid, expired, or already redeemed.
    pub const ENROLLMENT_CODE_INVALID: &str = "enrollment_code_invalid";
}

/// The lane a job runs in when the server names none.
pub const DEFAULT_LANE: &str = "mine";

fn default_lane() -> String {
    DEFAULT_LANE.to_string()
}

// ---------------------------------------------------------------------------
// Enrollment
// ---------------------------------------------------------------------------

/// Request body for `POST /fleet/enroll` (redeem a one-time code for a device token).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FleetEnrollRequest {
    pub code: String,
    pub device_name: String,
    pub platform: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub cli_version: String,
    pub extension_version: String,
}

/// Response body for `POST /fleet/enroll` (issued device identity + poll cadence).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FleetEnrollResponse {
    pub device_id: String,
    pub device_token: String,
    /// The user the device belongs to. It works for that user's tenants (their memberships,
    /// read by the server at each check-in), so no tenant is fixed at enrolment.
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_email: Option<String>,
    #[serde(default)]
    pub poll_interval_seconds: u64,
    #[serde(default)]
    pub server_time_ms: i64,
}

// ---------------------------------------------------------------------------
// Health (reuses supervisor readiness vocabulary)
// ---------------------------------------------------------------------------

/// Device health snapshot carried inside [`FleetPollRequest`] on `POST /fleet/poll`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceHealth {
    #[serde(default)]
    pub browser_running: bool,
    #[serde(default)]
    pub extension_bridge_up: bool,
    #[serde(default)]
    pub readiness_cause: Option<String>,
    pub cli_version: String,
    #[serde(default)]
    pub extension_version: Option<String>,
    #[serde(default)]
    pub uptime_seconds: u64,
    #[serde(default)]
    pub running_job_ids: Vec<String>,
}

// ---------------------------------------------------------------------------
// Poll
// ---------------------------------------------------------------------------

/// Request body for `POST /fleet/poll` (device pulls work and reports health).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FleetPollRequest {
    pub health: DeviceHealth,
    #[serde(default)]
    pub active_job_ids: Vec<String>,
    #[serde(default)]
    pub max_jobs: u32,
    /// Hash of the catalog the device holds; `None` when it holds none.
    #[serde(default)]
    pub catalog_hash: Option<String>,
    /// Sites (`system`) whose login wall the user reports cleared since the last check-in.
    #[serde(default)]
    pub sites_cleared: Vec<String>,
}

/// A single job the control plane assigns to a device in a `POST /fleet/poll` response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FleetJobAssignment {
    pub job_id: String,
    pub workflow_id: String,
    pub workflow_hash: String,
    pub params: Value,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub single_delivery: bool,
    #[serde(default)]
    pub execution_deadline_seconds: u64,
    #[serde(default)]
    pub lease_expires_at_ms: i64,
    /// Whose job it is: `mine`, `team` or `pool`.
    #[serde(default = "default_lane")]
    pub lane: String,
    /// The workflow version's `result.contract`, so the device can check its output before
    /// posting; `None` when the manifest names none.
    #[serde(default)]
    pub contract: Option<String>,
}

/// Response body for `POST /fleet/poll` (assignments, cancellations, device status).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FleetPollResponse {
    #[serde(default)]
    pub jobs: Vec<FleetJobAssignment>,
    #[serde(default)]
    pub cancellations: Vec<String>,
    #[serde(default)]
    pub poll_interval_seconds: u64,
    pub device_status: FleetDeviceStatus,
    /// True when the device's `catalog_hash` differs from the server's: fetch the catalog.
    #[serde(default)]
    pub catalog_changed: bool,
    /// Sites paused on this device; the device takes no local run for them until `until_ms`.
    #[serde(default)]
    pub sites_paused: Vec<FleetSitePause>,
}

/// A site paused on one device, carried in [`FleetPollResponse::sites_paused`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FleetSitePause {
    pub system: String,
    /// End of the pause; `None` for a login wall, which lasts until the device reports it
    /// cleared in `sites_cleared`.
    #[serde(default)]
    pub until_ms: Option<i64>,
    /// `site_challenge` or `auth_wall`.
    pub reason: String,
}

/// Lifecycle status of a device, carried in poll responses and self views.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FleetDeviceStatus {
    Active,
    Dormant,
    Revoked,
}

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

/// Response body for `GET /fleet/catalog`: the tenant's workflows, each at its current version.
/// The server sends more (each workflow's params, contract, side effects, and the families);
/// the device reads only what it syncs by.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FleetCatalog {
    pub catalog_hash: String,
    #[serde(default)]
    pub workflows: Vec<FleetCatalogEntry>,
}

/// One workflow of [`FleetCatalog`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FleetCatalogEntry {
    pub workflow_id: String,
    pub content_hash: String,
    /// `active`, `paused` or `retired`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_reason: Option<String>,
    #[serde(default)]
    pub system: String,
}

// ---------------------------------------------------------------------------
// Workflow fetch
// ---------------------------------------------------------------------------

/// Response body for `GET /fleet/workflows/{id}` (workflow manifest by content hash).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FleetWorkflowFetchResponse {
    pub workflow_id: String,
    pub content_hash: String,
    pub manifest: Value,
}

// ---------------------------------------------------------------------------
// Result post
// ---------------------------------------------------------------------------

/// Request body for `POST /fleet/results` (device posts a terminal run result).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FleetResultPost {
    pub job_id: String,
    pub status: FleetJobTerminalStatus,
    pub run_result: crate::workflow::RunResult,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub started_at_ms: i64,
    #[serde(default)]
    pub finished_at_ms: i64,
    /// The lease of the claim this result is for ([`FleetJobAssignment::lease_expires_at_ms`]).
    /// The server refuses a post whose lease is not the job's current one, so a late result of
    /// an expired claim never counts for the next attempt. `None` (an older device) matches by
    /// job id alone.
    #[serde(default)]
    pub lease_expires_at_ms: Option<i64>,
}

/// Terminal outcome of a fleet job, carried in `POST /fleet/results`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FleetJobTerminalStatus {
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    Aborted,
}

/// Response body for `POST /fleet/results` (server acknowledgement + dedupe signal).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FleetResultAck {
    #[serde(default)]
    pub ok: bool,
    pub job_status: String,
    #[serde(default)]
    pub deduped: bool,
}

// ---------------------------------------------------------------------------
// Repair candidates
// ---------------------------------------------------------------------------

/// Request body for `POST /fleet/candidates`: a local fix of a catalog workflow, proposed as a
/// repair candidate (`rzn-browser fleet propose <id>`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FleetRepairCandidate {
    /// The catalog id (the manifest's `id`).
    pub workflow_id: String,
    /// The fork's whole manifest.
    pub manifest: Value,
    /// Content hash of the version the fork was made from (the fleet or builtin copy it
    /// shadows); `None` when nothing is shadowed.
    #[serde(default)]
    pub base_hash: Option<String>,
    /// What broke and what changed.
    pub reason: String,
    /// The last local run of the fork, if any.
    #[serde(default)]
    pub last_run: Option<crate::workflow::RunResult>,
}

/// Response body for `POST /fleet/candidates`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FleetRepairCandidateAck {
    pub candidate_id: String,
    pub content_hash: String,
    /// The same manifest was already a candidate for this workflow.
    #[serde(default)]
    pub deduped: bool,
}

/// What changed between two manifests, one line per changed JSON path:
/// `~ path: before -> after`, `+ path: value`, `- path: value`. Values are compact JSON cut at
/// 160 characters. Both the server (candidate review) and the device (fork listing) use it.
pub fn manifest_diff(before: &Value, after: &Value) -> Vec<String> {
    let mut out = Vec::new();
    diff_at("", before, after, &mut out);
    out
}

fn diff_at(path: &str, before: &Value, after: &Value, out: &mut Vec<String>) {
    let child = |key: &str| {
        if path.is_empty() {
            key.to_string()
        } else {
            format!("{path}.{key}")
        }
    };
    match (before, after) {
        (Value::Object(a), Value::Object(b)) => {
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => diff_at(&child(key), x, y, out),
                    (Some(x), None) => out.push(format!("- {}: {}", child(key), short(x))),
                    (None, Some(y)) => out.push(format!("+ {}: {}", child(key), short(y))),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            for i in 0..a.len().max(b.len()) {
                let at = format!("{path}[{i}]");
                match (a.get(i), b.get(i)) {
                    (Some(x), Some(y)) => diff_at(&at, x, y, out),
                    (Some(x), None) => out.push(format!("- {at}: {}", short(x))),
                    (None, Some(y)) => out.push(format!("+ {at}: {}", short(y))),
                    (None, None) => {}
                }
            }
        }
        (a, b) if a != b => out.push(format!("~ {path}: {} -> {}", short(a), short(b))),
        _ => {}
    }
}

fn short(value: &Value) -> String {
    let text = value.to_string();
    match text.char_indices().nth(160) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    }
}

// ---------------------------------------------------------------------------
// Device self view (CLI `fleet status`)
// ---------------------------------------------------------------------------

/// Device self view returned to the CLI `fleet status` command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FleetDeviceSelf {
    pub device_id: String,
    pub name: String,
    pub status: FleetDeviceStatus,
    /// The user the device belongs to.
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_email: Option<String>,
    /// The tenants it works for now: its user's memberships, oldest first.
    #[serde(default)]
    pub tenant_ids: Vec<String>,
    #[serde(default)]
    pub last_seen_at_ms: Option<i64>,
}

// ---------------------------------------------------------------------------
// Error envelope
// ---------------------------------------------------------------------------

/// Error envelope shared by every fleet endpoint. See [`error_codes`] for known `code`s.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FleetError {
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{RunResult, RunStatus, RUN_RESULT_CONTRACT};
    use serde::de::DeserializeOwned;
    use serde_json::json;

    fn round_trip<T>(value: &T)
    where
        T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let text = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(value, &back, "round-trip mismatch");
    }

    fn assert_snapshot<T: Serialize>(value: &T, expected: Value) {
        assert_eq!(
            serde_json::to_value(value).expect("to_value"),
            expected,
            "wire field-name snapshot drift"
        );
    }

    fn sample_run_result() -> RunResult {
        RunResult {
            version: RUN_RESULT_CONTRACT.to_string(),
            run_id: "run_1".to_string(),
            workflow_id: "wf_1".to_string(),
            status: RunStatus::Succeeded,
            output: None,
            artifacts: Vec::new(),
            warnings: Vec::new(),
            steps: Vec::new(),
            debug: None,
            error: None,
            failure_summary: None,
        }
    }

    #[test]
    fn protocol_version_pinned() {
        assert_eq!(FLEET_PROTOCOL, "rzn.fleet");
    }

    #[test]
    fn error_codes_pinned() {
        assert_eq!(error_codes::DEVICE_REVOKED, "device_revoked");
        assert_eq!(error_codes::DEVICE_DORMANT, "device_dormant");
        assert_eq!(
            error_codes::JOB_NOT_CLAIMED_BY_DEVICE,
            "job_not_claimed_by_device"
        );
        assert_eq!(
            error_codes::ENROLLMENT_CODE_INVALID,
            "enrollment_code_invalid"
        );
    }

    #[test]
    fn enroll_request_round_trip_and_snapshot() {
        let value = FleetEnrollRequest {
            code: "otc_abc123".to_string(),
            device_name: "sarav-mbp".to_string(),
            platform: "darwin".to_string(),
            capabilities: vec!["extension_actor".to_string(), "cdp".to_string()],
            cli_version: "0.4.1".to_string(),
            extension_version: "1.2.0".to_string(),
        };
        round_trip(&value);
        assert_snapshot(
            &value,
            json!({
                "code": "otc_abc123",
                "device_name": "sarav-mbp",
                "platform": "darwin",
                "capabilities": ["extension_actor", "cdp"],
                "cli_version": "0.4.1",
                "extension_version": "1.2.0"
            }),
        );
    }

    #[test]
    fn enroll_response_round_trip_and_snapshot() {
        let value = FleetEnrollResponse {
            device_id: "dev_42".to_string(),
            device_token: "fld_secret".to_string(),
            user_id: "usr_1".to_string(),
            user_email: Some("me@example.com".to_string()),
            poll_interval_seconds: 15,
            server_time_ms: 1_720_000_000_000,
        };
        round_trip(&value);
        assert_snapshot(
            &value,
            json!({
                "device_id": "dev_42",
                "device_token": "fld_secret",
                "user_id": "usr_1",
                "user_email": "me@example.com",
                "poll_interval_seconds": 15,
                "server_time_ms": 1_720_000_000_000i64
            }),
        );
    }

    #[test]
    fn device_health_round_trip_and_snapshot() {
        let value = DeviceHealth {
            browser_running: true,
            extension_bridge_up: true,
            readiness_cause: Some("bridge_up".to_string()),
            cli_version: "0.4.1".to_string(),
            extension_version: Some("1.2.0".to_string()),
            uptime_seconds: 3600,
            running_job_ids: vec!["job_1".to_string()],
        };
        round_trip(&value);
        assert_snapshot(
            &value,
            json!({
                "browser_running": true,
                "extension_bridge_up": true,
                "readiness_cause": "bridge_up",
                "cli_version": "0.4.1",
                "extension_version": "1.2.0",
                "uptime_seconds": 3600,
                "running_job_ids": ["job_1"]
            }),
        );
    }

    #[test]
    fn poll_request_round_trip_and_snapshot() {
        let value = FleetPollRequest {
            health: DeviceHealth {
                browser_running: true,
                extension_bridge_up: false,
                readiness_cause: None,
                cli_version: "0.4.1".to_string(),
                extension_version: None,
                uptime_seconds: 10,
                running_job_ids: Vec::new(),
            },
            active_job_ids: vec!["job_1".to_string(), "job_2".to_string()],
            max_jobs: 2,
            catalog_hash: Some("abc".to_string()),
            sites_cleared: vec!["x".to_string()],
        };
        round_trip(&value);
        assert_snapshot(
            &value,
            json!({
                "health": {
                    "browser_running": true,
                    "extension_bridge_up": false,
                    "readiness_cause": null,
                    "cli_version": "0.4.1",
                    "extension_version": null,
                    "uptime_seconds": 10,
                    "running_job_ids": []
                },
                "active_job_ids": ["job_1", "job_2"],
                "max_jobs": 2,
                "catalog_hash": "abc",
                "sites_cleared": ["x"]
            }),
        );
    }

    #[test]
    fn job_assignment_round_trip_and_snapshot() {
        let value = FleetJobAssignment {
            job_id: "job_1".to_string(),
            workflow_id: "wf_1".to_string(),
            workflow_hash: "sha256:deadbeef".to_string(),
            params: json!({ "query": "shoes" }),
            priority: 5,
            single_delivery: true,
            execution_deadline_seconds: 300,
            lease_expires_at_ms: 1_720_000_300_000,
            lane: "mine".to_string(),
            contract: Some("rzn.items/v2".to_string()),
        };
        round_trip(&value);
        assert_snapshot(
            &value,
            json!({
                "job_id": "job_1",
                "workflow_id": "wf_1",
                "workflow_hash": "sha256:deadbeef",
                "params": { "query": "shoes" },
                "priority": 5,
                "single_delivery": true,
                "execution_deadline_seconds": 300,
                "lease_expires_at_ms": 1_720_000_300_000i64,
                "lane": "mine",
                "contract": "rzn.items/v2"
            }),
        );
    }

    #[test]
    fn poll_response_round_trip_and_snapshot() {
        let value = FleetPollResponse {
            jobs: vec![FleetJobAssignment {
                job_id: "job_1".to_string(),
                workflow_id: "wf_1".to_string(),
                workflow_hash: "sha256:deadbeef".to_string(),
                params: json!({}),
                priority: 0,
                single_delivery: false,
                execution_deadline_seconds: 120,
                lease_expires_at_ms: 1_720_000_300_000,
                lane: "mine".to_string(),
                contract: None,
            }],
            cancellations: vec!["job_9".to_string()],
            poll_interval_seconds: 15,
            device_status: FleetDeviceStatus::Active,
            catalog_changed: true,
            sites_paused: vec![FleetSitePause {
                system: "x".to_string(),
                until_ms: None,
                reason: "auth_wall".to_string(),
            }],
        };
        round_trip(&value);
        assert_snapshot(
            &value,
            json!({
                "jobs": [{
                    "job_id": "job_1",
                    "workflow_id": "wf_1",
                    "workflow_hash": "sha256:deadbeef",
                    "params": {},
                    "priority": 0,
                    "single_delivery": false,
                    "execution_deadline_seconds": 120,
                    "lease_expires_at_ms": 1_720_000_300_000i64,
                    "lane": "mine",
                    "contract": null
                }],
                "cancellations": ["job_9"],
                "poll_interval_seconds": 15,
                "device_status": "active",
                "catalog_changed": true,
                "sites_paused": [{"system": "x", "until_ms": null, "reason": "auth_wall"}]
            }),
        );
    }

    #[test]
    fn device_status_enum_casing() {
        for (variant, wire) in [
            (FleetDeviceStatus::Active, "active"),
            (FleetDeviceStatus::Dormant, "dormant"),
            (FleetDeviceStatus::Revoked, "revoked"),
        ] {
            round_trip(&variant);
            assert_eq!(serde_json::to_value(&variant).unwrap(), json!(wire));
        }
    }

    #[test]
    fn workflow_fetch_response_round_trip_and_snapshot() {
        let value = FleetWorkflowFetchResponse {
            workflow_id: "wf_1".to_string(),
            content_hash: "sha256:cafef00d".to_string(),
            manifest: json!({ "schema_version": "rzn.workflow_manifest", "id": "wf_1" }),
        };
        round_trip(&value);
        assert_snapshot(
            &value,
            json!({
                "workflow_id": "wf_1",
                "content_hash": "sha256:cafef00d",
                "manifest": { "schema_version": "rzn.workflow_manifest", "id": "wf_1" }
            }),
        );
    }

    #[test]
    fn result_post_round_trip_and_snapshot() {
        let value = FleetResultPost {
            job_id: "job_1".to_string(),
            status: FleetJobTerminalStatus::TimedOut,
            run_result: sample_run_result(),
            error: Some("deadline exceeded".to_string()),
            started_at_ms: 1_720_000_000_000,
            finished_at_ms: 1_720_000_300_000,
            lease_expires_at_ms: Some(1_720_000_300_000),
        };
        round_trip(&value);
        assert_snapshot(
            &value,
            json!({
                "job_id": "job_1",
                "status": "timed_out",
                "run_result": {
                    "version": "rzn.run_result",
                    "run_id": "run_1",
                    "workflow_id": "wf_1",
                    "status": "succeeded"
                },
                "error": "deadline exceeded",
                "started_at_ms": 1_720_000_000_000i64,
                "finished_at_ms": 1_720_000_300_000i64,
                "lease_expires_at_ms": 1_720_000_300_000i64
            }),
        );
        // An older device sends no lease.
        let old: FleetResultPost = serde_json::from_value(json!({
            "job_id": "job_1", "status": "failed",
            "run_result": {"version": "rzn.run_result", "run_id": "r", "workflow_id": "w",
                           "status": "failed"}
        }))
        .unwrap();
        assert_eq!(old.lease_expires_at_ms, None);
    }

    #[test]
    fn repair_candidate_round_trip_and_manifest_diff() {
        let value = FleetRepairCandidate {
            workflow_id: "x/screenshot".to_string(),
            manifest: json!({"id": "x/screenshot", "steps": [{"selector": "a"}]}),
            base_hash: Some("abc".to_string()),
            reason: "selector moved".to_string(),
            last_run: Some(sample_run_result()),
        };
        round_trip(&value);
        let minimal: FleetRepairCandidate = serde_json::from_value(json!({
            "workflow_id": "x/a", "manifest": {}, "reason": "r"
        }))
        .unwrap();
        assert_eq!((minimal.base_hash, minimal.last_run), (None, None));
        round_trip(&FleetRepairCandidateAck {
            candidate_id: "c1".to_string(),
            content_hash: "h".to_string(),
            deduped: true,
        });

        let before = json!({"id": "x/a", "version": "1", "steps": [{"selector": "a"}, {"t": 1}],
                            "old": true});
        let after = json!({"id": "x/a", "version": "2", "steps": [{"selector": "b"}],
                           "new": "y"});
        assert_eq!(
            manifest_diff(&before, &after),
            [
                "+ new: \"y\"",
                "- old: true",
                "~ steps[0].selector: \"a\" -> \"b\"",
                "- steps[1]: {\"t\":1}",
                "~ version: \"1\" -> \"2\"",
            ]
        );
        assert!(manifest_diff(&before, &before).is_empty());
    }

    #[test]
    fn terminal_status_enum_casing() {
        for (variant, wire) in [
            (FleetJobTerminalStatus::Succeeded, "succeeded"),
            (FleetJobTerminalStatus::Failed, "failed"),
            (FleetJobTerminalStatus::TimedOut, "timed_out"),
            (FleetJobTerminalStatus::Cancelled, "cancelled"),
            (FleetJobTerminalStatus::Aborted, "aborted"),
        ] {
            round_trip(&variant);
            assert_eq!(serde_json::to_value(&variant).unwrap(), json!(wire));
        }
    }

    #[test]
    fn result_ack_round_trip_and_snapshot() {
        let value = FleetResultAck {
            ok: true,
            job_status: "succeeded".to_string(),
            deduped: false,
        };
        round_trip(&value);
        assert_snapshot(
            &value,
            json!({
                "ok": true,
                "job_status": "succeeded",
                "deduped": false
            }),
        );
    }

    #[test]
    fn device_self_round_trip_and_snapshot() {
        let value = FleetDeviceSelf {
            device_id: "dev_42".to_string(),
            name: "sarav-mbp".to_string(),
            status: FleetDeviceStatus::Dormant,
            user_id: "usr_1".to_string(),
            user_email: None,
            tenant_ids: vec!["tnt_1".to_string(), "tnt_2".to_string()],
            last_seen_at_ms: Some(1_720_000_000_000),
        };
        round_trip(&value);
        assert_snapshot(
            &value,
            json!({
                "device_id": "dev_42",
                "name": "sarav-mbp",
                "status": "dormant",
                "user_id": "usr_1",
                "tenant_ids": ["tnt_1", "tnt_2"],
                "last_seen_at_ms": 1_720_000_000_000i64
            }),
        );
    }

    #[test]
    fn error_round_trip_and_snapshot() {
        let value = FleetError {
            code: error_codes::DEVICE_REVOKED.to_string(),
            message: "device token revoked".to_string(),
        };
        round_trip(&value);
        assert_snapshot(
            &value,
            json!({
                "code": "device_revoked",
                "message": "device token revoked"
            }),
        );
    }

    #[test]
    fn forward_compat_defaults_tolerate_missing_fields() {
        // Old peer omits every optional/defaulting field on a health snapshot.
        let health: DeviceHealth =
            serde_json::from_value(json!({ "cli_version": "0.4.1" })).expect("defaults apply");
        assert_eq!(
            health,
            DeviceHealth {
                browser_running: false,
                extension_bridge_up: false,
                readiness_cause: None,
                cli_version: "0.4.1".to_string(),
                extension_version: None,
                uptime_seconds: 0,
                running_job_ids: Vec::new(),
            }
        );
    }

    #[test]
    fn old_peers_omit_the_check_in_additions() {
        let req: FleetPollRequest =
            serde_json::from_value(json!({ "health": { "cli_version": "0.4.1" } })).unwrap();
        assert_eq!(req.catalog_hash, None);
        assert!(req.sites_cleared.is_empty());
        let job: FleetJobAssignment = serde_json::from_value(json!({
            "job_id": "j", "workflow_id": "w", "workflow_hash": "h", "params": {}
        }))
        .unwrap();
        assert_eq!(job.lane, "mine");
        assert_eq!(job.contract, None);
        let resp: FleetPollResponse =
            serde_json::from_value(json!({ "device_status": "active" })).unwrap();
        assert!(!resp.catalog_changed);
        assert!(resp.sites_paused.is_empty());
    }

    #[test]
    fn catalog_reads_the_server_body_and_ignores_what_it_does_not_sync_by() {
        let catalog: FleetCatalog = serde_json::from_value(json!({
            "catalog_hash": "h",
            "workflows": [{"workflow_id": "x/profile_posts", "content_hash": "c", "status": "paused",
                           "paused_reason": "broken", "system": "x", "contract": "rzn.items/v2",
                           "params": null, "side_effect_classes": []}],
            "families": [{"connector": "x-browser", "describe": {}}]
        }))
        .unwrap();
        assert_eq!(catalog.catalog_hash, "h");
        assert_eq!(catalog.workflows[0].workflow_id, "x/profile_posts");
        assert_eq!(
            catalog.workflows[0].paused_reason.as_deref(),
            Some("broken")
        );
    }
}
