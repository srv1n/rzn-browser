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
    pub tenant_id: String,
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
// Device self view (CLI `fleet status`)
// ---------------------------------------------------------------------------

/// Device self view returned to the CLI `fleet status` command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FleetDeviceSelf {
    pub device_id: String,
    pub name: String,
    pub status: FleetDeviceStatus,
    pub tenant_id: String,
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
            tenant_id: "tnt_1".to_string(),
            poll_interval_seconds: 15,
            server_time_ms: 1_720_000_000_000,
        };
        round_trip(&value);
        assert_snapshot(
            &value,
            json!({
                "device_id": "dev_42",
                "device_token": "fld_secret",
                "tenant_id": "tnt_1",
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
                "max_jobs": 2
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
                "lease_expires_at_ms": 1_720_000_300_000i64
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
            }],
            cancellations: vec!["job_9".to_string()],
            poll_interval_seconds: 15,
            device_status: FleetDeviceStatus::Active,
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
                    "lease_expires_at_ms": 1_720_000_300_000i64
                }],
                "cancellations": ["job_9"],
                "poll_interval_seconds": 15,
                "device_status": "active"
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
                "finished_at_ms": 1_720_000_300_000i64
            }),
        );
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
            tenant_id: "tnt_1".to_string(),
            last_seen_at_ms: Some(1_720_000_000_000),
        };
        round_trip(&value);
        assert_snapshot(
            &value,
            json!({
                "device_id": "dev_42",
                "name": "sarav-mbp",
                "status": "dormant",
                "tenant_id": "tnt_1",
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
}
