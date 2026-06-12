use anyhow::{anyhow, Context, Result};
use interprocess::local_socket::{
    tokio::Stream as LocalSocketStream,
    traits::tokio::{Listener as _, Stream as _},
    GenericFilePath, ListenerOptions, ToFsName,
};
use rzn_contracts::v1::{
    ActionResultV1, CapabilitiesV1, CloudBrowserCommandV1, CloudCommandEnvelopeV1,
    CloudCommandResultV1, CLOUD_CONTRACT_VERSION,
};
use rzn_contracts::v2::{
    format_tab_ref, parse_tab_ref, ArtifactKindV1, ArtifactV1, DebugBundleV1, DebugEventV1,
    RunErrorV1, RunResultV2, RunStatusV2, RunWarningV1, SideEffectClassV2, RUN_RESULT_VERSION,
};
use rzn_core::framing::{read_frame, read_required_frame, write_frame};
use rzn_core::runtime_paths::{
    default_app_base_dir, env_trimmed, infer_current_app_base, supervisor_paths_for_app_base,
    APP_BASE_ENV_KEYS,
};
use rzn_core::secure_files::{set_secret_file_permissions, write_secret_file};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::{timeout, Duration};
use uuid::Uuid;

use crate::supervisor_cloud::{self, CloudDispatchRequest, SupervisorCloudActor};

pub(crate) const RZN_LOCAL_PROTOCOL_VERSION: &str = "rzn.local.v1";
const SUPERVISOR_LOCK_FILENAME: &str = "rzn-supervisor.lock";
const HANDSHAKE_TIMEOUT_MS: u64 = 2_000;
const REQUEST_TIMEOUT_MS: u64 = 30_000;
const REQUEST_TIMEOUT_GRACE_MS: u64 = 5_000;
const MAX_CALLER_TIMEOUT_MS: u64 = 10 * 60 * 1_000;
const EXTENSION_BRIDGE_TIMEOUT_MS: u64 = 20_000;
const EXTENSION_BRIDGE_GRACE_MS: u64 = 5_000;
const DEFAULT_BRIDGE_WAIT_MS: u64 = 2_500;
const DEFAULT_BRIDGE_PROBE_TIMEOUT_MS: u64 = 1_500;
const BRIDGE_PROBE_TIMEOUT_GRACE_MS: u64 = 500;
const BRIDGE_RECONNECT_BEFORE_CALL_WAIT_MS: u64 = 20_000;
const HEAL_INITIAL_BRIDGE_WAIT_MS: u64 = 500;
const HEAL_BRIDGE_WAIT_MS: u64 = 45_000;
const HEAL_POST_PROBE_RECONNECT_WAIT_MS: u64 = 10_000;
const HEAL_READINESS_PROBE_COUNT: u64 = 3;
const HEAL_STABILITY_DELAY_MS: u64 = 1_500;
const HEAL_STABILITY_BRIDGE_WAIT_MS: u64 = 2_500;
const TOOL_DISPATCH_RECOVERY_WAIT_MS: u64 = 2_500;
const REQUIRED_EXTENSION_KEEPALIVE_CAPABILITY: &str = "content_keepalive_port";
const EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION: u64 = 8;
const REQUIRED_EXTENSION_BRIDGE_CAPABILITIES: &[&str] = &[
    "content_keepalive_port",
    "native_host_stdout_heartbeat",
    "native_roundtrip_ping_health",
    "native_port_epoch_fencing",
    "workflow_session_epoch_fencing",
    "broker_watchdog",
    "request_lease_cancellation",
    "watchdog_queue_unblock",
    "epoch_chain_identity",
    "native_control_epoch_fencing",
    "supervisor_bridge_response_fencing",
    "health_beacon_v2",
    "auxiliary_path_lease_guards",
    "port_scoped_disconnect_suppression",
    "native_message_frame_cap",
];
const READINESS_CAUSE_BRIDGE_DOWN: &str = "bridge_down";
const READINESS_CAUSE_SERVICE_WORKER_UNRESPONSIVE: &str = "service_worker_unresponsive";
const READINESS_CAUSE_STALE_EXTENSION_BUNDLE: &str = "stale_extension_bundle";
const READINESS_CAUSE_BROWSER_TARGET_UNRESOLVED: &str = "browser_target_unresolved";
const READINESS_CAUSE_BROWSER_TARGET_MISMATCH: &str = "browser_target_mismatch";
const READINESS_CAUSE_TRANSPORT_TIMEOUT: &str = "transport_timeout";
const READINESS_CAUSE_ZOMBIE_NATIVE_HOST: &str = "zombie_native_host";
const NATIVE_HOST_SHUTDOWN_METHOD: &str = "native_host.shutdown";
const STRICT_BRIDGE_IDENTITY_ENV: &str = "RZN_SUPERVISOR_STRICT_BRIDGE_IDENTITY";

#[derive(Clone, Debug)]
pub(crate) struct SupervisorConfig {
    pub app_base: Option<PathBuf>,
}

impl SupervisorConfig {
    pub(crate) fn app_base_dir(&self) -> PathBuf {
        self.app_base
            .clone()
            .or_else(|| {
                APP_BASE_ENV_KEYS
                    .iter()
                    .find_map(|key| env_trimmed(key))
                    .map(PathBuf::from)
            })
            .or_else(infer_current_app_base)
            .unwrap_or_else(default_app_base_dir)
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SupervisorPaths {
    pub app_base: PathBuf,
    pub secure_dir: PathBuf,
    pub run_dir: PathBuf,
    pub socket_path: PathBuf,
    pub token_path: PathBuf,
    pub lock_path: PathBuf,
}

impl SupervisorPaths {
    pub(crate) fn for_config(config: &SupervisorConfig) -> Self {
        let app_base = config.app_base_dir();
        let (socket_path, token_path) = supervisor_paths_for_app_base(&app_base);
        let secure_dir = app_base.join("secure");
        let run_dir = app_base.join("run");
        Self {
            app_base,
            socket_path,
            token_path,
            lock_path: run_dir.join(SUPERVISOR_LOCK_FILENAME),
            secure_dir,
            run_dir,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct SupervisorServeReport {
    pub ok: bool,
    pub protocol: &'static str,
    pub pid: u32,
    pub app_base: String,
    pub socket_path: String,
    pub token_path: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SupervisorClientStatus {
    pub ok: bool,
    pub result: Value,
}

struct SupervisorState {
    config: SupervisorConfig,
    paths: SupervisorPaths,
    supervisor_boot_id: String,
    bridge_epoch_counter: AtomicU64,
    native_bridges: Mutex<HashMap<String, NativeHostBridge>>,
    native_bridge_pending: Mutex<HashMap<String, PendingNativeCall>>,
    native_bridge_health: Mutex<HashMap<String, NativeBridgeHealth>>,
    last_registered_bridge_id: Mutex<Option<String>>,
    cloud_actor: Option<SupervisorCloudActor>,
    sessions: Mutex<HashMap<String, BrowserSessionRecord>>,
    shutdown: AtomicBool,
}

#[allow(dead_code)]
#[derive(Clone)]
struct NativeHostBridge {
    id: String,
    epoch: u64,
    tx: mpsc::UnboundedSender<Vec<u8>>,
    registered_at_ms: u64,
    metadata: NativeHostBridgeMetadata,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
struct NativeHostBridgeMetadata {
    native_host_pid: Option<u64>,
    native_host_boot_id: Option<String>,
    caller_origin: Option<String>,
    caller_extension_id: Option<String>,
    caller_origin_status: String,
    extension_reported_origin: Option<String>,
    extension_reported_id: Option<String>,
    browser_instance_id: Option<String>,
    extension_target: Option<String>,
    extension_manifest_version: Option<u64>,
    extension_target_hint: Option<String>,
    extension_build_signature: Option<Value>,
    bridge_contract_version: Option<u64>,
    extension_worker_boot_id: Option<String>,
    browser_diagnostics: Option<Value>,
    identity_match: Option<bool>,
    identity_status: String,
    launch: NativeHostBridgeLaunchMetadata,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
struct NativeHostBridgeLaunchMetadata {
    parent_window: Option<String>,
    has_socket_override: bool,
    has_token_override: bool,
    extra_arg_count: Option<u64>,
}

impl NativeHostBridgeMetadata {
    fn missing() -> Self {
        Self {
            caller_origin_status: "missing".to_string(),
            identity_status: "missing".to_string(),
            ..Self::default()
        }
    }

    fn from_hello_params(params: &Value, strict_identity: bool) -> Result<Self> {
        let mut metadata = Self {
            native_host_pid: params.get("native_host_pid").and_then(Value::as_u64),
            native_host_boot_id: params
                .get("native_host_boot_id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(ToString::to_string),
            launch: NativeHostBridgeLaunchMetadata::from_hello_params(params.get("launch")),
            ..Self::missing()
        };

        if let Some(origin) = params.get("caller_origin").and_then(Value::as_str) {
            match normalize_bridge_caller_origin(origin) {
                Ok((origin, extension_id)) => {
                    metadata.caller_origin = Some(origin);
                    metadata.caller_extension_id = Some(extension_id);
                    metadata.caller_origin_status = "present".to_string();
                    metadata.identity_status = "verified_launch_origin".to_string();
                }
                Err(reason) if strict_identity => {
                    return Err(anyhow!(
                        "invalid native-host caller_origin `{}`: {}",
                        origin,
                        reason
                    ));
                }
                Err(_) => {
                    metadata.caller_origin_status = "invalid_origin".to_string();
                    metadata.identity_status = "invalid_origin".to_string();
                }
            }
        }

        Ok(metadata)
    }

    fn update_extension_reported_identity(&mut self, ping_response: &Value) {
        let reported_id = ping_response
            .pointer("/result/extension_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
        let reported_origin = ping_response
            .pointer("/result/extension_origin")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .or_else(|| {
                reported_id
                    .as_ref()
                    .map(|id| format!("chrome-extension://{id}/"))
            });

        self.extension_reported_id = reported_id;
        self.extension_reported_origin = reported_origin.clone();

        match (&self.caller_origin, reported_origin.as_deref()) {
            (Some(launch_origin), Some(reported_origin)) => {
                match normalize_bridge_caller_origin(reported_origin) {
                    Ok((normalized_reported_origin, reported_id)) => {
                        self.extension_reported_origin = Some(normalized_reported_origin.clone());
                        self.extension_reported_id = Some(reported_id);
                        let matches = normalized_reported_origin == *launch_origin;
                        self.identity_match = Some(matches);
                        self.identity_status = if matches {
                            "matched".to_string()
                        } else {
                            "mismatched".to_string()
                        };
                    }
                    Err(_) => {
                        self.identity_match = Some(false);
                        self.identity_status = "reported_invalid".to_string();
                    }
                }
            }
            (Some(_), None) => {
                self.identity_match = None;
                self.identity_status = "verified_launch_origin".to_string();
            }
            (None, Some(reported_origin)) => {
                match normalize_bridge_caller_origin(reported_origin) {
                    Ok((normalized_reported_origin, reported_id)) => {
                        self.extension_reported_origin = Some(normalized_reported_origin);
                        self.extension_reported_id = Some(reported_id);
                        self.identity_match = None;
                        self.identity_status = "reported_only".to_string();
                    }
                    Err(_) => {
                        self.identity_match = Some(false);
                        self.identity_status = "reported_invalid".to_string();
                    }
                }
            }
            (None, None) => {
                self.identity_match = None;
                if self.identity_status.is_empty() {
                    self.identity_status = "missing".to_string();
                }
            }
        }
    }

    fn update_extension_reported_metadata(&mut self, ping_response: &Value) {
        self.update_extension_reported_identity(ping_response);
        self.browser_instance_id = ping_response
            .pointer("/result/browser_instance_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
        self.extension_target = ping_response
            .pointer("/result/extension_target")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
        self.extension_manifest_version = ping_response
            .pointer("/result/extension_manifest_version")
            .and_then(Value::as_u64);
        self.extension_target_hint = ping_response
            .pointer("/result/extension_target_hint")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
        self.extension_build_signature = ping_response
            .pointer("/result/extension_build_signature")
            .filter(|value| !value.is_null())
            .cloned();
        self.bridge_contract_version = ping_response
            .pointer("/result/bridge_contract_version")
            .and_then(Value::as_u64);
        self.extension_worker_boot_id = ping_response
            .pointer("/result/extension_worker_boot_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
        self.browser_diagnostics = ping_response
            .pointer("/result/browser_diagnostics")
            .filter(|value| !value.is_null())
            .cloned();
    }
}

impl NativeHostBridgeLaunchMetadata {
    fn from_hello_params(value: Option<&Value>) -> Self {
        let Some(value) = value else {
            return Self::default();
        };
        Self {
            parent_window: value
                .get("parent_window")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            has_socket_override: value
                .get("has_socket_override")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            has_token_override: value
                .get("has_token_override")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            extra_arg_count: value.get("extra_arg_count").and_then(Value::as_u64),
        }
    }
}

fn normalize_bridge_caller_origin(value: &str) -> std::result::Result<(String, String), String> {
    let value = value.trim();
    let Some(extension_id) = value.strip_prefix("chrome-extension://") else {
        return Err("origin must use chrome-extension:// scheme".to_string());
    };
    let Some(extension_id) = extension_id.strip_suffix('/') else {
        return Err("origin must end with a trailing slash".to_string());
    };
    if extension_id.len() != 32 || !extension_id.bytes().all(|b| (b'a'..=b'p').contains(&b)) {
        return Err("extension ID must be 32 lowercase characters from a through p".to_string());
    }
    Ok((
        format!("chrome-extension://{extension_id}/"),
        extension_id.to_string(),
    ))
}

fn strict_bridge_identity_enabled() -> bool {
    matches!(
        std::env::var(STRICT_BRIDGE_IDENTITY_ENV)
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

struct PendingNativeCall {
    bridge_id: String,
    bridge_epoch: u64,
    method: String,
    deadline_at_ms: u64,
    responder: oneshot::Sender<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BridgeTarget {
    Default,
    BridgeId(String),
    BrowserInstanceId(String),
    BrowserKind(String),
    SessionId(String),
    Preferred(Box<BridgeTarget>),
}

impl BridgeTarget {
    fn from_params(params: &Value) -> Self {
        if let Some(explicit) = Self::explicit_from_params(params) {
            return explicit;
        }
        let target = params.get("browser_target").unwrap_or(params);
        if let Some(preferred) = target
            .get("preferred")
            .or_else(|| target.get("preferred_browser_target"))
            .and_then(Self::explicit_from_params)
        {
            return Self::Preferred(Box::new(preferred));
        }
        if let Some(session_id) = session_id_from_params(params) {
            return Self::SessionId(session_id);
        }
        Self::Default
    }

    fn explicit_from_params(params: &Value) -> Option<Self> {
        let target = params.get("browser_target").unwrap_or(params);
        if let Some(bridge_id) = params
            .get("bridge_id")
            .or_else(|| params.get("supervisor_bridge_id"))
            .or_else(|| target.get("bridge_id"))
            .or_else(|| target.get("supervisor_bridge_id"))
            .or_else(|| target.get("bridge"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(Self::BridgeId(bridge_id.to_string()));
        }
        if let Some(browser_instance_id) = params
            .get("browser_instance_id")
            .or_else(|| params.get("browserInstanceId"))
            .or_else(|| target.get("browser_instance_id"))
            .or_else(|| target.get("browserInstanceId"))
            .or_else(|| target.get("browser_instance"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(Self::BrowserInstanceId(browser_instance_id.to_string()));
        }
        if let Some(browser) = params
            .get("browser")
            .or_else(|| params.get("browser_kind"))
            .or_else(|| params.get("browserTarget"))
            .or_else(|| target.get("browser"))
            .or_else(|| target.get("browser_kind"))
            .or_else(|| target.get("browserTarget"))
            .and_then(Value::as_str)
            .and_then(normalize_browser_target_slug)
        {
            return Some(Self::BrowserKind(browser));
        }
        None
    }
}

fn string_param(params: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| params.get(*key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn tab_ref_from_params(params: &Value) -> Option<String> {
    string_param(params, &["tab_ref", "tabRef"])
        .or_else(|| {
            params
                .get("payload")
                .and_then(|value| string_param(value, &["tab_ref", "tabRef"]))
        })
        .or_else(|| {
            params
                .get("data")
                .and_then(|value| string_param(value, &["tab_ref", "tabRef"]))
        })
}

fn insert_numeric_tab_target(map: &mut serde_json::Map<String, Value>, tab_id: u64) {
    map.entry("current_tab_id".to_string())
        .or_insert_with(|| Value::Number(tab_id.into()));
    map.entry("tab_id".to_string())
        .or_insert_with(|| Value::Number(tab_id.into()));
    map.remove("tab_ref");
    map.remove("tabRef");
}

fn normalize_tab_ref_input(params: &mut Value) -> Result<Option<BridgeTarget>> {
    let Some(tab_ref) = tab_ref_from_params(params) else {
        return Ok(None);
    };
    let parsed = parse_tab_ref(&tab_ref).map_err(|err| {
        invalid_tab_ref_error(format!(
            "Invalid tab_ref `{}`: {}. Expected rzn://browser/<browser_instance_id>/tab/<numeric_tab_id>.",
            tab_ref, err
        ))
    })?;
    let tab_id = parsed.tab_id;

    let Some(map) = params.as_object_mut() else {
        return Err(invalid_tab_ref_error(
            "tab_ref requires an object payload with browser command parameters.",
        ));
    };
    map.insert(
        "browser_instance_id".to_string(),
        Value::String(parsed.browser_instance_id.clone()),
    );
    insert_numeric_tab_target(map, tab_id);

    for key in ["payload", "data"] {
        if let Some(Value::Object(nested)) = map.get_mut(key) {
            insert_numeric_tab_target(nested, tab_id);
        }
    }

    Ok(Some(BridgeTarget::BrowserInstanceId(
        parsed.browser_instance_id,
    )))
}

fn normalized_bridge_target_from_params(params: &mut Value) -> Result<BridgeTarget> {
    normalize_tab_ref_input(params)
        .map(|target| target.unwrap_or_else(|| BridgeTarget::from_params(params)))
}

fn inherit_browser_target_params(source: &Value, target: &mut Value) {
    let Some(target_map) = target.as_object_mut() else {
        return;
    };

    if let Some(browser_target) = source.get("browser_target") {
        target_map.insert("browser_target".to_string(), browser_target.clone());
        return;
    }

    for key in [
        "bridge_id",
        "supervisor_bridge_id",
        "bridge",
        "browser_instance_id",
        "browserInstanceId",
        "browser_instance",
        "browser",
        "browser_kind",
        "browserTarget",
        "session_id",
    ] {
        if let Some(value) = source.get(key) {
            target_map.insert(key.to_string(), value.clone());
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BrowserSessionRecord {
    session_id: String,
    bridge_id: String,
    bridge_epoch: u64,
    browser_instance_id: Option<String>,
    browser: Option<String>,
    caller_origin: Option<String>,
    caller_extension_id: Option<String>,
    created_at_ms: u64,
    last_activity_at_ms: u64,
    disconnected_at_ms: Option<u64>,
    reconnected_at_ms: Option<u64>,
    reconnect_count: u64,
    suspicious_identity_at_ms: Option<u64>,
}

impl BrowserSessionRecord {
    fn from_resolved(session_id: String, resolved: &ResolvedBridgeTarget) -> Self {
        let now = now_ms();
        Self {
            session_id,
            bridge_id: resolved.bridge.id.clone(),
            bridge_epoch: resolved.bridge.epoch,
            browser_instance_id: resolved.bridge.metadata.browser_instance_id.clone(),
            browser: resolved.bridge.metadata.extension_target.clone(),
            caller_origin: resolved.bridge.metadata.caller_origin.clone(),
            caller_extension_id: resolved.bridge.metadata.caller_extension_id.clone(),
            created_at_ms: now,
            last_activity_at_ms: now,
            disconnected_at_ms: None,
            reconnected_at_ms: None,
            reconnect_count: 0,
            suspicious_identity_at_ms: None,
        }
    }

    fn target_json(&self) -> Value {
        json!({
            "session_id": self.session_id.clone(),
            "bridge_id": self.bridge_id.clone(),
            "bridge_epoch": self.bridge_epoch,
            "browser_instance_id": self.browser_instance_id.clone(),
            "browser": self.browser.clone(),
            "caller_origin": self.caller_origin.clone(),
            "caller_extension_id": self.caller_extension_id.clone(),
            "created_at_ms": self.created_at_ms,
            "last_activity_at_ms": self.last_activity_at_ms,
            "disconnected_at_ms": self.disconnected_at_ms,
            "reconnected_at_ms": self.reconnected_at_ms,
            "reconnect_count": self.reconnect_count,
            "suspicious_identity_at_ms": self.suspicious_identity_at_ms,
        })
    }
}

#[derive(Clone)]
struct ResolvedBridgeTarget {
    bridge: NativeHostBridge,
    source: &'static str,
}

impl ResolvedBridgeTarget {
    fn to_json(&self) -> Value {
        json!({
            "bridge_id": self.bridge.id.clone(),
            "supervisor_bridge_id": self.bridge.id.clone(),
            "supervisor_bridge_epoch": self.bridge.epoch,
            "browser_instance_id": self.bridge.metadata.browser_instance_id.clone(),
            "browser": self.bridge.metadata.extension_target.clone(),
            "extension_target": self.bridge.metadata.extension_target.clone(),
            "extension_target_hint": self.bridge.metadata.extension_target_hint.clone(),
            "source": self.source,
        })
    }
}

fn normalize_browser_target_slug(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    match normalized.as_str() {
        "msedge" | "microsoft-edge" => Some("edge".to_string()),
        "google-chrome" => Some("chrome".to_string()),
        value => Some(value.to_string()),
    }
}

fn session_id_from_params(params: &Value) -> Option<String> {
    params
        .get("session_id")
        .or_else(|| params.get("sessionId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn bridge_matches_browser_kind(bridge: &NativeHostBridge, browser: &str) -> bool {
    let matches = |value: &Option<String>| {
        value
            .as_deref()
            .and_then(normalize_browser_target_slug)
            .as_deref()
            == Some(browser)
    };
    matches(&bridge.metadata.extension_target) || matches(&bridge.metadata.extension_target_hint)
}

fn browser_kind_from_bridge_target(target: &BridgeTarget) -> Option<&str> {
    match target {
        BridgeTarget::BrowserKind(browser) => Some(browser.as_str()),
        BridgeTarget::Preferred(inner) => browser_kind_from_bridge_target(inner),
        _ => None,
    }
}

fn bridge_has_reported_browser_kind(bridge: &NativeHostBridge) -> bool {
    bridge
        .metadata
        .extension_target
        .as_deref()
        .and_then(normalize_browser_target_slug)
        .is_some()
        || bridge
            .metadata
            .extension_target_hint
            .as_deref()
            .and_then(normalize_browser_target_slug)
            .is_some()
}

fn bridge_browser_kind_bootstrap_candidate(
    bridges: &HashMap<String, NativeHostBridge>,
    requested_browser: &str,
) -> Option<NativeHostBridge> {
    let candidates = bridges
        .values()
        .filter(|bridge| {
            !bridge_matches_browser_kind(bridge, requested_browser)
                && !bridge_has_reported_browser_kind(bridge)
        })
        .collect::<Vec<_>>();
    if candidates.len() != 1 {
        return None;
    }
    Some((*candidates[0]).clone())
}

#[derive(Clone, Debug)]
struct BridgeProbeTargetMatch {
    ok: bool,
    requested_browser: Option<String>,
    loaded_browser: Option<String>,
    loaded_browser_hint: Option<String>,
    error: Option<String>,
}

impl BridgeProbeTargetMatch {
    fn ok() -> Self {
        Self {
            ok: true,
            requested_browser: None,
            loaded_browser: None,
            loaded_browser_hint: None,
            error: None,
        }
    }
}

fn bridge_probe_target_match(value: &Value, target: &BridgeTarget) -> BridgeProbeTargetMatch {
    let BridgeTarget::BrowserKind(requested_browser) = target else {
        return BridgeProbeTargetMatch::ok();
    };

    let loaded_browser = value
        .pointer("/result/extension_target")
        .and_then(Value::as_str)
        .and_then(normalize_browser_target_slug);
    let loaded_browser_hint = value
        .pointer("/result/extension_target_hint")
        .and_then(Value::as_str)
        .and_then(normalize_browser_target_slug);
    let matches_requested = loaded_browser.as_deref() == Some(requested_browser.as_str())
        || loaded_browser_hint.as_deref() == Some(requested_browser.as_str());

    if matches_requested {
        return BridgeProbeTargetMatch {
            ok: true,
            requested_browser: Some(requested_browser.clone()),
            loaded_browser,
            loaded_browser_hint,
            error: None,
        };
    }

    let error = if loaded_browser.is_some() || loaded_browser_hint.is_some() {
        format!(
            "readiness ping reached a browser bridge, but it reported browser target `{}`/hint `{}` instead of requested `{}`",
            loaded_browser.as_deref().unwrap_or("unknown"),
            loaded_browser_hint.as_deref().unwrap_or("unknown"),
            requested_browser
        )
    } else {
        format!(
            "readiness ping reached a browser bridge, but it did not report a browser target for requested `{}`",
            requested_browser
        )
    };

    BridgeProbeTargetMatch {
        ok: false,
        requested_browser: Some(requested_browser.clone()),
        loaded_browser,
        loaded_browser_hint,
        error: Some(error),
    }
}

fn bridge_candidate_summaries(
    bridges: &[NativeHostBridge],
    health_by_bridge: &HashMap<String, NativeBridgeHealth>,
) -> Vec<Value> {
    let mut bridges = bridges.to_vec();
    bridges.sort_by(|left, right| left.id.cmp(&right.id));
    bridges
        .iter()
        .map(|bridge| bridge_candidate_summary(bridge, health_by_bridge.get(&bridge.id)))
        .collect()
}

fn bridge_candidate_summary(
    bridge: &NativeHostBridge,
    health: Option<&NativeBridgeHealth>,
) -> Value {
    json!({
        "bridge_id": bridge.id.clone(),
        "supervisor_bridge_id": bridge.id.clone(),
        "supervisor_bridge_epoch": bridge.epoch,
        "browser_instance_id": bridge.metadata.browser_instance_id.clone(),
        "browser": bridge.metadata.extension_target.clone(),
        "extension_target": bridge.metadata.extension_target.clone(),
        "extension_target_hint": bridge.metadata.extension_target_hint.clone(),
        "extension_id": bridge
            .metadata
            .extension_reported_id
            .clone()
            .or_else(|| bridge.metadata.caller_extension_id.clone()),
        "last_health_at_ms": health.and_then(last_bridge_health_timestamp_ms),
    })
}

fn last_bridge_health_timestamp_ms(health: &NativeBridgeHealth) -> Option<u64> {
    [
        health.last_successful_ping_at_ms,
        health.last_failure_at_ms,
        health.last_step_timeout_at_ms,
        health.last_restart_requested_at_ms,
        health.current_bridge_registered_at_ms,
    ]
    .into_iter()
    .flatten()
    .max()
}

fn browser_target_error(
    code: &'static str,
    message: String,
    candidates: Vec<Value>,
) -> anyhow::Error {
    anyhow!("{}", browser_target_error_value(code, message, candidates))
}

fn browser_target_error_value(code: &str, message: String, candidates: Vec<Value>) -> Value {
    json!({
        "ok": false,
        "success": false,
        "error_code": code,
        "error": message,
        "candidates": candidates,
        "next_steps": browser_target_next_steps(&candidates),
    })
}

fn invalid_tab_ref_error(message: impl Into<String>) -> anyhow::Error {
    let value = json!({
        "ok": false,
        "success": false,
        "error_code": "INVALID_TAB_REF",
        "error": message.into(),
        "format_example": "rzn://browser/<browser_instance_id>/tab/123",
        "next_steps": [
            "Use --tab-ref rzn://browser/<browser_instance_id>/tab/<numeric_tab_id>.",
            "Use --tab <id> only together with --browser, --browser-instance, --bridge, or --session-id."
        ],
    });
    anyhow!("{}", value)
}

fn browser_target_next_steps(candidates: &[Value]) -> Vec<Value> {
    if candidates.is_empty() {
        return vec![json!(
            "Connect a browser extension bridge, then retry the command."
        )];
    }
    let mut steps = Vec::new();
    if candidates
        .iter()
        .any(|candidate| candidate.get("browser").and_then(Value::as_str).is_some())
    {
        steps.push(json!(
            "Pass --browser <chrome|chromium|edge> to select a browser kind."
        ));
    }
    if candidates.iter().any(|candidate| {
        candidate
            .get("browser_instance_id")
            .and_then(Value::as_str)
            .is_some()
    }) {
        steps.push(json!(
            "Pass --browser-instance <browser_instance_id> to select a browser profile."
        ));
    }
    steps.push(json!(
        "Pass --bridge <bridge_id> to select an exact connected bridge."
    ));
    steps
}

fn parse_browser_target_error(message: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(message).ok()?;
    let code = value.get("error_code").and_then(Value::as_str)?;
    if is_browser_target_error_code(code) {
        Some(value)
    } else {
        None
    }
}

fn is_browser_target_error_code(code: &str) -> bool {
    matches!(
        code,
        "NO_BROWSER_BRIDGE_CONNECTED"
            | "AMBIGUOUS_BROWSER_TARGET"
            | "BRIDGE_NOT_FOUND"
            | "BROWSER_INSTANCE_NOT_CONNECTED"
            | "BROWSER_TARGET_MISMATCH"
            | "SESSION_TARGET_CONFLICT"
            | "SESSION_NOT_FOUND"
            | "INVALID_TAB_REF"
    )
}

fn inject_resolved_browser_target(payload: &mut Value, resolved_target: Value) {
    if let Value::Object(map) = payload {
        map.insert("resolved_browser_target".to_string(), resolved_target);
    }
}

#[derive(Clone, Debug)]
struct BrowserTabResultContext {
    browser_instance_id: String,
    bridge_id: Option<String>,
    browser: Option<String>,
}

impl BrowserTabResultContext {
    fn from_value(value: &Value) -> Option<Self> {
        let target = value
            .get("resolved_browser_target")
            .or_else(|| value.pointer("/result/resolved_browser_target"))
            .or_else(|| value.pointer("/data/resolved_browser_target"))
            .or_else(|| value.pointer("/payload/resolved_browser_target"));
        let browser_instance_id = target
            .and_then(|target| target.get("browser_instance_id"))
            .or_else(|| value.get("browser_instance_id"))
            .or_else(|| value.pointer("/result/browser_instance_id"))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())?
            .to_string();
        Some(Self {
            browser_instance_id,
            bridge_id: target
                .and_then(|target| {
                    target
                        .get("bridge_id")
                        .or_else(|| target.get("supervisor_bridge_id"))
                })
                .or_else(|| value.get("bridge_id"))
                .or_else(|| value.get("supervisor_bridge_id"))
                .or_else(|| value.pointer("/result/bridge_id"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string),
            browser: target
                .and_then(|target| {
                    target
                        .get("browser")
                        .or_else(|| target.get("extension_target"))
                        .or_else(|| target.get("extension_target_hint"))
                })
                .or_else(|| value.get("browser"))
                .or_else(|| value.get("extension_target"))
                .or_else(|| value.pointer("/result/browser"))
                .or_else(|| value.pointer("/result/extension_target"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string),
        })
    }
}

fn tab_id_from_map(map: &serde_json::Map<String, Value>) -> Option<u64> {
    map.get("current_tab_id")
        .or_else(|| map.get("tab_id"))
        .or_else(|| map.get("tabId"))
        .and_then(Value::as_u64)
}

fn inject_tab_ref_into_object(
    map: &mut serde_json::Map<String, Value>,
    context: &BrowserTabResultContext,
    fallback_tab_id: Option<u64>,
) {
    let tab_id = tab_id_from_map(map).or(fallback_tab_id);
    if let Some(bridge_id) = &context.bridge_id {
        map.entry("bridge_id".to_string())
            .or_insert_with(|| Value::String(bridge_id.clone()));
    }
    if let Some(browser) = &context.browser {
        map.entry("browser".to_string())
            .or_insert_with(|| Value::String(browser.clone()));
    }
    map.entry("browser_instance_id".to_string())
        .or_insert_with(|| Value::String(context.browser_instance_id.clone()));

    let Some(tab_id) = tab_id else {
        return;
    };
    map.entry("tab_id".to_string())
        .or_insert_with(|| Value::Number(tab_id.into()));
    let Ok(tab_ref) = format_tab_ref(&context.browser_instance_id, tab_id) else {
        return;
    };
    map.entry("tab_ref".to_string())
        .or_insert_with(|| Value::String(tab_ref.clone()));
    if map.contains_key("current_tab_id") || fallback_tab_id.is_some() {
        map.entry("current_tab_ref".to_string())
            .or_insert_with(|| Value::String(tab_ref));
    }
}

fn inject_tab_ref_from_browser_target(value: &mut Value) {
    let Some(context) = BrowserTabResultContext::from_value(value) else {
        return;
    };
    let root_tab_id = value
        .get("current_tab_id")
        .or_else(|| value.get("tab_id"))
        .and_then(Value::as_u64);

    if let Value::Object(map) = value {
        inject_tab_ref_into_object(map, &context, None);
        if let Some(Value::Object(result)) = map.get_mut("result") {
            if root_tab_id.is_some() && tab_id_from_map(result).is_none() {
                result
                    .entry("current_tab_id".to_string())
                    .or_insert_with(|| Value::Number(root_tab_id.unwrap().into()));
            }
            inject_tab_ref_into_object(result, &context, root_tab_id);

            if let Some(Value::Array(results)) = result.get_mut("results") {
                for item in results.iter_mut() {
                    if let Value::Object(item) = item {
                        inject_tab_ref_into_object(item, &context, None);
                    }
                }
            }
        }
    }
}

fn resolved_targets_match_session(left: &NativeHostBridge, right: &NativeHostBridge) -> bool {
    match (
        left.metadata.browser_instance_id.as_deref(),
        right.metadata.browser_instance_id.as_deref(),
    ) {
        (Some(left_instance), Some(right_instance)) => left_instance == right_instance,
        _ => left.id == right.id,
    }
}

fn session_bridge_identity_compatible(
    session: &BrowserSessionRecord,
    bridge: &NativeHostBridge,
) -> bool {
    if let (Some(session_origin), Some(bridge_origin)) = (
        session.caller_origin.as_deref(),
        bridge.metadata.caller_origin.as_deref(),
    ) {
        if session_origin != bridge_origin {
            return false;
        }
    }
    if let (Some(session_extension), Some(bridge_extension)) = (
        session.caller_extension_id.as_deref(),
        bridge.metadata.caller_extension_id.as_deref(),
    ) {
        if session_extension != bridge_extension {
            return false;
        }
    }
    true
}

#[derive(Clone, Debug, Default)]
struct NativeBridgeHealth {
    supervisor_boot_id: Option<String>,
    current_supervisor_bridge_epoch: Option<u64>,
    current_bridge_id: Option<String>,
    current_bridge_registered_at_ms: Option<u64>,
    current_bridge_metadata: Option<NativeHostBridgeMetadata>,
    active_request_count: u64,
    active_request_id: Option<String>,
    active_method: Option<String>,
    active_deadline_at_ms: Option<u64>,
    last_successful_ping_at_ms: Option<u64>,
    last_successful_ping_latency_ms: Option<u64>,
    last_successful_extension_build_signature: Option<Value>,
    last_successful_bridge_contract_version: Option<u64>,
    last_successful_supervisor_bridge_epoch: Option<u64>,
    last_successful_native_host_pid: Option<u64>,
    last_successful_native_host_boot_id: Option<String>,
    last_successful_extension_worker_boot_id: Option<String>,
    last_successful_native_port_epoch: Option<u64>,
    last_successful_stdout_heartbeat_ms: Option<u64>,
    last_successful_stdout_heartbeat_age_ms: Option<u64>,
    last_successful_stdout_heartbeat_seq: Option<u64>,
    last_successful_roundtrip_ping_ms: Option<u64>,
    last_successful_roundtrip_ping_age_ms: Option<u64>,
    missed_roundtrip_count: Option<u64>,
    last_failure_at_ms: Option<u64>,
    last_failure_cause: Option<String>,
    last_failure_error: Option<String>,
    last_step_timeout_at_ms: Option<u64>,
    last_restart_requested_at_ms: Option<u64>,
    last_restart_reason: Option<String>,
    native_host_restart_count: u64,
    bridge_registration_count: u64,
    bridge_unregistration_count: u64,
    target_resolution_count: u64,
    ambiguous_target_count: u64,
    session_rebind_count: u64,
    timeout_count: u64,
    stale_bridge_response_drop_count: u64,
    bridge_response_mismatch_drop_count: u64,
}

impl NativeBridgeHealth {
    fn new_for_bridge(
        supervisor_boot_id: String,
        bridge: &NativeHostBridge,
        metadata: NativeHostBridgeMetadata,
    ) -> Self {
        Self {
            supervisor_boot_id: Some(supervisor_boot_id),
            current_supervisor_bridge_epoch: Some(bridge.epoch),
            current_bridge_id: Some(bridge.id.clone()),
            current_bridge_registered_at_ms: Some(bridge.registered_at_ms),
            current_bridge_metadata: Some(metadata),
            ..Self::default()
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "supervisor_boot_id": self.supervisor_boot_id.clone(),
            "current_supervisor_bridge_epoch": self.current_supervisor_bridge_epoch,
            "current_bridge_id": self.current_bridge_id.clone(),
            "current_bridge_registered_at_ms": self.current_bridge_registered_at_ms,
            "current_bridge_metadata": self.current_bridge_metadata.clone(),
            "active_request_count": self.active_request_count,
            "active_request_id": self.active_request_id.clone(),
            "active_method": self.active_method.clone(),
            "active_deadline_at_ms": self.active_deadline_at_ms,
            "last_successful_ping_at_ms": self.last_successful_ping_at_ms,
            "last_successful_ping_latency_ms": self.last_successful_ping_latency_ms,
            "last_successful_extension_build_signature": self
                .last_successful_extension_build_signature
                .clone()
                .unwrap_or(Value::Null),
            "last_successful_bridge_contract_version": self.last_successful_bridge_contract_version,
            "last_successful_supervisor_bridge_epoch": self.last_successful_supervisor_bridge_epoch,
            "last_successful_native_host_pid": self.last_successful_native_host_pid,
            "last_successful_native_host_boot_id": self.last_successful_native_host_boot_id.clone(),
            "last_successful_extension_worker_boot_id": self.last_successful_extension_worker_boot_id.clone(),
            "last_successful_native_port_epoch": self.last_successful_native_port_epoch,
            "last_successful_stdout_heartbeat_ms": self.last_successful_stdout_heartbeat_ms,
            "last_successful_stdout_heartbeat_age_ms": self.last_successful_stdout_heartbeat_age_ms,
            "last_successful_stdout_heartbeat_seq": self.last_successful_stdout_heartbeat_seq,
            "last_successful_roundtrip_ping_ms": self.last_successful_roundtrip_ping_ms,
            "last_successful_roundtrip_ping_age_ms": self.last_successful_roundtrip_ping_age_ms,
            "missed_roundtrip_count": self.missed_roundtrip_count,
            "last_failure_at_ms": self.last_failure_at_ms,
            "last_failure_cause": self.last_failure_cause.clone(),
            "last_failure_error": self.last_failure_error.clone(),
            "last_step_timeout_at_ms": self.last_step_timeout_at_ms,
            "last_restart_requested_at_ms": self.last_restart_requested_at_ms,
            "last_restart_reason": self.last_restart_reason.clone(),
            "native_host_restart_count": self.native_host_restart_count,
            "bridge_registration_count": self.bridge_registration_count,
            "bridge_unregistration_count": self.bridge_unregistration_count,
            "target_resolution_count": self.target_resolution_count,
            "ambiguous_target_count": self.ambiguous_target_count,
            "session_rebind_count": self.session_rebind_count,
            "timeout_count": self.timeout_count,
            "stale_bridge_response_drop_count": self.stale_bridge_response_drop_count,
            "bridge_response_mismatch_drop_count": self.bridge_response_mismatch_drop_count,
        })
    }
}

impl SupervisorState {
    fn new(config: SupervisorConfig) -> Self {
        let paths = SupervisorPaths::for_config(&config);
        let supervisor_boot_id = format!("supervisor-{}", Uuid::new_v4());
        Self {
            config,
            paths,
            supervisor_boot_id,
            bridge_epoch_counter: AtomicU64::new(0),
            native_bridges: Mutex::new(HashMap::new()),
            native_bridge_pending: Mutex::new(HashMap::new()),
            native_bridge_health: Mutex::new(HashMap::new()),
            last_registered_bridge_id: Mutex::new(None),
            cloud_actor: None,
            sessions: Mutex::new(HashMap::new()),
            shutdown: AtomicBool::new(false),
        }
    }

    fn with_cloud_actor(config: SupervisorConfig, cloud_actor: SupervisorCloudActor) -> Self {
        let mut state = Self::new(config);
        state.cloud_actor = Some(cloud_actor);
        state
    }

    async fn native_bridge_count(&self) -> usize {
        self.native_bridges.lock().await.len()
    }

    async fn has_native_bridge(&self) -> bool {
        self.native_bridge_count().await > 0
    }

    async fn current_native_bridge(&self) -> Option<NativeHostBridge> {
        let bridges = self.native_bridges.lock().await;
        if bridges.len() == 1 {
            bridges.values().next().cloned()
        } else {
            None
        }
    }

    async fn current_native_bridge_id(&self) -> Option<String> {
        self.current_native_bridge().await.map(|bridge| bridge.id)
    }

    async fn resolve_native_bridge_target(
        &self,
        target: &BridgeTarget,
        allow_reconnect_wait: bool,
    ) -> Result<Option<ResolvedBridgeTarget>> {
        if allow_reconnect_wait && !self.has_native_bridge().await {
            let _ = self
                .wait_for_native_bridge(BRIDGE_RECONNECT_BEFORE_CALL_WAIT_MS)
                .await;
        }

        let session_binding = match target {
            BridgeTarget::SessionId(session_id) => {
                self.sessions.lock().await.get(session_id).cloned()
            }
            _ => None,
        };
        let bridges = self.native_bridges.lock().await;
        let health_by_bridge = self.native_bridge_health.lock().await.clone();
        match target {
            BridgeTarget::BridgeId(bridge_id) => bridges
                .get(bridge_id)
                .cloned()
                .map(|bridge| {
                    Some(ResolvedBridgeTarget {
                        bridge,
                        source: "bridge_id",
                    })
                })
                .ok_or_else(|| {
                    browser_target_error(
                        "BRIDGE_NOT_FOUND",
                        format!("Bridge `{}` is not connected.", bridge_id),
                        Vec::new(),
                    )
                }),
            BridgeTarget::BrowserInstanceId(browser_instance_id) => {
                let mut candidates = bridges
                    .values()
                    .filter(|bridge| {
                        bridge.metadata.browser_instance_id.as_deref() == Some(browser_instance_id)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if candidates.is_empty() {
                    return Err(browser_target_error(
                        "BROWSER_INSTANCE_NOT_CONNECTED",
                        format!(
                            "Browser instance `{}` is not connected.",
                            browser_instance_id
                        ),
                        Vec::new(),
                    ));
                }
                candidates.sort_by_key(|bridge| (bridge.registered_at_ms, bridge.epoch));
                let bridge = candidates
                    .pop()
                    .expect("non-empty browser instance candidates");
                Ok(Some(ResolvedBridgeTarget {
                    bridge,
                    source: "browser_instance_id",
                }))
            }
            BridgeTarget::SessionId(session_id) => {
                let Some(binding) = session_binding else {
                    return Err(browser_target_error(
                        "SESSION_NOT_FOUND",
                        format!("Session `{}` is not bound.", session_id),
                        Vec::new(),
                    ));
                };
                let bridge = match self.bridge_for_session_binding(&bridges, &binding) {
                    Some(bridge) => bridge,
                    None => {
                        if self.has_incompatible_same_instance_candidate(&bridges, &binding) {
                            self.mark_session_identity_suspicious(session_id).await;
                            return Err(browser_target_error(
                                "SESSION_TARGET_CONFLICT",
                                format!(
                                    "Session `{}` has a reconnect candidate with conflicting browser identity.",
                                    session_id
                                ),
                                Vec::new(),
                            ));
                        }
                        return Err(browser_target_error(
                            "BROWSER_INSTANCE_NOT_CONNECTED",
                            format!("Session `{}` target is not connected.", session_id),
                            Vec::new(),
                        ));
                    }
                };
                self.refresh_session_activity(session_id, &bridge).await;
                Ok(Some(ResolvedBridgeTarget {
                    bridge,
                    source: "session_id",
                }))
            }
            BridgeTarget::BrowserKind(browser) => {
                let candidates = bridges
                    .values()
                    .filter(|bridge| bridge_matches_browser_kind(bridge, browser))
                    .cloned()
                    .collect::<Vec<_>>();
                match candidates.len() {
                    0 => {
                        if let Some(bridge) =
                            bridge_browser_kind_bootstrap_candidate(&bridges, browser)
                        {
                            Ok(Some(ResolvedBridgeTarget {
                                bridge,
                                source: "browser_bootstrap_single_bridge",
                            }))
                        } else {
                            Err(browser_target_error(
                                "BROWSER_INSTANCE_NOT_CONNECTED",
                                format!("Browser `{}` is not connected.", browser),
                                Vec::new(),
                            ))
                        }
                    }
                    1 => Ok(candidates
                        .into_iter()
                        .next()
                        .map(|bridge| ResolvedBridgeTarget {
                            bridge,
                            source: "browser",
                        })),
                    _ => Err(browser_target_error(
                        "AMBIGUOUS_BROWSER_TARGET",
                        format!("Browser `{}` matched multiple connected bridges.", browser),
                        bridge_candidate_summaries(&candidates, &health_by_bridge),
                    )),
                }
            }
            BridgeTarget::Preferred(preferred) => {
                self.resolve_preferred_bridge_target(preferred, &bridges, &health_by_bridge)
            }
            BridgeTarget::Default => {
                self.resolve_default_bridge_target(&bridges, &health_by_bridge)
            }
        }
    }

    fn resolve_preferred_bridge_target(
        &self,
        preferred: &BridgeTarget,
        bridges: &HashMap<String, NativeHostBridge>,
        health_by_bridge: &HashMap<String, NativeBridgeHealth>,
    ) -> Result<Option<ResolvedBridgeTarget>> {
        match preferred {
            BridgeTarget::BridgeId(bridge_id) => {
                if let Some(bridge) = bridges.get(bridge_id).cloned() {
                    return Ok(Some(ResolvedBridgeTarget {
                        bridge,
                        source: "preferred_bridge_id",
                    }));
                }
                self.resolve_default_bridge_target(bridges, health_by_bridge)
            }
            BridgeTarget::BrowserInstanceId(browser_instance_id) => {
                let mut candidates = bridges
                    .values()
                    .filter(|bridge| {
                        bridge.metadata.browser_instance_id.as_deref() == Some(browser_instance_id)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if candidates.is_empty() {
                    return self.resolve_default_bridge_target(bridges, health_by_bridge);
                }
                candidates.sort_by_key(|bridge| (bridge.registered_at_ms, bridge.epoch));
                let bridge = candidates
                    .pop()
                    .expect("non-empty preferred browser instance candidates");
                Ok(Some(ResolvedBridgeTarget {
                    bridge,
                    source: "preferred_browser_instance_id",
                }))
            }
            BridgeTarget::BrowserKind(browser) => {
                let candidates = bridges
                    .values()
                    .filter(|bridge| bridge_matches_browser_kind(bridge, browser))
                    .cloned()
                    .collect::<Vec<_>>();
                match candidates.len() {
                    0 => self.resolve_default_bridge_target(bridges, health_by_bridge),
                    1 => Ok(candidates
                        .into_iter()
                        .next()
                        .map(|bridge| ResolvedBridgeTarget {
                            bridge,
                            source: "preferred_browser",
                        })),
                    _ => Err(browser_target_error(
                        "AMBIGUOUS_BROWSER_TARGET",
                        format!(
                            "Default browser `{}` matched multiple connected bridges.",
                            browser
                        ),
                        bridge_candidate_summaries(&candidates, health_by_bridge),
                    )),
                }
            }
            BridgeTarget::Default | BridgeTarget::SessionId(_) | BridgeTarget::Preferred(_) => {
                self.resolve_default_bridge_target(bridges, health_by_bridge)
            }
        }
    }

    fn resolve_default_bridge_target(
        &self,
        bridges: &HashMap<String, NativeHostBridge>,
        health_by_bridge: &HashMap<String, NativeBridgeHealth>,
    ) -> Result<Option<ResolvedBridgeTarget>> {
        match bridges.len() {
            0 => Err(browser_target_error(
                "NO_BROWSER_BRIDGE_CONNECTED",
                "No browser bridge is connected.".to_string(),
                Vec::new(),
            )),
            1 => Ok(bridges
                .values()
                .next()
                .cloned()
                .map(|bridge| ResolvedBridgeTarget {
                    bridge,
                    source: "single_bridge_default",
                })),
            _ => {
                let candidates = bridges.values().cloned().collect::<Vec<_>>();
                Err(browser_target_error(
                    "AMBIGUOUS_BROWSER_TARGET",
                    "Multiple browser bridges are connected; pass --browser, --browser-instance, --bridge, or save a default browser target.".to_string(),
                    bridge_candidate_summaries(&candidates, health_by_bridge),
                ))
            }
        }
    }

    fn bridge_for_session_binding(
        &self,
        bridges: &HashMap<String, NativeHostBridge>,
        binding: &BrowserSessionRecord,
    ) -> Option<NativeHostBridge> {
        if let Some(browser_instance_id) = binding.browser_instance_id.as_deref() {
            let mut candidates = bridges
                .values()
                .filter(|bridge| {
                    bridge.metadata.browser_instance_id.as_deref() == Some(browser_instance_id)
                        && session_bridge_identity_compatible(binding, bridge)
                })
                .cloned()
                .collect::<Vec<_>>();
            if !candidates.is_empty() {
                candidates.sort_by_key(|bridge| (bridge.registered_at_ms, bridge.epoch));
                return candidates.pop();
            }
        }
        bridges.get(&binding.bridge_id).cloned()
    }

    fn has_incompatible_same_instance_candidate(
        &self,
        bridges: &HashMap<String, NativeHostBridge>,
        binding: &BrowserSessionRecord,
    ) -> bool {
        let Some(browser_instance_id) = binding.browser_instance_id.as_deref() else {
            return false;
        };
        bridges.values().any(|bridge| {
            bridge.metadata.browser_instance_id.as_deref() == Some(browser_instance_id)
                && !session_bridge_identity_compatible(binding, bridge)
        })
    }

    async fn refresh_session_activity(&self, session_id: &str, bridge: &NativeHostBridge) {
        if let Some(session) = self.sessions.lock().await.get_mut(session_id) {
            if session.bridge_id != bridge.id {
                session.reconnect_count = session.reconnect_count.saturating_add(1);
                session.reconnected_at_ms = Some(now_ms());
            }
            session.bridge_id = bridge.id.clone();
            session.bridge_epoch = bridge.epoch;
            session.browser_instance_id = bridge.metadata.browser_instance_id.clone();
            session.browser = bridge.metadata.extension_target.clone();
            session.caller_origin = bridge.metadata.caller_origin.clone();
            session.caller_extension_id = bridge.metadata.caller_extension_id.clone();
            session.disconnected_at_ms = None;
            session.last_activity_at_ms = now_ms();
        }
    }

    async fn mark_sessions_for_disconnected_bridge(&self, bridge_id: &str) {
        let now = now_ms();
        for session in self.sessions.lock().await.values_mut() {
            if session.bridge_id == bridge_id {
                session.disconnected_at_ms = Some(now);
            }
        }
    }

    async fn mark_session_identity_suspicious(&self, session_id: &str) {
        if let Some(session) = self.sessions.lock().await.get_mut(session_id) {
            session.suspicious_identity_at_ms = Some(now_ms());
        }
    }

    async fn ensure_session_target_compatible(
        &self,
        params: &Value,
        target: &BridgeTarget,
        allow_reconnect_wait: bool,
    ) -> Result<()> {
        let Some(session_id) = session_id_from_params(params) else {
            return Ok(());
        };
        if matches!(target, BridgeTarget::SessionId(_)) {
            return Ok(());
        }
        let Some(explicit_target) = BridgeTarget::explicit_from_params(params) else {
            return Ok(());
        };
        let session_target = self
            .resolve_native_bridge_target(
                &BridgeTarget::SessionId(session_id.clone()),
                allow_reconnect_wait,
            )
            .await?;
        let explicit_target = self
            .resolve_native_bridge_target(&explicit_target, allow_reconnect_wait)
            .await?;
        let (Some(session_target), Some(explicit_target)) = (session_target, explicit_target)
        else {
            return Ok(());
        };
        if resolved_targets_match_session(&session_target.bridge, &explicit_target.bridge) {
            return Ok(());
        }
        Err(browser_target_error(
            "SESSION_TARGET_CONFLICT",
            format!(
                "Session `{}` is bound to bridge `{}`/instance `{}` but explicit target resolved to bridge `{}`/instance `{}`.",
                session_id,
                session_target.bridge.id,
                session_target
                    .bridge
                    .metadata
                    .browser_instance_id
                    .as_deref()
                    .unwrap_or(""),
                explicit_target.bridge.id,
                explicit_target
                    .bridge
                    .metadata
                    .browser_instance_id
                    .as_deref()
                    .unwrap_or("")
            ),
            vec![
                session_target.to_json(),
                explicit_target.to_json(),
            ],
        ))
    }

    async fn bridge_id_from_ping_response(&self, ping_response: &Value) -> Option<String> {
        let reported = ping_response
            .pointer("/result/supervisor_bridge_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .or_else(|| {
                ping_response
                    .pointer("/resolved_browser_target/bridge_id")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string)
            })
            .or_else(|| {
                ping_response
                    .pointer("/result/resolved_browser_target/bridge_id")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string)
            })
            .or_else(|| {
                ping_response
                    .pointer("/result/current_bridge_id")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string)
            });
        if reported.is_some() {
            reported
        } else {
            self.current_native_bridge_id().await
        }
    }

    async fn dispatch(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "runtime.hello" | "runtime.status" => Ok(self.runtime_status().await),
            "browser.targets" | "runtime.bridges" => Ok(self.browser_targets().await),
            "runtime.ensure_ready" => self.ensure_ready(params).await,
            "runtime.heal" => self.runtime_heal(params).await,
            "cloud.status" => Ok(self.cloud_status().await),
            "cloud.set_config" => self.cloud_set_config(params).await,
            "cloud.clear_config" => self.cloud_clear_config().await,
            "runtime.shutdown" => {
                self.shutdown.store(true, Ordering::SeqCst);
                Ok(json!({ "ok": true, "shutdown": true }))
            }
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| anyhow!("tools/call missing params.name"))?;
                let args = tool_call_arguments(&params);
                self.dispatch_tool(name, args).await
            }
            name if is_browser_tool(name) => self.dispatch_tool(name, params).await,
            _ => Err(anyhow!("Unknown supervisor method: {}", method)),
        }
    }

    async fn dispatch_tool(&self, name: &str, params: Value) -> Result<Value> {
        match name {
            "rzn.supervisor.health" => Ok(self.runtime_status().await),
            "browser.session_open"
            | "browser.session_close"
            | "browser.targets"
            | "browser.snapshot"
            | "browser.execute_step"
            | "browser.static_command"
            | "browser.poll_events" => {
                if let Some(value) = self
                    .try_dispatch_supervisor_browser_tool(name, params.clone())
                    .await?
                {
                    Ok(value)
                } else {
                    let mut readiness_params = json!({
                        "bridge_wait_ms": TOOL_DISPATCH_RECOVERY_WAIT_MS,
                        "bridge_probe_timeout_ms": DEFAULT_BRIDGE_PROBE_TIMEOUT_MS
                    });
                    inherit_browser_target_params(&params, &mut readiness_params);
                    let readiness = self.ensure_ready(readiness_params).await?;
                    if readiness_value_ready(&readiness) {
                        if let Some(value) = self
                            .try_dispatch_supervisor_browser_tool(name, params)
                            .await?
                        {
                            return Ok(value);
                        }
                    }
                    Err(self.native_bridge_required_error(name, Some(&readiness)))
                }
            }
            _ => Err(anyhow!("Unknown supervisor tool: {}", name)),
        }
    }

    async fn runtime_status(&self) -> Value {
        json!({
            "ok": true,
            "protocol": RZN_LOCAL_PROTOCOL_VERSION,
            "pid": std::process::id(),
            "app_base": self.paths.app_base.to_string_lossy(),
            "socket_path": self.paths.socket_path.to_string_lossy(),
            "token_path": self.paths.token_path.to_string_lossy(),
            "lock_path": self.paths.lock_path.to_string_lossy(),
            "browser_proxy": {
                "mode": "native_host_bridge_required",
                "authority": "supervisor_handshake"
            },
            "native_host_bridge": {
                "connected": self.has_native_bridge().await,
                "health": self.native_bridge_health_json().await
            },
            "cloud": self.cloud_status().await
        })
    }

    async fn browser_targets(&self) -> Value {
        let bridges = self.native_bridges.lock().await.clone();
        let sessions = self.sessions.lock().await.clone();
        let health = self.native_bridge_health_json().await;
        let health_by_bridge = health
            .get("bridges")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        let mut bridge_ids = bridges.keys().cloned().collect::<Vec<_>>();
        bridge_ids.sort();
        let targets = bridge_ids
            .iter()
            .filter_map(|bridge_id| {
                let bridge = bridges.get(bridge_id)?;
                let metadata = bridge.metadata.clone();
                let active_sessions = sessions
                    .values()
                    .filter(|session| {
                        session.bridge_id == *bridge_id && session.disconnected_at_ms.is_none()
                    })
                    .map(BrowserSessionRecord::target_json)
                    .collect::<Vec<_>>();
                let health = health_by_bridge
                    .get(bridge_id)
                    .cloned()
                    .unwrap_or(Value::Null);
                let last_ping_status = if health
                    .get("last_successful_ping_at_ms")
                    .and_then(Value::as_u64)
                    .is_some()
                {
                    "ok"
                } else {
                    "unknown"
                };
                Some(json!({
                    "bridge_id": bridge.id.clone(),
                    "supervisor_bridge_id": bridge.id.clone(),
                    "supervisor_bridge_epoch": bridge.epoch,
                    "connected_since_ms": bridge.registered_at_ms,
                    "browser_instance_id": metadata.browser_instance_id.clone(),
                    "browser": metadata.extension_target.clone(),
                    "extension_target": metadata.extension_target.clone(),
                    "extension_target_hint": metadata.extension_target_hint.clone(),
                    "extension_id": metadata
                        .extension_reported_id
                        .clone()
                        .or(metadata.caller_extension_id.clone()),
                    "caller_origin": metadata.caller_origin.clone(),
                    "caller_origin_status": metadata.caller_origin_status.clone(),
                    "identity_status": metadata.identity_status.clone(),
                    "identity_match": metadata.identity_match,
                    "last_ping_status": last_ping_status,
                    "last_successful_ping_at_ms": health
                        .get("last_successful_ping_at_ms")
                        .and_then(Value::as_u64),
                    "last_successful_ping_latency_ms": health
                        .get("last_successful_ping_latency_ms")
                        .and_then(Value::as_u64),
                    "last_failure_cause": health
                        .get("last_failure_cause")
                        .and_then(Value::as_str),
                    "active_request_count": health
                        .get("active_request_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    "native_host_restart_count": health
                        .get("native_host_restart_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    "bridge_registration_count": health
                        .get("bridge_registration_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    "bridge_unregistration_count": health
                        .get("bridge_unregistration_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    "target_resolution_count": health
                        .get("target_resolution_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    "ambiguous_target_count": health
                        .get("ambiguous_target_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    "session_rebind_count": health
                        .get("session_rebind_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    "timeout_count": health
                        .get("timeout_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    "stale_bridge_response_drop_count": health
                        .get("stale_bridge_response_drop_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    "bridge_response_mismatch_drop_count": health
                        .get("bridge_response_mismatch_drop_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    "active_session_count": active_sessions.len(),
                    "active_sessions": active_sessions,
                    "current_tab": Value::Null,
                    "target_flags": {
                        "bridge": bridge.id.clone(),
                        "browser_instance": metadata.browser_instance_id.clone(),
                    },
                    "health": health,
                    "metadata": metadata,
                }))
            })
            .collect::<Vec<_>>();
        json!({
            "ok": true,
            "version": "rzn.runtime.bridges.v1",
            "capabilities": {
                "bridge_status_api": true,
                "active_sessions": true,
                "health_counters": true,
            },
            "status": if targets.is_empty() { "no_bridges_connected" } else { "connected" },
            "target_count": targets.len(),
            "bridge_count": targets.len(),
            "targets": targets.clone(),
            "bridges": targets,
        })
    }

    async fn ensure_ready(&self, mut params: Value) -> Result<Value> {
        let target = normalized_bridge_target_from_params(&mut params)?;
        let wait_ms = params
            .get("bridge_wait_ms")
            .and_then(|value| value.as_u64())
            .or_else(|| env_u64("RZN_SUPERVISOR_BRIDGE_WAIT_MS"))
            .unwrap_or(DEFAULT_BRIDGE_WAIT_MS);
        let native_host_bridge_connected = self.wait_for_native_bridge(wait_ms).await;
        let verify_bridge = params
            .get("verify_bridge")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let bridge_probe_timeout_ms = params
            .get("bridge_probe_timeout_ms")
            .and_then(|value| value.as_u64())
            .or_else(|| env_u64("RZN_SUPERVISOR_BRIDGE_PROBE_TIMEOUT_MS"))
            .unwrap_or(DEFAULT_BRIDGE_PROBE_TIMEOUT_MS);
        let bridge_probe = if native_host_bridge_connected && verify_bridge {
            let probe = self
                .probe_native_bridge_target(target.clone(), bridge_probe_timeout_ms)
                .await;
            if let Some(browser) = browser_kind_from_bridge_target(&target) {
                if bridge_probe_has_target_resolution_error(&probe) {
                    self.probe_unknown_browser_bridges(browser, bridge_probe_timeout_ms)
                        .await;
                    Some(
                        self.probe_native_bridge_target(target, bridge_probe_timeout_ms)
                            .await,
                    )
                } else {
                    Some(probe)
                }
            } else {
                Some(probe)
            }
        } else {
            None
        };
        let native_host_bridge_transport_ok = native_host_bridge_connected
            && bridge_probe
                .as_ref()
                .map(|probe| {
                    probe.get("transport_ok").and_then(|value| value.as_bool()) == Some(true)
                })
                .unwrap_or(false);
        let native_host_bridge_required_capabilities_ok = native_host_bridge_connected
            && bridge_probe
                .as_ref()
                .map(bridge_probe_required_capabilities_ok)
                .unwrap_or(false);
        let native_host_bridge_target_ok = native_host_bridge_connected
            && bridge_probe
                .as_ref()
                .map(bridge_probe_target_ok)
                .unwrap_or(false);
        let native_host_bridge_ready = native_host_bridge_transport_ok
            && native_host_bridge_required_capabilities_ok
            && native_host_bridge_target_ok;
        let ready = native_host_bridge_ready;
        let diagnostic = bridge_readiness_diagnostic(
            native_host_bridge_connected,
            native_host_bridge_ready,
            bridge_probe.as_ref(),
        );
        let diagnostic_cause = diagnostic.get("cause").cloned().unwrap_or(Value::Null);
        let error = if ready {
            Value::Null
        } else {
            diagnostic
                .get("message")
                .cloned()
                .unwrap_or_else(|| json!("Supervisor runtime is not ready."))
        };
        let remediation = if ready {
            json!([])
        } else {
            diagnostic
                .get("actions")
                .cloned()
                .unwrap_or_else(|| json!([]))
        };
        Ok(json!({
            "ok": ready,
            "ready": ready,
            "protocol": RZN_LOCAL_PROTOCOL_VERSION,
            "pid": std::process::id(),
            "app_base": self.paths.app_base.to_string_lossy(),
            "browser_proxy": {
                "mode": "native_host_bridge_required"
            },
            "native_host_bridge": {
                "connected": native_host_bridge_connected,
                "responsive": native_host_bridge_ready,
                "transport_ok": native_host_bridge_transport_ok,
                "required_capabilities_ok": native_host_bridge_required_capabilities_ok,
                "target_ok": native_host_bridge_target_ok,
                "capability_policy": bridge_readiness_capability_policy(),
                "cause": diagnostic_cause,
                "probe": bridge_probe,
                "wait_ms": wait_ms,
                "health": self.native_bridge_health_json().await
            },
            "diagnostic": diagnostic,
            "error": error,
            "remediation": remediation
        }))
    }

    async fn native_bridge_health_json(&self) -> Value {
        let active_by_bridge = {
            let guard = self.native_bridge_pending.lock().await;
            let mut active_by_bridge: HashMap<String, (u64, Option<(String, String, u64, u64)>)> =
                HashMap::new();
            for (id, pending) in guard.iter() {
                let entry = active_by_bridge
                    .entry(pending.bridge_id.clone())
                    .or_insert((0, None));
                entry.0 = entry.0.saturating_add(1);
                let active = (
                    id.clone(),
                    pending.method.clone(),
                    pending.deadline_at_ms,
                    pending.bridge_epoch,
                );
                if entry
                    .1
                    .as_ref()
                    .map(|(_, _, deadline_at_ms, _)| active.2 < *deadline_at_ms)
                    .unwrap_or(true)
                {
                    entry.1 = Some(active);
                }
            }
            active_by_bridge
        };

        let bridges = self.native_bridges.lock().await.clone();
        let last_registered_bridge_id = self.last_registered_bridge_id.lock().await.clone();
        let mut health_by_bridge = self.native_bridge_health.lock().await.clone();
        for (bridge_id, bridge) in bridges.iter() {
            health_by_bridge
                .entry(bridge_id.clone())
                .or_insert_with(|| {
                    NativeBridgeHealth::new_for_bridge(
                        self.supervisor_boot_id.clone(),
                        bridge,
                        bridge.metadata.clone(),
                    )
                });
        }

        for (bridge_id, health) in health_by_bridge.iter_mut() {
            let active = active_by_bridge.get(bridge_id);
            health.active_request_count = active.map(|(count, _)| *count).unwrap_or(0);
            if let Some(Some((id, method, deadline_at_ms, bridge_epoch))) =
                active.map(|(_, active)| active.clone())
            {
                health.active_request_id = Some(id);
                health.active_method = Some(method);
                health.active_deadline_at_ms = Some(deadline_at_ms);
                health.current_supervisor_bridge_epoch = health
                    .current_supervisor_bridge_epoch
                    .or(Some(bridge_epoch));
            } else {
                health.active_request_id = None;
                health.active_method = None;
                health.active_deadline_at_ms = None;
            }
        }

        let mut bridge_ids = health_by_bridge.keys().cloned().collect::<Vec<_>>();
        bridge_ids.sort();
        let bridges_json = bridge_ids
            .iter()
            .map(|bridge_id| {
                let mut value = health_by_bridge
                    .get(bridge_id)
                    .cloned()
                    .unwrap_or_default()
                    .to_json();
                value["connected"] = json!(bridges.contains_key(bridge_id));
                value["is_last_registered"] =
                    json!(last_registered_bridge_id.as_deref() == Some(bridge_id));
                (bridge_id.clone(), value)
            })
            .collect::<serde_json::Map<_, _>>();

        let connected_bridge_count = bridges.len() as u64;
        let healthy_bridge_count = bridges
            .keys()
            .filter(|bridge_id| {
                health_by_bridge
                    .get(*bridge_id)
                    .map(|health| {
                        health
                            .last_successful_ping_at_ms
                            .map(|success_at| {
                                health
                                    .last_failure_at_ms
                                    .map(|failure_at| success_at > failure_at)
                                    .unwrap_or(true)
                            })
                            .unwrap_or(false)
                    })
                    .unwrap_or(false)
            })
            .count() as u64;
        let current_health = if bridges.len() == 1 {
            bridges
                .keys()
                .next()
                .and_then(|bridge_id| health_by_bridge.get(bridge_id).cloned())
        } else {
            None
        }
        .or_else(|| {
            health_by_bridge
                .values()
                .max_by_key(|health| {
                    [
                        health.last_successful_ping_at_ms,
                        health.last_failure_at_ms,
                        health.last_restart_requested_at_ms,
                    ]
                    .into_iter()
                    .flatten()
                    .max()
                    .unwrap_or(0)
                })
                .cloned()
        });
        let mut summary = current_health
            .unwrap_or_else(|| NativeBridgeHealth {
                supervisor_boot_id: Some(self.supervisor_boot_id.clone()),
                ..NativeBridgeHealth::default()
            })
            .to_json();
        summary["connected_bridge_count"] = json!(connected_bridge_count);
        summary["healthy_bridge_count"] = json!(healthy_bridge_count);
        summary["ambiguous_default_target"] = json!(connected_bridge_count > 1);
        summary["bridge_ids"] = json!(bridge_ids);
        summary["bridges"] = Value::Object(bridges_json);
        summary["current_bridge_id"] = if bridges.len() == 1 {
            bridges
                .keys()
                .next()
                .cloned()
                .map(Value::String)
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        summary["last_registered_bridge_id"] = last_registered_bridge_id
            .map(Value::String)
            .unwrap_or(Value::Null);
        summary
    }

    async fn probe_unknown_browser_bridges(&self, requested_browser: &str, timeout_ms: u64) {
        let bridge_ids = {
            let bridges = self.native_bridges.lock().await;
            bridges
                .values()
                .filter(|bridge| {
                    !bridge_matches_browser_kind(bridge, requested_browser)
                        && !bridge_has_reported_browser_kind(bridge)
                })
                .map(|bridge| bridge.id.clone())
                .collect::<Vec<_>>()
        };
        for bridge_id in bridge_ids {
            let _ = self
                .probe_native_bridge_target(BridgeTarget::BridgeId(bridge_id), timeout_ms)
                .await;
        }
    }

    async fn probe_native_bridge_target(&self, target: BridgeTarget, timeout_ms: u64) -> Value {
        let started_at_ms = now_ms();
        let requested_target = target.clone();
        match self
            .try_call_native_bridge_raw_inner(
                "ping",
                json!({
                    "source": "supervisor.ensure_ready",
                    "timeout_ms": timeout_ms.max(1),
                    "timeout_grace_ms": 500
                }),
                None,
                None,
                None,
                false,
                target,
            )
            .await
        {
            Ok(Some(value)) => {
                let latency_ms = now_ms().saturating_sub(started_at_ms);
                let transport_ok = bridge_probe_transport_ok(&value);
                let required_capabilities = bridge_probe_required_capabilities_map(&value);
                let required_capabilities_ok = required_capabilities
                    .as_object()
                    .map(|map| map.values().all(|value| value.as_bool() == Some(true)))
                    .unwrap_or(false);
                let bridge_contract_version = bridge_probe_contract_version(&value);
                let bridge_contract_version_ok = bridge_probe_contract_version_ok(&value);
                let extension_build_signature = value
                    .pointer("/result/extension_build_signature")
                    .cloned()
                    .unwrap_or(Value::Null);
                let target_match = bridge_probe_target_match(&value, &requested_target);
                let target_error = target_match.error.clone();
                if transport_ok {
                    if let Some(bridge_id) = self.bridge_id_from_ping_response(&value).await {
                        self.record_native_bridge_ping_success(&bridge_id, &value, latency_ms)
                            .await;
                    }
                    if !target_match.ok {
                        let bridge_id = self.bridge_id_from_ping_response(&value).await;
                        self.record_native_bridge_failure(
                            bridge_id.as_deref(),
                            READINESS_CAUSE_BROWSER_TARGET_MISMATCH,
                            target_error
                                .clone()
                                .unwrap_or_else(|| "browser target mismatch".to_string()),
                            false,
                        )
                        .await;
                    }
                } else {
                    let bridge_id = self.current_native_bridge_id().await;
                    self.record_native_bridge_failure(
                        bridge_id.as_deref(),
                        READINESS_CAUSE_SERVICE_WORKER_UNRESPONSIVE,
                        "readiness ping returned without transport success",
                        false,
                    )
                    .await;
                }
                json!({
                    "ok": transport_ok
                        && required_capabilities_ok
                        && bridge_contract_version_ok
                        && target_match.ok,
                    "transport_ok": transport_ok,
                    "required_capabilities": required_capabilities,
                    "required_capabilities_ok": required_capabilities_ok,
                    "target_match_ok": target_match.ok,
                    "requested_browser": target_match.requested_browser.clone(),
                    "loaded_browser": target_match.loaded_browser.clone(),
                    "loaded_browser_hint": target_match.loaded_browser_hint.clone(),
                    "expected_bridge_contract_version": EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION,
                    "bridge_contract_version": bridge_contract_version,
                    "bridge_contract_version_ok": bridge_contract_version_ok,
                    "extension_build_signature": extension_build_signature,
                    "timeout_ms": timeout_ms,
                    "error": if transport_ok && (!required_capabilities_ok || !bridge_contract_version_ok) {
                        json!("loaded extension is missing the current bridge contract/capabilities; reload the extension so the current bridge hardening code is active")
                    } else if transport_ok && !target_match.ok {
                        target_error.map(Value::String).unwrap_or(Value::Null)
                    } else {
                        Value::Null
                    },
                    "response": value
                })
            }
            Ok(None) => {
                let bridge_id = self.current_native_bridge_id().await;
                self.record_native_bridge_failure(
                    bridge_id.as_deref(),
                    READINESS_CAUSE_SERVICE_WORKER_UNRESPONSIVE,
                    "native-host bridge disappeared before ping",
                    false,
                )
                .await;
                json!({
                    "ok": false,
                    "timeout_ms": timeout_ms,
                    "error": "native-host bridge disappeared before ping"
                })
            }
            Err(err) => {
                let error = err.to_string();
                if let Some(error_value) = parse_browser_target_error(&error) {
                    if error_value.get("error_code").and_then(Value::as_str)
                        == Some("AMBIGUOUS_BROWSER_TARGET")
                    {
                        self.record_ambiguous_target_error("ping", &error_value)
                            .await;
                    }
                    let error_code = error_value
                        .get("error_code")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let cause = if error_code.as_deref() == Some("BROWSER_TARGET_MISMATCH") {
                        READINESS_CAUSE_BROWSER_TARGET_MISMATCH
                    } else {
                        READINESS_CAUSE_BROWSER_TARGET_UNRESOLVED
                    };
                    let bridge_id = self.current_native_bridge_id().await;
                    self.record_native_bridge_failure(
                        bridge_id.as_deref(),
                        cause,
                        error.clone(),
                        false,
                    )
                    .await;
                    return json!({
                        "ok": false,
                        "timeout_ms": timeout_ms,
                        "error": error,
                        "error_code": error_code,
                        "target_resolution_error": error_value
                    });
                }

                let cause = if error.to_ascii_lowercase().contains("timeout") {
                    READINESS_CAUSE_ZOMBIE_NATIVE_HOST
                } else {
                    READINESS_CAUSE_TRANSPORT_TIMEOUT
                };
                let bridge_id = self.current_native_bridge_id().await;
                self.record_native_bridge_failure(
                    bridge_id.as_deref(),
                    cause,
                    error.clone(),
                    false,
                )
                .await;
                json!({
                    "ok": false,
                    "timeout_ms": timeout_ms,
                    "error": error
                })
            }
        }
    }

    async fn wait_for_native_bridge(&self, wait_ms: u64) -> bool {
        if self.has_native_bridge().await {
            return true;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms);
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if self.has_native_bridge().await {
                return true;
            }
        }
        false
    }

    fn native_bridge_required_error(
        &self,
        tool_name: &str,
        readiness: Option<&Value>,
    ) -> anyhow::Error {
        if let Some(readiness) = readiness {
            if let Some(message) = readiness
                .pointer("/diagnostic/action_text")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    readiness
                        .pointer("/diagnostic/message")
                        .and_then(Value::as_str)
                        .filter(|value| !value.trim().is_empty())
                })
            {
                return anyhow!(
                    "Browser tool '{}' requires a live native-host bridge, but dispatch-time readiness failed: {}",
                    tool_name,
                    message
                );
            }
        }
        anyhow!(
            "Browser tool '{}' requires the supervisor native-host bridge, but no bridge is connected. Open the target browser with the RZN extension enabled, reload the extension if needed, then retry.",
            tool_name
        )
    }

    async fn runtime_heal(&self, params: Value) -> Result<Value> {
        let heal_bridge_wait_ms = heal_bridge_wait_ms(&params);
        let bridge_probe_timeout_ms = bridge_probe_timeout_ms(&params);
        let readiness_params = |bridge_wait_ms| {
            let mut value = json!({
                "bridge_wait_ms": bridge_wait_ms,
                "bridge_probe_timeout_ms": bridge_probe_timeout_ms
            });
            inherit_browser_target_params(&params, &mut value);
            value
        };
        let initial_readiness = self
            .ensure_ready(readiness_params(HEAL_INITIAL_BRIDGE_WAIT_MS))
            .await?;
        let mut final_readiness = if initial_readiness
            .get("ready")
            .and_then(|value| value.as_bool())
            == Some(true)
        {
            initial_readiness.clone()
        } else {
            self.ensure_ready(readiness_params(heal_bridge_wait_ms))
                .await?
        };
        let post_probe_recovery_attempted = should_retry_heal_after_probe_reset(&final_readiness);
        if post_probe_recovery_attempted {
            final_readiness = self
                .ensure_ready(readiness_params(HEAL_POST_PROBE_RECONNECT_WAIT_MS))
                .await?;
        }
        let mut stability_readiness = Value::Null;
        if readiness_value_ready(&final_readiness) {
            tokio::time::sleep(Duration::from_millis(HEAL_STABILITY_DELAY_MS)).await;
            stability_readiness = self
                .ensure_ready(readiness_params(HEAL_STABILITY_BRIDGE_WAIT_MS))
                .await?;
            final_readiness = stability_readiness.clone();
        }
        let status = self.runtime_status().await;
        let status_bridge_connected = status
            .pointer("/native_host_bridge/connected")
            .and_then(Value::as_bool)
            == Some(true);
        let ready = readiness_value_ready(&final_readiness) && status_bridge_connected;
        Ok(json!({
            "ok": ready,
            "ready": ready,
            "protocol": RZN_LOCAL_PROTOCOL_VERSION,
            "heal_wait_ms": heal_bridge_wait_ms,
            "stability_delay_ms": if stability_readiness.is_null() {
                0
            } else {
                HEAL_STABILITY_DELAY_MS
            },
            "stability_readiness": stability_readiness,
            "post_probe_recovery_attempted": post_probe_recovery_attempted,
            "post_probe_reconnect_wait_ms": if post_probe_recovery_attempted {
                HEAL_POST_PROBE_RECONNECT_WAIT_MS
            } else {
                0
            },
            "readiness": final_readiness,
            "status": status
        }))
    }

    async fn cloud_status(&self) -> Value {
        let native_host_bridge_connected = self.has_native_bridge().await;
        match self.cloud_actor.as_ref() {
            Some(actor) => actor.status(native_host_bridge_connected).await,
            None => supervisor_cloud::disabled_status(native_host_bridge_connected),
        }
    }

    async fn cloud_set_config(&self, params: Value) -> Result<Value> {
        let Some(actor) = self.cloud_actor.as_ref() else {
            return Err(anyhow!(
                "Supervisor cloud actor is not started in this process"
            ));
        };
        let native_host_bridge_connected = self.has_native_bridge().await;
        actor
            .apply_config_value(params, native_host_bridge_connected)
            .await
    }

    async fn cloud_clear_config(&self) -> Result<Value> {
        let Some(actor) = self.cloud_actor.as_ref() else {
            return Err(anyhow!(
                "Supervisor cloud actor is not started in this process"
            ));
        };
        let native_host_bridge_connected = self.has_native_bridge().await;
        actor.clear_config(native_host_bridge_connected).await
    }

    async fn try_dispatch_supervisor_browser_tool(
        &self,
        name: &str,
        params: Value,
    ) -> Result<Option<Value>> {
        if let Some(blocked) = enforce_manifest_side_effect_policy(name, &params) {
            return Ok(Some(blocked));
        }

        let result = match name {
            "browser.session_open" => self.try_session_open(params).await,
            "browser.session_close" => self.try_session_close(params).await,
            "browser.targets" => Ok(Some(self.browser_targets().await)),
            "browser.poll_events" => self.try_poll_events(params).await,
            "browser.snapshot" => {
                self.try_call_native_bridge("get_dom_snapshot", params)
                    .await
            }
            "browser.execute_step" => self.try_execute_step(params).await,
            "browser.static_command" => self.try_static_command(params).await,
            _ => Ok(None),
        };
        match result {
            Err(err) => {
                if let Some(error_value) = parse_browser_target_error(&err.to_string()) {
                    if error_value.get("error_code").and_then(Value::as_str)
                        == Some("AMBIGUOUS_BROWSER_TARGET")
                    {
                        self.record_ambiguous_target_error(name, &error_value).await;
                    }
                    Ok(Some(with_run_result(name, error_value, true)))
                } else {
                    Err(err)
                }
            }
            value => value,
        }
    }

    async fn try_session_open(&self, params: Value) -> Result<Option<Value>> {
        let mut params = params;
        let target = normalized_bridge_target_from_params(&mut params)?;
        let Some(resolved_target) = self.resolve_native_bridge_target(&target, true).await? else {
            return Ok(None);
        };
        let resolved_target_json = resolved_target.to_json();

        let session_id = Uuid::new_v4().to_string();
        let session_record =
            BrowserSessionRecord::from_resolved(session_id.clone(), &resolved_target);
        self.sessions
            .lock()
            .await
            .insert(session_id.clone(), session_record.clone());
        let requested_url = params
            .get("url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Some(url) = requested_url.clone() {
            let payload = json!({
                "session_id": session_id,
                "step": { "type": "navigate_to_url", "url": url }
            });
            if let Some(result) = self.try_call_native_bridge("execute_step", payload).await? {
                let response = json!({
                    "ok": true,
                    "session_id": session_id,
                    "url": requested_url.unwrap_or_default(),
                    "browser_session": session_record.target_json(),
                    "resolved_browser_target": resolved_target_json,
                    "result": result
                });
                return Ok(Some(with_run_result(
                    "browser.session_open",
                    response,
                    true,
                )));
            }
            self.sessions.lock().await.remove(&session_id);
            return Ok(None);
        }

        let response = json!({
            "ok": true,
            "session_id": session_id,
            "url": "",
            "browser_session": session_record.target_json(),
            "resolved_browser_target": resolved_target_json
        });
        Ok(Some(with_run_result(
            "browser.session_open",
            response,
            true,
        )))
    }

    async fn try_session_close(&self, params: Value) -> Result<Option<Value>> {
        let session_id = params
            .get("session_id")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        if session_id.is_empty() {
            return Ok(Some(with_run_result(
                "browser.session_close",
                json!({ "ok": false, "error": "session_id is required" }),
                true,
            )));
        }

        // Best-effort: ask the extension to close the dedicated workflow tab so
        // each `rzn-browser run` invocation cleans up after itself. Failures
        // here (no extension connected, tab already gone) are non-fatal.
        let extension_result = self
            .try_call_native_bridge("session_close", params.clone())
            .await;
        let tab_closed = match &extension_result {
            Ok(Some(value)) => value
                .get("tab_closed")
                .and_then(|v| v.as_bool())
                .or_else(|| {
                    value
                        .pointer("/result/tab_closed")
                        .and_then(|v| v.as_bool())
                })
                .unwrap_or(false),
            _ => false,
        };

        self.sessions.lock().await.remove(&session_id);
        let response = json!({
            "ok": true,
            "session_id": session_id,
            "tab_closed": tab_closed,
        });
        Ok(Some(with_run_result(
            "browser.session_close",
            response,
            true,
        )))
    }

    async fn try_poll_events(&self, params: Value) -> Result<Option<Value>> {
        let session_id = params
            .get("session_id")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        if !session_id.is_empty() {
            if let Some(session) = self.sessions.lock().await.get_mut(&session_id) {
                session.last_activity_at_ms = now_ms();
            }
        }
        let response = json!({
            "ok": true,
            "session_id": session_id,
            "events": []
        });
        Ok(Some(with_run_result("browser.poll_events", response, true)))
    }

    async fn try_execute_step(&self, params: Value) -> Result<Option<Value>> {
        match self.try_call_native_bridge("execute_step", params).await {
            Ok(value) => Ok(value),
            Err(err) if is_native_bridge_transient_dispatch_error(&err.to_string()) => {
                let message = err.to_string();
                Ok(Some(with_run_result(
                    "browser.execute_step",
                    json!({
                        "ok": false,
                        "success": false,
                        "error": message,
                        "error_msg": message,
                        "error_code": native_bridge_transient_dispatch_error_code(&message),
                    }),
                    true,
                )))
            }
            Err(err) => Err(err),
        }
    }

    async fn try_static_command(&self, params: Value) -> Result<Option<Value>> {
        let mut params = params;
        let target = normalized_bridge_target_from_params(&mut params)?;
        let cmd = params
            .get("cmd")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("browser.static_command requires params.cmd"))?;
        if !is_allowed_static_command(cmd) {
            return Err(anyhow!(
                "browser.static_command does not allow cmd '{}'",
                cmd
            ));
        }
        let payload = params.get("payload").cloned().unwrap_or_else(|| json!({}));
        let data = Some(static_command_forward_data(&params));
        let timeout_ms_override = params
            .get("timeout_ms")
            .and_then(|value| value.as_u64())
            .or_else(|| params.get("timeoutMs").and_then(|value| value.as_u64()));
        let req_id_override = params
            .get("req_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let Some(response) = self
            .try_call_native_bridge_raw_with_target(
                cmd,
                payload,
                data,
                timeout_ms_override,
                req_id_override,
                target,
            )
            .await?
        else {
            return Ok(None);
        };

        Ok(Some(with_run_result(cmd, response, true)))
    }

    async fn try_call_native_bridge(&self, cmd: &str, params: Value) -> Result<Option<Value>> {
        let mut params = params;
        let target = normalized_bridge_target_from_params(&mut params)?;
        self.try_call_native_bridge_raw_with_target(cmd, params, None, None, None, target)
            .await
    }

    async fn try_call_native_bridge_without_reconnect(
        &self,
        cmd: &str,
        params: Value,
    ) -> Result<Option<Value>> {
        let mut params = params;
        let target = normalized_bridge_target_from_params(&mut params)?;
        self.try_call_native_bridge_raw_inner(cmd, params, None, None, None, false, target)
            .await
    }

    async fn try_call_native_bridge_raw(
        &self,
        cmd: &str,
        payload: Value,
        data: Option<Value>,
        timeout_ms_override: Option<u64>,
        req_id_override: Option<String>,
    ) -> Result<Option<Value>> {
        let mut payload = payload;
        let target = normalized_bridge_target_from_params(&mut payload)?;
        self.try_call_native_bridge_raw_with_target(
            cmd,
            payload,
            data,
            timeout_ms_override,
            req_id_override,
            target,
        )
        .await
    }

    async fn try_call_native_bridge_raw_with_target(
        &self,
        cmd: &str,
        payload: Value,
        data: Option<Value>,
        timeout_ms_override: Option<u64>,
        req_id_override: Option<String>,
        target: BridgeTarget,
    ) -> Result<Option<Value>> {
        self.try_call_native_bridge_raw_inner(
            cmd,
            payload,
            data,
            timeout_ms_override,
            req_id_override,
            true,
            target,
        )
        .await
    }

    async fn try_call_native_bridge_raw_inner(
        &self,
        cmd: &str,
        payload: Value,
        data: Option<Value>,
        timeout_ms_override: Option<u64>,
        req_id_override: Option<String>,
        allow_reconnect_wait: bool,
        target: BridgeTarget,
    ) -> Result<Option<Value>> {
        let mut payload = payload;
        let target = normalize_tab_ref_input(&mut payload)?.unwrap_or(target);
        self.ensure_session_target_compatible(&payload, &target, allow_reconnect_wait)
            .await?;
        let resolved_target = self
            .resolve_native_bridge_target(&target, allow_reconnect_wait)
            .await?;
        let Some(resolved_target) = resolved_target else {
            return Ok(None);
        };
        self.record_target_resolution_success(&resolved_target, cmd)
            .await;
        let bridge = resolved_target.bridge.clone();
        let resolved_target_json = resolved_target.to_json();
        let mut active_resolved_target_json = resolved_target_json.clone();
        inject_resolved_browser_target(&mut payload, resolved_target_json.clone());
        let mut active_bridge_id = bridge.id.clone();
        let mut active_bridge_epoch = bridge.epoch;

        let id = format!("native-call-{}", Uuid::new_v4());
        let req_id = req_id_override.unwrap_or_else(|| format!("supervisor-{}", Uuid::new_v4()));
        let timeout_ms = timeout_ms_override
            .map(clamp_caller_timeout_ms)
            .unwrap_or_else(|| extension_timeout_ms(&payload));
        let (tx, rx) = oneshot::channel();
        self.native_bridge_pending.lock().await.insert(
            id.clone(),
            PendingNativeCall {
                bridge_id: bridge.id.clone(),
                bridge_epoch: bridge.epoch,
                method: cmd.to_string(),
                deadline_at_ms: 0,
                responder: tx,
            },
        );

        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "native_host.extension_call",
            "params": {
                "cmd": cmd,
                "payload": payload,
                "req_id": req_id,
                "timeout_ms": timeout_ms,
                "supervisor_bridge_id": bridge.id.clone(),
                "supervisor_bridge_epoch": bridge.epoch,
                "supervisor_boot_id": self.supervisor_boot_id.clone(),
                "resolved_browser_target": resolved_target_json
            }
        });
        let mut request = request;
        if let Some(data) = data {
            request["params"]["data"] = data;
        }
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .unwrap_or_default();
        let bytes = serde_json::to_vec(&request)?;
        if bridge.tx.send(bytes).is_err() {
            let pending_call = self.native_bridge_pending.lock().await.remove(&request_id);
            self.record_native_bridge_failure(
                Some(&bridge.id),
                READINESS_CAUSE_BRIDGE_DOWN,
                "native-host bridge sender is closed",
                false,
            )
            .await;
            self.clear_native_bridge(&bridge.id).await;
            if !allow_reconnect_wait {
                return Ok(None);
            }
            if !self
                .wait_for_native_bridge(BRIDGE_RECONNECT_BEFORE_CALL_WAIT_MS)
                .await
            {
                return Ok(None);
            }

            let Some(reconnected_target) = self
                .resolve_native_bridge_target(&target, allow_reconnect_wait)
                .await?
            else {
                return Ok(None);
            };
            let reconnected_target_json = reconnected_target.to_json();
            let reconnected_bridge = reconnected_target.bridge;
            if let Some(pending_call) = pending_call {
                self.native_bridge_pending.lock().await.insert(
                    request_id.clone(),
                    PendingNativeCall {
                        bridge_id: reconnected_bridge.id.clone(),
                        bridge_epoch: reconnected_bridge.epoch,
                        method: pending_call.method,
                        deadline_at_ms: 0,
                        responder: pending_call.responder,
                    },
                );
            } else {
                return Ok(None);
            }
            request["params"]["supervisor_bridge_id"] = json!(reconnected_bridge.id.clone());
            request["params"]["supervisor_bridge_epoch"] = json!(reconnected_bridge.epoch);
            active_resolved_target_json = reconnected_target_json.clone();
            request["params"]["resolved_browser_target"] = reconnected_target_json;
            let resolved_browser_target = request["params"]["resolved_browser_target"].clone();
            inject_resolved_browser_target(
                &mut request["params"]["payload"],
                resolved_browser_target,
            );
            let retry_bytes = serde_json::to_vec(&request)?;
            if reconnected_bridge.tx.send(retry_bytes).is_err() {
                self.native_bridge_pending.lock().await.remove(&request_id);
                self.clear_native_bridge(&reconnected_bridge.id).await;
                return Ok(None);
            }
            if let Some(health) = self
                .native_bridge_health
                .lock()
                .await
                .get_mut(&reconnected_bridge.id)
            {
                health.session_rebind_count = health.session_rebind_count.saturating_add(1);
            }
            log::info!(
                target: "rzn_browser::supervisor",
                "native_host_bridge_rebound request_id={} cmd={} bridge_id={} reason=sender_closed outcome=retry_sent",
                request_id,
                cmd,
                reconnected_bridge.id
            );
            active_bridge_id = reconnected_bridge.id.clone();
            active_bridge_epoch = reconnected_bridge.epoch;
        }
        self.set_native_bridge_pending_deadline(&request_id, now_ms().saturating_add(timeout_ms))
            .await;
        let response = match timeout(Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                self.native_bridge_pending.lock().await.remove(&request_id);
                self.record_native_bridge_failure(
                    Some(&active_bridge_id),
                    READINESS_CAUSE_TRANSPORT_TIMEOUT,
                    "native-host bridge response channel closed",
                    cmd != "ping",
                )
                .await;
                self.request_native_bridge_restart(
                    &active_bridge_id,
                    active_bridge_epoch,
                    "native-host bridge response channel closed",
                )
                .await;
                if cmd != "ping" {
                    return Err(anyhow!("Native-host bridge response channel closed"));
                }
                return Ok(None);
            }
            Err(_) => {
                self.native_bridge_pending.lock().await.remove(&request_id);
                let reason = format!("extension call '{}' timed out after {}ms", cmd, timeout_ms);
                self.record_native_bridge_failure(
                    Some(&active_bridge_id),
                    READINESS_CAUSE_ZOMBIE_NATIVE_HOST,
                    reason.clone(),
                    cmd != "ping",
                )
                .await;
                self.request_native_bridge_restart(&active_bridge_id, active_bridge_epoch, &reason)
                    .await;
                return Err(anyhow!(
                    "Native-host extension bridge timeout after {}ms",
                    timeout_ms
                ));
            }
        };

        if let Some(error) = response.get("error") {
            let error_message = native_host_bridge_error_message(error);
            if let Some(cause) = native_host_bridge_transport_error_cause(error) {
                let step_timeout = cmd != "ping" && cause == READINESS_CAUSE_ZOMBIE_NATIVE_HOST;
                self.record_native_bridge_failure(
                    Some(&active_bridge_id),
                    cause,
                    error_message.clone(),
                    step_timeout,
                )
                .await;
                self.request_native_bridge_restart(
                    &active_bridge_id,
                    active_bridge_epoch,
                    &error_message,
                )
                .await;
            }
            return Err(anyhow!("Native-host extension bridge error: {}", error));
        }
        let mut result = response.get("result").cloned().unwrap_or(response);
        inject_resolved_browser_target(&mut result, active_resolved_target_json);
        inject_tab_ref_from_browser_target(&mut result);
        inherit_run_context_from_params(&mut result, &payload);
        Ok(Some(with_run_result(cmd, result, true)))
    }

    #[cfg(test)]
    async fn register_native_bridge(&self, id: String, tx: mpsc::UnboundedSender<Vec<u8>>) -> u64 {
        self.register_native_bridge_with_metadata(id, tx, NativeHostBridgeMetadata::missing())
            .await
    }

    async fn register_native_bridge_with_metadata(
        &self,
        id: String,
        tx: mpsc::UnboundedSender<Vec<u8>>,
        metadata: NativeHostBridgeMetadata,
    ) -> u64 {
        let registered_at_ms = now_ms();
        let epoch = self
            .bridge_epoch_counter
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        let bridge = NativeHostBridge {
            id: id.clone(),
            epoch,
            tx,
            registered_at_ms,
            metadata: metadata.clone(),
        };
        {
            let mut guard = self.native_bridges.lock().await;
            guard.insert(id.clone(), bridge.clone());
        }
        let mut health =
            NativeBridgeHealth::new_for_bridge(self.supervisor_boot_id.clone(), &bridge, metadata);
        health.bridge_registration_count = health.bridge_registration_count.saturating_add(1);
        self.native_bridge_health
            .lock()
            .await
            .insert(id.clone(), health);
        *self.last_registered_bridge_id.lock().await = Some(id);
        log::info!(
            target: "rzn_browser::supervisor",
            "native_host_bridge_registered bridge_id={} epoch={} browser_instance_id={} extension_id={} caller_origin_status={}",
            bridge.id,
            bridge.epoch,
            bridge.metadata.browser_instance_id.as_deref().unwrap_or(""),
            bridge.metadata
                .extension_reported_id
                .as_deref()
                .or(bridge.metadata.caller_extension_id.as_deref())
                .unwrap_or(""),
            bridge.metadata.caller_origin_status
        );
        epoch
    }

    async fn clear_native_bridge(&self, id: &str) {
        let replacement = {
            let mut guard = self.native_bridges.lock().await;
            if guard.remove(id).is_none() {
                None
            } else {
                guard
                    .values()
                    .max_by_key(|bridge| (bridge.registered_at_ms, bridge.epoch))
                    .cloned()
            }
        };

        self.drain_native_bridge_pending(id).await;
        self.mark_sessions_for_disconnected_bridge(id).await;
        if let Some(health) = self.native_bridge_health.lock().await.get_mut(id) {
            health.bridge_unregistration_count =
                health.bridge_unregistration_count.saturating_add(1);
        }
        log::info!(
            target: "rzn_browser::supervisor",
            "native_host_bridge_unregistered bridge_id={}",
            id
        );
        let mut last_registered_bridge_id = self.last_registered_bridge_id.lock().await;
        if last_registered_bridge_id.as_deref() == Some(id) {
            *last_registered_bridge_id = replacement.map(|bridge| bridge.id);
        }
    }

    async fn record_target_resolution_success(&self, resolved: &ResolvedBridgeTarget, cmd: &str) {
        if let Some(health) = self
            .native_bridge_health
            .lock()
            .await
            .get_mut(&resolved.bridge.id)
        {
            health.target_resolution_count = health.target_resolution_count.saturating_add(1);
        }
        log::debug!(
            target: "rzn_browser::supervisor",
            "browser_target_resolved cmd={} bridge_id={} source={} browser_instance_id={}",
            cmd,
            resolved.bridge.id,
            resolved.source,
            resolved
                .bridge
                .metadata
                .browser_instance_id
                .as_deref()
                .unwrap_or("")
        );
    }

    async fn record_ambiguous_target_error(&self, cmd: &str, error_value: &Value) {
        let candidate_count = error_value
            .get("candidates")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        for bridge_id in error_value
            .get("candidates")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|candidate| candidate.get("bridge_id").and_then(Value::as_str))
        {
            if let Some(health) = self.native_bridge_health.lock().await.get_mut(bridge_id) {
                health.ambiguous_target_count = health.ambiguous_target_count.saturating_add(1);
            }
        }
        log::warn!(
            target: "rzn_browser::supervisor",
            "browser_target_ambiguous cmd={} candidate_count={}",
            cmd,
            candidate_count
        );
    }

    async fn drain_native_bridge_pending(&self, bridge_id: &str) -> usize {
        let drained = {
            let mut guard = self.native_bridge_pending.lock().await;
            let ids = guard
                .iter()
                .filter_map(|(id, pending)| {
                    if pending.bridge_id == bridge_id {
                        Some(id.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            let mut drained = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(pending) = guard.remove(&id) {
                    drained.push((id, pending));
                }
            }
            drained
        };
        let drained_len = drained.len();
        for (id, pending) in drained {
            let _ = pending.responder.send(native_bridge_disconnected_response(
                &id,
                bridge_id,
                &pending.method,
                pending.deadline_at_ms,
            ));
        }
        drained_len
    }

    async fn request_native_bridge_restart(&self, id: &str, epoch: u64, reason: &str) {
        let bridge = {
            let guard = self.native_bridges.lock().await;
            guard
                .get(id)
                .filter(|bridge| bridge.epoch == epoch)
                .cloned()
        };

        let Some(bridge) = bridge else {
            return;
        };

        let request = json!({
            "jsonrpc": "2.0",
            "id": format!("native-host-shutdown-{}", Uuid::new_v4()),
            "method": NATIVE_HOST_SHUTDOWN_METHOD,
            "params": {
                "reason": reason,
                "bridge_id": id
            }
        });
        if let Ok(bytes) = serde_json::to_vec(&request) {
            let _ = bridge.tx.send(bytes);
        }

        {
            let mut health_by_bridge = self.native_bridge_health.lock().await;
            let health = health_by_bridge.entry(id.to_string()).or_insert_with(|| {
                let mut health = NativeBridgeHealth {
                    supervisor_boot_id: Some(self.supervisor_boot_id.clone()),
                    current_bridge_id: Some(id.to_string()),
                    current_supervisor_bridge_epoch: Some(epoch),
                    ..NativeBridgeHealth::default()
                };
                health.current_bridge_registered_at_ms = Some(bridge.registered_at_ms);
                health.current_bridge_metadata = Some(bridge.metadata.clone());
                health
            });
            health.last_restart_requested_at_ms = Some(now_ms());
            health.last_restart_reason = Some(reason.to_string());
            health.native_host_restart_count = health.native_host_restart_count.saturating_add(1);
        }
        self.clear_native_bridge(id).await;
    }

    async fn record_native_bridge_ping_success(
        &self,
        bridge_id: &str,
        ping_response: &Value,
        latency_ms: u64,
    ) {
        let bridge = self.native_bridges.lock().await.get(bridge_id).cloned();
        let mut health_by_bridge = self.native_bridge_health.lock().await;
        let health = health_by_bridge
            .entry(bridge_id.to_string())
            .or_insert_with(|| {
                if let Some(bridge) = bridge.as_ref() {
                    NativeBridgeHealth::new_for_bridge(
                        self.supervisor_boot_id.clone(),
                        bridge,
                        bridge.metadata.clone(),
                    )
                } else {
                    NativeBridgeHealth {
                        supervisor_boot_id: Some(self.supervisor_boot_id.clone()),
                        current_bridge_id: Some(bridge_id.to_string()),
                        ..NativeBridgeHealth::default()
                    }
                }
            });
        health.last_successful_ping_at_ms = Some(now_ms());
        health.last_successful_ping_latency_ms = Some(latency_ms);
        health.last_successful_extension_build_signature = Some(
            ping_response
                .pointer("/result/extension_build_signature")
                .cloned()
                .unwrap_or(Value::Null),
        );
        if let Some(metadata) = health.current_bridge_metadata.as_mut() {
            metadata.update_extension_reported_metadata(ping_response);
        }
        health.last_successful_bridge_contract_version = ping_response
            .pointer("/result/bridge_contract_version")
            .and_then(Value::as_u64);
        health.last_successful_supervisor_bridge_epoch = ping_response
            .pointer("/result/supervisor_bridge_epoch")
            .and_then(Value::as_u64);
        health.last_successful_native_host_pid = ping_response
            .pointer("/result/native_host_pid")
            .and_then(Value::as_u64);
        health.last_successful_native_host_boot_id = ping_response
            .pointer("/result/native_host_boot_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        health.last_successful_extension_worker_boot_id = ping_response
            .pointer("/result/extension_worker_boot_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        health.last_successful_native_port_epoch = ping_response
            .pointer("/result/native_port_epoch")
            .and_then(Value::as_u64);
        health.last_successful_stdout_heartbeat_ms = ping_response
            .pointer("/result/last_native_host_stdout_heartbeat_ms")
            .and_then(Value::as_u64);
        health.last_successful_stdout_heartbeat_age_ms = ping_response
            .pointer("/result/last_native_host_stdout_heartbeat_age_ms")
            .and_then(Value::as_u64);
        health.last_successful_stdout_heartbeat_seq = ping_response
            .pointer("/result/last_native_host_stdout_heartbeat_seq")
            .and_then(Value::as_u64);
        health.last_successful_roundtrip_ping_ms = ping_response
            .pointer("/result/last_native_roundtrip_ping_ms")
            .and_then(Value::as_u64);
        health.last_successful_roundtrip_ping_age_ms = ping_response
            .pointer("/result/last_native_roundtrip_ping_age_ms")
            .and_then(Value::as_u64);
        health.missed_roundtrip_count = ping_response
            .pointer("/result/missed_native_roundtrip_pings")
            .and_then(Value::as_u64);
        let bridge_metadata_update = health.current_bridge_metadata.clone();
        drop(health_by_bridge);

        if let Some(metadata) = bridge_metadata_update {
            let mut bridges = self.native_bridges.lock().await;
            if let Some(bridge) = bridges.get_mut(bridge_id) {
                bridge.metadata = metadata;
            }
        }
    }

    async fn record_native_bridge_failure(
        &self,
        bridge_id: Option<&str>,
        cause: &str,
        error: impl Into<String>,
        step_timeout: bool,
    ) {
        let bridge_id = match bridge_id.map(str::to_string) {
            Some(bridge_id) => Some(bridge_id),
            None => self.current_native_bridge_id().await,
        };
        let Some(bridge_id) = bridge_id else {
            return;
        };
        let bridge = self.native_bridges.lock().await.get(&bridge_id).cloned();
        let mut health_by_bridge = self.native_bridge_health.lock().await;
        let health = health_by_bridge
            .entry(bridge_id.clone())
            .or_insert_with(|| {
                if let Some(bridge) = bridge.as_ref() {
                    NativeBridgeHealth::new_for_bridge(
                        self.supervisor_boot_id.clone(),
                        bridge,
                        bridge.metadata.clone(),
                    )
                } else {
                    NativeBridgeHealth {
                        supervisor_boot_id: Some(self.supervisor_boot_id.clone()),
                        current_bridge_id: Some(bridge_id),
                        ..NativeBridgeHealth::default()
                    }
                }
            });
        health.last_failure_at_ms = Some(now_ms());
        health.last_failure_cause = Some(cause.to_string());
        health.last_failure_error = Some(error.into());
        if cause == READINESS_CAUSE_TRANSPORT_TIMEOUT || cause == READINESS_CAUSE_ZOMBIE_NATIVE_HOST
        {
            health.timeout_count = health.timeout_count.saturating_add(1);
        }
        if step_timeout {
            health.last_step_timeout_at_ms = Some(now_ms());
        }
    }

    async fn set_native_bridge_pending_deadline(&self, request_id: &str, deadline_at_ms: u64) {
        if let Some(pending) = self.native_bridge_pending.lock().await.get_mut(request_id) {
            pending.deadline_at_ms = deadline_at_ms;
        }
    }

    async fn complete_native_bridge_response(
        &self,
        bridge_id: &str,
        bridge_epoch: u64,
        value: &Value,
    ) -> bool {
        let Some(id) = value.get("id").and_then(value_id_string) else {
            return false;
        };
        let (pending, mismatch_bridge_id) = {
            let mut guard = self.native_bridge_pending.lock().await;
            match guard.get(&id) {
                Some(pending)
                    if pending.bridge_id == bridge_id && pending.bridge_epoch == bridge_epoch =>
                {
                    (guard.remove(&id), None)
                }
                Some(pending) => (None, Some(pending.bridge_id.clone())),
                None => (None, None),
            }
        };
        if let Some(pending) = pending {
            let _ = pending.responder.send(value.clone());
            return true;
        }

        let health_bridge_id = mismatch_bridge_id
            .as_deref()
            .or(Some(bridge_id))
            .unwrap_or(bridge_id)
            .to_string();
        let bridge = self
            .native_bridges
            .lock()
            .await
            .get(&health_bridge_id)
            .cloned();
        let mut health_by_bridge = self.native_bridge_health.lock().await;
        let health = health_by_bridge
            .entry(health_bridge_id.clone())
            .or_insert_with(|| {
                if let Some(bridge) = bridge.as_ref() {
                    NativeBridgeHealth::new_for_bridge(
                        self.supervisor_boot_id.clone(),
                        bridge,
                        bridge.metadata.clone(),
                    )
                } else {
                    NativeBridgeHealth {
                        supervisor_boot_id: Some(self.supervisor_boot_id.clone()),
                        current_bridge_id: Some(health_bridge_id),
                        ..NativeBridgeHealth::default()
                    }
                }
            });
        if mismatch_bridge_id.is_some() {
            health.bridge_response_mismatch_drop_count =
                health.bridge_response_mismatch_drop_count.saturating_add(1);
            log::warn!(
                target: "rzn_browser::supervisor",
                "native_host_bridge_response_mismatch request_id={} expected_bridge_id={} actual_bridge_id={} actual_epoch={}",
                id,
                mismatch_bridge_id.as_deref().unwrap_or(""),
                bridge_id,
                bridge_epoch
            );
        } else {
            health.stale_bridge_response_drop_count =
                health.stale_bridge_response_drop_count.saturating_add(1);
            log::warn!(
                target: "rzn_browser::supervisor",
                "native_host_bridge_response_stale request_id={} actual_bridge_id={} actual_epoch={}",
                id,
                bridge_id,
                bridge_epoch
            );
        }
        false
    }

    async fn dispatch_cloud_command_to_extension(
        &self,
        envelope: &CloudCommandEnvelopeV1,
        default_request_timeout_ms: u64,
    ) -> CloudCommandResultV1 {
        match self
            .dispatch_cloud_command_to_extension_inner(envelope, default_request_timeout_ms)
            .await
        {
            Ok(result) => result,
            Err(error) => cloud_error_command_result(envelope, error.to_string()),
        }
    }
    async fn dispatch_cloud_command_to_extension_inner(
        &self,
        envelope: &CloudCommandEnvelopeV1,
        default_request_timeout_ms: u64,
    ) -> Result<CloudCommandResultV1> {
        let command = envelope
            .payload
            .command
            .as_ref()
            .ok_or_else(|| anyhow!("browser_command payload missing command object"))?;
        let payload = cloud_command_payload(envelope, command);
        let timeout_ms = supervisor_cloud::compute_request_timeout_ms(
            envelope.deadline_ms,
            default_request_timeout_ms,
            EXTENSION_BRIDGE_GRACE_MS,
        );
        let Some(response) = self
            .try_call_native_bridge_raw(
                command.cmd.as_str(),
                payload,
                command.data.clone(),
                Some(timeout_ms),
                Some(envelope.command_id.clone()),
            )
            .await?
        else {
            return Err(anyhow!(
                "Native-host bridge is not connected for cloud command"
            ));
        };
        Ok(cloud_command_result_from_response(envelope, response))
    }
}

fn cloud_command_payload(
    envelope: &CloudCommandEnvelopeV1,
    command: &CloudBrowserCommandV1,
) -> Value {
    let mut payload = command.payload.clone().unwrap_or_else(|| json!({}));
    if let Some(obj) = payload.as_object_mut() {
        obj.entry("session_id".to_string())
            .or_insert_with(|| Value::String(envelope.session_id.clone()));
    } else {
        payload = json!({
            "session_id": envelope.session_id.clone(),
            "value": payload
        });
    }
    payload
}

fn cloud_command_result_from_response(
    envelope: &CloudCommandEnvelopeV1,
    response: Value,
) -> CloudCommandResultV1 {
    let success = response_success(&response)
        || response
            .pointer("/result/success")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
    let error_message = response
        .get("error_msg")
        .and_then(|value| value.as_str())
        .or_else(|| response.get("error").and_then(|value| value.as_str()))
        .or_else(|| {
            response
                .pointer("/error/message")
                .and_then(|value| value.as_str())
        })
        .map(str::to_string);
    let capabilities = response
        .get("capabilities")
        .or_else(|| response.pointer("/result/capabilities"))
        .cloned()
        .and_then(|value| serde_json::from_value::<CapabilitiesV1>(value).ok());
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
        result: Some(ActionResultV1 {
            success,
            error_code: response
                .get("error_code")
                .or_else(|| response.pointer("/error/code"))
                .and_then(|value| value.as_str())
                .map(str::to_string),
            error: error_message.clone(),
            current_url: response
                .get("current_url")
                .or_else(|| response.pointer("/result/current_url"))
                .and_then(|value| value.as_str())
                .map(str::to_string),
            current_tab_id: response
                .get("current_tab_id")
                .or_else(|| response.pointer("/result/current_tab_id"))
                .and_then(|value| value.as_u64())
                .and_then(|value| u32::try_from(value).ok()),
            current_tab_ref: current_tab_ref_from_response(&response),
            dom_hash: response
                .get("dom_hash")
                .or_else(|| response.pointer("/result/dom_hash"))
                .and_then(|value| value.as_str())
                .map(str::to_string),
            dom_snapshot: None,
            capabilities,
            raw: Some(response),
        }),
        error: error_message,
    }
}

fn current_tab_ref_from_response(response: &Value) -> Option<String> {
    response
        .get("current_tab_ref")
        .or_else(|| response.get("tab_ref"))
        .or_else(|| response.pointer("/result/current_tab_ref"))
        .or_else(|| response.pointer("/result/tab_ref"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn cloud_error_command_result(
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

pub(crate) fn run_result_for_tool(tool_name: &str, response: &Value) -> Value {
    response
        .get("run_result")
        .filter(|value| is_run_result_v2(value))
        .cloned()
        .unwrap_or_else(|| build_run_result_value(tool_name, response, true))
}

fn with_run_result(tool_name: &str, mut response: Value, include_debug: bool) -> Value {
    let run_result = build_run_result_value(tool_name, &response, include_debug);
    if let Value::Object(map) = &mut response {
        if !map.get("run_result").is_some_and(is_run_result_v2) {
            map.insert("run_result".to_string(), run_result);
        }
    }
    response
}

fn is_run_result_v2(value: &Value) -> bool {
    value.get("version").and_then(Value::as_str) == Some(RUN_RESULT_VERSION)
}

fn build_run_result_value(tool_name: &str, response: &Value, include_debug: bool) -> Value {
    serde_json::to_value(build_run_result(tool_name, response, include_debug))
        .expect("RunResultV2 serializes")
}

fn build_run_result(tool_name: &str, response: &Value, include_debug: bool) -> RunResultV2 {
    let success = response_success(response);
    let status = run_status(response, success);
    let mut warnings = collect_run_warnings(response);
    if response.get("success").is_none() && response.get("ok").is_none() {
        warnings.push(RunWarningV1 {
            code: "legacy_success_missing".to_string(),
            message:
                "Supervisor response did not include success or ok; run result treats it as failed."
                    .to_string(),
            step_id: None,
        });
    }

    RunResultV2 {
        version: RUN_RESULT_VERSION.to_string(),
        run_id: response
            .get("run_id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("local-{}", Uuid::new_v4())),
        workflow_id: response_workflow_id(response)
            .unwrap_or_else(|| local_tool_workflow_id(tool_name)),
        status: status.clone(),
        output: select_standard_output(response),
        artifacts: collect_contract_artifacts(response),
        warnings,
        steps: Vec::new(),
        debug: include_debug.then(|| DebugBundleV1 {
            trace_id: response
                .get("trace_id")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            events: vec![DebugEventV1 {
                at_ms: now_ms(),
                message: "supervisor_native_host_bridge".to_string(),
                step_id: None,
                data: None,
            }],
            raw: Some(response.clone()),
        }),
        error: run_error(response, &status),
    }
}

fn response_workflow_id(response: &Value) -> Option<String> {
    response
        .get("workflow_id")
        .or_else(|| response.get("workflow"))
        .and_then(|value| value.as_str())
        .or_else(|| {
            response
                .pointer("/workflow/id")
                .and_then(|value| value.as_str())
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn inherit_run_context_from_params(response: &mut Value, params: &Value) {
    let Some(map) = response.as_object_mut() else {
        return;
    };

    for key in ["workflow_id", "workflow_version", "system", "capability"] {
        if let Some(value) = params.get(key).cloned() {
            map.entry(key.to_string()).or_insert(value);
        }
    }
}

fn local_tool_workflow_id(tool_name: &str) -> String {
    format!("rzn.local.{}", tool_name.replace('/', "."))
}

fn run_status(response: &Value, success: bool) -> RunStatusV2 {
    if success {
        return RunStatusV2::Succeeded;
    }
    match response
        .get("error_code")
        .or_else(|| response.pointer("/error/code"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
    {
        "POLICY_SIDE_EFFECT_UNDECLARED" => RunStatusV2::PolicyBlocked,
        "TIMEOUT" | "TIMED_OUT" | "EXTENSION_BRIDGE_TIMEOUT" => RunStatusV2::TimedOut,
        "CANCELLED" | "CANCELED" => RunStatusV2::Cancelled,
        _ => RunStatusV2::Failed,
    }
}

fn response_success(response: &Value) -> bool {
    response
        .get("success")
        .and_then(|value| value.as_bool())
        .or_else(|| response.get("ok").and_then(|value| value.as_bool()))
        .unwrap_or(false)
}

fn select_standard_output(response: &Value) -> Option<Value> {
    if let Some(output) = response
        .get("result")
        .cloned()
        .or_else(|| response.get("payload").cloned())
        .or_else(|| response.get("data").cloned())
    {
        return Some(output);
    }

    let Value::Object(map) = response else {
        return Some(response.clone());
    };

    let mut output = map.clone();
    for key in [
        "ok",
        "success",
        "run_result",
        "run_envelope",
        "warnings",
        "artifacts",
        "downloads",
        "error",
        "error_code",
    ] {
        output.remove(key);
    }
    if output.is_empty() {
        None
    } else {
        Some(Value::Object(output))
    }
}

fn run_error(response: &Value, status: &RunStatusV2) -> Option<RunErrorV1> {
    if *status == RunStatusV2::Succeeded {
        return None;
    }
    let error_value = response.get("error");
    let message = error_value
        .and_then(|value| value.as_str())
        .or_else(|| {
            error_value
                .and_then(|value| value.get("message"))
                .and_then(|value| value.as_str())
        })
        .or_else(|| response.get("error_msg").and_then(|value| value.as_str()))
        .unwrap_or("Run failed")
        .to_string();
    Some(RunErrorV1 {
        code: response
            .get("error_code")
            .or_else(|| error_value.and_then(|value| value.get("code")))
            .and_then(|value| value.as_str())
            .unwrap_or("RUN_FAILED")
            .to_string(),
        message,
        step_id: error_value
            .and_then(|value| value.get("step_id"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
        retry_hint: error_value
            .and_then(|value| value.get("retry_hint"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
    })
}

fn collect_response_array(response: &Value, key: &str) -> Vec<Value> {
    response
        .get(key)
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default()
}

fn collect_run_warnings(response: &Value) -> Vec<RunWarningV1> {
    collect_response_array(response, "warnings")
        .into_iter()
        .enumerate()
        .map(|(index, warning)| {
            if let Some(message) = warning.as_str() {
                return RunWarningV1 {
                    code: format!("warning_{index}"),
                    message: message.to_string(),
                    step_id: None,
                };
            }
            RunWarningV1 {
                code: warning
                    .get("code")
                    .and_then(|value| value.as_str())
                    .unwrap_or("warning")
                    .to_string(),
                message: warning
                    .get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or("warning")
                    .to_string(),
                step_id: warning
                    .get("step_id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            }
        })
        .collect()
}

fn collect_contract_artifacts(response: &Value) -> Vec<ArtifactV1> {
    let mut artifacts = Vec::new();
    if let Some(raw_artifacts) = response.get("artifacts").and_then(|value| value.as_array()) {
        for raw in raw_artifacts {
            artifacts.push(contract_artifact(
                raw,
                artifacts.len(),
                ArtifactKindV1::Json,
            ));
        }
    }
    if let Some(downloads) = response.get("downloads").and_then(|value| value.as_array()) {
        for download in downloads {
            artifacts.push(contract_artifact(
                download,
                artifacts.len(),
                ArtifactKindV1::Download,
            ));
        }
    }
    artifacts
}

fn contract_artifact(raw: &Value, index: usize, default_kind: ArtifactKindV1) -> ArtifactV1 {
    ArtifactV1 {
        id: raw
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("artifact-{index}")),
        kind: raw
            .get("kind")
            .and_then(|value| value.as_str())
            .and_then(parse_artifact_kind)
            .unwrap_or(default_kind),
        uri: artifact_uri(raw).unwrap_or_else(|| format!("inline:artifact-{index}")),
        label: raw
            .get("label")
            .or_else(|| raw.get("name"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
        media_type: raw
            .get("media_type")
            .or_else(|| raw.get("mime_type"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
        byte_count: raw
            .get("byte_count")
            .or_else(|| raw.get("size"))
            .and_then(|value| value.as_u64()),
        sha256: raw
            .get("sha256")
            .and_then(|value| value.as_str())
            .map(str::to_string),
    }
}

fn artifact_uri(raw: &Value) -> Option<String> {
    raw.as_str()
        .map(str::to_string)
        .or_else(|| {
            raw.get("uri")
                .or_else(|| raw.get("url"))
                .or_else(|| raw.get("path"))
                .or_else(|| raw.get("file"))
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .or_else(|| Some(format!("inline:{}", raw)))
}

fn parse_artifact_kind(kind: &str) -> Option<ArtifactKindV1> {
    match kind {
        "file" => Some(ArtifactKindV1::File),
        "download" => Some(ArtifactKindV1::Download),
        "screenshot" => Some(ArtifactKindV1::Screenshot),
        "json" => Some(ArtifactKindV1::Json),
        "text" => Some(ArtifactKindV1::Text),
        _ => None,
    }
}

fn enforce_manifest_side_effect_policy(tool_name: &str, params: &Value) -> Option<Value> {
    let policy = params
        .get("side_effect_policy")
        .or_else(|| params.get("manifest_policy"))
        .or_else(|| params.get("policy"))?;

    let enforce = policy
        .get("enforce")
        .or_else(|| policy.get("strict"))
        .or_else(|| policy.get("strict_side_effects"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !enforce {
        return None;
    }

    let declared = side_effect_set_from_array(
        policy
            .get("declared_side_effects")
            .or_else(|| policy.get("declared")),
    );
    let observed = observed_side_effects(tool_name, params);
    let undeclared: Vec<String> = observed
        .iter()
        .filter(|effect| !declared.contains(*effect))
        .cloned()
        .collect();
    if undeclared.is_empty() {
        return None;
    }

    let mut response = json!({
        "ok": false,
        "success": false,
        "error": format!(
            "Manifest side-effect policy blocked {}: undeclared effects [{}]",
            tool_name,
            undeclared.join(", ")
        ),
        "error_code": "POLICY_SIDE_EFFECT_UNDECLARED",
        "policy": {
            "enforced": true,
            "declared_side_effects": sorted_strings(declared),
            "observed_side_effects": observed,
            "undeclared_side_effects": undeclared
        }
    });
    inherit_run_context_from_params(&mut response, params);
    Some(with_run_result(tool_name, response, true))
}

fn observed_side_effects(tool_name: &str, params: &Value) -> Vec<String> {
    let mut effects = HashSet::new();
    match tool_name {
        "browser.session_open" | "browser.session_close" => {
            insert_side_effect(&mut effects, SideEffectClassV2::BrowserState);
        }
        "browser.snapshot" | "browser.poll_events" => {
            insert_side_effect(&mut effects, SideEffectClassV2::ReadOnly);
        }
        "browser.execute_step" => {
            if let Some(explicit) = params.get("step").and_then(|step| step.get("side_effects")) {
                for effect in side_effect_set_from_array(Some(explicit)) {
                    effects.insert(effect);
                }
            }
            let step_type = params
                .get("step")
                .and_then(|step| step.get("type"))
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            if effects.is_empty() {
                for effect in side_effects_for_step_type(step_type) {
                    insert_side_effect(&mut effects, *effect);
                }
            }
        }
        "browser.static_command" => {
            let cmd = params
                .get("cmd")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            for effect in side_effects_for_static_command(cmd) {
                insert_side_effect(&mut effects, *effect);
            }
        }
        _ => {}
    }
    let mut effects: Vec<String> = effects.into_iter().collect();
    effects.sort();
    effects
}

fn insert_side_effect(effects: &mut HashSet<String>, class: SideEffectClassV2) {
    effects.insert(class.as_str().to_string());
}

fn side_effects_for_step_type(step_type: &str) -> &'static [SideEffectClassV2] {
    match step_type {
        "get_page_source"
        | "get_element_text"
        | "wait"
        | "wait_for_element"
        | "wait_for_timeout"
        | "extract"
        | "extract_structured_data"
        | "assert_selector_state"
        | "take_screenshot" => &[SideEffectClassV2::ReadOnly],
        "same_origin_request" => &[
            SideEffectClassV2::ReadOnly,
            SideEffectClassV2::ExternalRead,
            SideEffectClassV2::NetworkAccess,
        ],
        "navigate"
        | "navigate_to_url"
        | "click"
        | "click_element"
        | "fill_input_field"
        | "type_text"
        | "press_key"
        | "press_special_key"
        | "scroll"
        | "scroll_element_into_view"
        | "scroll_window_to"
        | "select_option"
        | "dismiss_popups"
        | "infinite_scroll"
        | "request_user_intervention"
        | "open_new_tab"
        | "close_current_tab"
        | "execute_javascript"
        | "javascript"
        | "eval" => &[SideEffectClassV2::BrowserState],
        "upload_file" => &[
            SideEffectClassV2::BrowserState,
            SideEffectClassV2::ExternalWrite,
        ],
        "download_file" | "download" | "download_images" => &[
            SideEffectClassV2::BrowserState,
            SideEffectClassV2::Download,
            SideEffectClassV2::ExternalRead,
            SideEffectClassV2::FileWrite,
            SideEffectClassV2::NetworkAccess,
        ],
        "submit_input" => &[SideEffectClassV2::BrowserState],
        _ => &[SideEffectClassV2::BrowserState],
    }
}

fn side_effects_for_static_command(cmd: &str) -> &'static [SideEffectClassV2] {
    match cmd {
        "get_dom_snapshot"
        | "get_dom_hash"
        | "process_dom"
        | "detect_auto_list"
        | "execute_extraction_plan"
        | "observe" => &[SideEffectClassV2::ReadOnly],
        "get_cdp_context" | "get_ax_tree" | "get_interactive_elements" => {
            &[SideEffectClassV2::BrowserState]
        }
        "cdp_action" | "set_flags" | "enable_debug" | "disable_debug" => {
            &[SideEffectClassV2::BrowserState]
        }
        _ => &[],
    }
}

fn is_allowed_static_command(cmd: &str) -> bool {
    matches!(
        cmd,
        "get_dom_snapshot"
            | "get_dom_hash"
            | "process_dom"
            | "detect_auto_list"
            | "execute_extraction_plan"
            | "observe"
            | "get_cdp_context"
            | "get_ax_tree"
            | "get_interactive_elements"
            | "cdp_action"
            | "set_flags"
            | "enable_debug"
            | "disable_debug"
    )
}

fn static_command_forward_data(params: &Value) -> Value {
    let mut data = params.get("data").cloned().unwrap_or_else(|| json!({}));
    if !data.is_object() {
        data = json!({});
    }

    let Some(data_obj) = data.as_object_mut() else {
        return data;
    };

    for key in [
        "session_id",
        "current_tab_id",
        "use_current_tab",
        "use_active_tab",
    ] {
        if let Some(value) = params.get(key).cloned() {
            data_obj.entry(key.to_string()).or_insert(value);
        }
    }

    if let Some(payload_obj) = params.get("payload").and_then(|value| value.as_object()) {
        for key in [
            "session_id",
            "current_tab_id",
            "use_current_tab",
            "use_active_tab",
        ] {
            if let Some(value) = payload_obj.get(key).cloned() {
                data_obj.entry(key.to_string()).or_insert(value);
            }
        }
    }

    data
}

fn side_effect_set_from_array(value: Option<&Value>) -> HashSet<String> {
    value
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<SideEffectClassV2>(item.clone()).ok())
                .map(SideEffectClassV2::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn sorted_strings(values: HashSet<String>) -> Vec<String> {
    let mut values: Vec<String> = values.into_iter().collect();
    values.sort();
    values
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug)]
struct SupervisorProcessLock {
    path: PathBuf,
}

impl Drop for SupervisorProcessLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn acquire_supervisor_process_lock(paths: &SupervisorPaths) -> Result<SupervisorProcessLock> {
    for _ in 0..2 {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&paths.lock_path)
        {
            Ok(mut file) => {
                let payload = json!({
                    "pid": std::process::id(),
                    "started_at_ms": now_ms(),
                    "socket_path": paths.socket_path.to_string_lossy(),
                    "app_base": paths.app_base.to_string_lossy()
                });
                file.write_all(serde_json::to_string(&payload)?.as_bytes())?;
                file.write_all(b"\n")?;
                let _ = file.sync_all();
                return Ok(SupervisorProcessLock {
                    path: paths.lock_path.clone(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if let Some(pid) = read_supervisor_lock_pid(&paths.lock_path) {
                    if process_is_live(pid) {
                        return Err(anyhow!(
                            "Supervisor already running or starting for app base {} (pid {}, lock {})",
                            paths.app_base.display(),
                            pid,
                            paths.lock_path.display()
                        ));
                    }
                }
                let _ = std::fs::remove_file(&paths.lock_path);
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Acquire supervisor process lock {}",
                        paths.lock_path.display()
                    )
                });
            }
        }
    }

    Err(anyhow!(
        "Could not acquire supervisor process lock {}",
        paths.lock_path.display()
    ))
}

fn read_supervisor_lock_pid(path: &Path) -> Option<u32> {
    let raw = std::fs::read_to_string(path).ok()?;
    if let Ok(value) = serde_json::from_str::<Value>(&raw) {
        return value
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok());
    }
    raw.trim().parse::<u32>().ok()
}

#[cfg(unix)]
fn process_is_live(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // kill(pid, 0) does not send a signal; it only checks whether the process
    // exists and whether the current user may signal it.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
fn process_is_live(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    if pid == 0 {
        return false;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return false;
    }
    unsafe {
        CloseHandle(handle);
    }
    true
}

#[cfg(not(any(unix, windows)))]
fn process_is_live(pid: u32) -> bool {
    pid != 0
}

pub(crate) async fn serve(config: SupervisorConfig) -> Result<SupervisorServeReport> {
    let (cloud_dispatch_tx, cloud_dispatch_rx) = mpsc::unbounded_channel::<CloudDispatchRequest>();
    let cloud_actor = supervisor_cloud::spawn_cloud_actor(cloud_dispatch_tx);
    let state = Arc::new(SupervisorState::with_cloud_actor(config, cloud_actor));
    tokio::spawn(handle_cloud_dispatch_requests(
        state.clone(),
        cloud_dispatch_rx,
    ));
    prepare_paths(&state.paths)?;
    let _process_lock = acquire_supervisor_process_lock(&state.paths)?;

    if state.paths.socket_path.exists() {
        match SupervisorClient::connect_with_paths(state.paths.clone()).await {
            Ok(mut client) => {
                let status = client.call("runtime.status", json!({})).await?;
                return Err(anyhow!(
                    "Supervisor already running at {}: {}",
                    state.paths.socket_path.display(),
                    status
                ));
            }
            Err(_) => {
                let _ = std::fs::remove_file(&state.paths.socket_path);
            }
        }
    }

    let name = state
        .paths
        .socket_path
        .clone()
        .to_fs_name::<GenericFilePath>()
        .with_context(|| {
            format!(
                "Invalid supervisor socket path {}",
                state.paths.socket_path.display()
            )
        })?;
    let listener = ListenerOptions::new()
        .name(name)
        .create_tokio()
        .with_context(|| {
            format!(
                "Failed to bind supervisor socket {}",
                state.paths.socket_path.display()
            )
        })?;

    let report = SupervisorServeReport {
        ok: true,
        protocol: RZN_LOCAL_PROTOCOL_VERSION,
        pid: std::process::id(),
        app_base: state.paths.app_base.to_string_lossy().to_string(),
        socket_path: state.paths.socket_path.to_string_lossy().to_string(),
        token_path: state.paths.token_path.to_string_lossy().to_string(),
    };

    loop {
        if state.shutdown.load(Ordering::SeqCst) {
            break;
        }
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => {
                break;
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok(stream) => {
                        let state = state.clone();
                        tokio::spawn(async move {
                            if let Err(err) = handle_connection(stream, state).await {
                                eprintln!("[supervisor] connection error: {}", err);
                            }
                        });
                    }
                    Err(err) => {
                        eprintln!("[supervisor] accept error: {}", err);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }

    Ok(report)
}

pub(crate) async fn call(config: SupervisorConfig, method: &str, params: Value) -> Result<Value> {
    let paths = SupervisorPaths::for_config(&config);
    let mut client = SupervisorClient::connect_with_paths(paths).await?;
    client.call(method, params).await
}

pub(crate) async fn ensure_running(config: SupervisorConfig) -> Result<SupervisorClientStatus> {
    match call(config.clone(), "runtime.status", json!({})).await {
        Ok(result) => Ok(SupervisorClientStatus { ok: true, result }),
        Err(first_err) => {
            spawn_supervisor(&config).await?;
            let deadline = tokio::time::Instant::now() + Duration::from_millis(5_000);
            loop {
                match call(config.clone(), "runtime.status", json!({})).await {
                    Ok(result) => return Ok(SupervisorClientStatus { ok: true, result }),
                    Err(err) if tokio::time::Instant::now() < deadline => {
                        let _ = err;
                        tokio::time::sleep(Duration::from_millis(150)).await;
                    }
                    Err(err) => {
                        return Err(anyhow!(
                            "Supervisor did not become ready after spawn: {}; first error: {}",
                            err,
                            first_err
                        ))
                    }
                }
            }
        }
    }
}

async fn spawn_supervisor(config: &SupervisorConfig) -> Result<()> {
    let exe = std::env::current_exe().context("Resolve current executable")?;
    let mut command = std::process::Command::new(exe);
    command.arg("supervisor").arg("serve");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    if let Some(app_base) = config.app_base.as_ref() {
        command.arg("--app-base").arg(app_base);
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Spawn supervisor")?;
    Ok(())
}

async fn handle_connection(
    mut stream: LocalSocketStream,
    state: Arc<SupervisorState>,
) -> Result<()> {
    let token = read_token(&state.paths.token_path)?;
    let handshake = timeout(
        Duration::from_millis(HANDSHAKE_TIMEOUT_MS),
        read_required_frame(&mut stream),
    )
    .await
    .context("Supervisor handshake timeout")??;
    let value: Value = serde_json::from_slice(&handshake)?;
    if value.get("method").and_then(|value| value.as_str()) == Some("runtime.hello") {
        return handle_native_bridge_connection(stream, state, value, token).await;
    }

    let ok = value.get("type").and_then(|v| v.as_str()) == Some("rzn_local_handshake")
        && value.get("v").and_then(|v| v.as_u64()) == Some(1)
        && value.get("token").and_then(|v| v.as_str()) == Some(token.as_str());
    if !ok {
        let response = json!({ "ok": false, "error": "invalid supervisor handshake" });
        write_frame(&mut stream, &serde_json::to_vec(&response)?).await?;
        return Err(anyhow!("Invalid supervisor handshake"));
    }

    let response = json!({
        "ok": true,
        "protocol": RZN_LOCAL_PROTOCOL_VERSION,
        "pid": std::process::id()
    });
    write_frame(&mut stream, &serde_json::to_vec(&response)?).await?;

    loop {
        let frame = match read_frame(&mut stream).await {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(_) => break,
        };
        let request: Value = serde_json::from_slice(&frame)?;
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request
            .get("method")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));

        let response = match state.dispatch(method, params).await {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(err) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32000, "message": err.to_string() }
            }),
        };
        write_frame(&mut stream, &serde_json::to_vec(&response)?).await?;
    }

    Ok(())
}

async fn handle_cloud_dispatch_requests(
    state: Arc<SupervisorState>,
    mut rx: mpsc::UnboundedReceiver<CloudDispatchRequest>,
) {
    while let Some(request) = rx.recv().await {
        let result = state
            .dispatch_cloud_command_to_extension(
                &request.envelope,
                request.default_request_timeout_ms,
            )
            .await;
        let _ = request.respond_to.send(result);
    }
}

async fn handle_native_bridge_connection(
    mut stream: LocalSocketStream,
    state: Arc<SupervisorState>,
    hello: Value,
    token: String,
) -> Result<()> {
    let id = hello.get("id").cloned().unwrap_or(Value::Null);
    let params = hello.get("params").cloned().unwrap_or_else(|| json!({}));
    let ok = params.get("version").and_then(|value| value.as_str())
        == Some(RZN_LOCAL_PROTOCOL_VERSION)
        && params.get("token").and_then(|value| value.as_str()) == Some(token.as_str())
        && params.get("role").and_then(|value| value.as_str()) == Some("native_host_bridge");
    if !ok {
        let response = json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32001, "message": "invalid native-host bridge hello" }
        });
        write_frame(&mut stream, &serde_json::to_vec(&response)?).await?;
        return Err(anyhow!("Invalid native-host bridge hello"));
    }

    let bridge_id = format!("native-host-{}", Uuid::new_v4());
    let metadata = match NativeHostBridgeMetadata::from_hello_params(
        &params,
        strict_bridge_identity_enabled(),
    ) {
        Ok(metadata) => metadata,
        Err(error) => {
            let response = json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32002,
                    "message": "invalid native-host bridge metadata",
                    "details": {
                        "error_code": "INVALID_BRIDGE_CALLER_ORIGIN",
                        "error": error.to_string()
                    }
                }
            });
            write_frame(&mut stream, &serde_json::to_vec(&response)?).await?;
            return Err(error);
        }
    };
    let (mut reader, mut writer) = stream.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let bridge_epoch = state
        .register_native_bridge_with_metadata(bridge_id.clone(), tx, metadata)
        .await;

    let response = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "ok": true,
            "protocol": RZN_LOCAL_PROTOCOL_VERSION,
            "bridge_id": bridge_id,
            "supervisor_boot_id": state.supervisor_boot_id.clone(),
            "supervisor_bridge_epoch": bridge_epoch,
            "pid": std::process::id(),
            "accepts": ["native_host.extension_call"]
        }
    });
    write_frame(&mut writer, &serde_json::to_vec(&response)?).await?;

    let writer_task = tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if write_frame(&mut writer, &bytes).await.is_err() {
                break;
            }
        }
    });

    loop {
        let frame = match read_frame(&mut reader).await {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(_) => break,
        };
        let value: Value = match serde_json::from_slice(&frame) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if state
            .complete_native_bridge_response(&bridge_id, bridge_epoch, &value)
            .await
        {
            continue;
        }
    }

    state.clear_native_bridge(&bridge_id).await;
    let _ = writer_task.await;
    Ok(())
}

struct SupervisorClient {
    paths: SupervisorPaths,
    stream: LocalSocketStream,
}

impl SupervisorClient {
    async fn connect_with_paths(paths: SupervisorPaths) -> Result<Self> {
        let stream = Self::connect_stream(&paths).await?;
        Ok(Self { paths, stream })
    }

    async fn reconnect(&mut self) -> Result<()> {
        self.stream = Self::connect_stream(&self.paths).await?;
        Ok(())
    }

    async fn connect_stream(paths: &SupervisorPaths) -> Result<LocalSocketStream> {
        let token = read_token(&paths.token_path)?;
        let name = paths
            .socket_path
            .clone()
            .to_fs_name::<GenericFilePath>()
            .with_context(|| {
                format!(
                    "Invalid supervisor socket path {}",
                    paths.socket_path.display()
                )
            })?;
        let mut stream = timeout(
            Duration::from_millis(HANDSHAKE_TIMEOUT_MS),
            LocalSocketStream::connect(name),
        )
        .await
        .context("Connect supervisor timeout")??;

        let handshake = json!({
            "type": "rzn_local_handshake",
            "v": 1,
            "token": token,
            "client": {
                "name": "rzn-browser",
                "pid": std::process::id()
            }
        });
        write_frame(&mut stream, &serde_json::to_vec(&handshake)?).await?;
        let response = timeout(
            Duration::from_millis(HANDSHAKE_TIMEOUT_MS),
            read_required_frame(&mut stream),
        )
        .await
        .context("Supervisor handshake response timeout")??;
        let value: Value = serde_json::from_slice(&response)?;
        if value.get("ok").and_then(|value| value.as_bool()) != Some(true) {
            return Err(anyhow!("Supervisor handshake failed: {}", value));
        }
        Ok(stream)
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let timeout_ms = supervisor_request_timeout_ms(method, &params);
        let id = format!("req-{}", Uuid::new_v4());
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        write_frame(&mut self.stream, &serde_json::to_vec(&request)?).await?;
        let response = match timeout(
            Duration::from_millis(timeout_ms),
            read_required_frame(&mut self.stream),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                let message = match self.reconnect().await {
                    Ok(()) => format!("Supervisor request read failed; connection reset: {error}"),
                    Err(reconnect_error) => format!(
                        "Supervisor request read failed: {error}; reconnect failed: {reconnect_error}"
                    ),
                };
                return Err(anyhow!(message));
            }
            Err(_) => {
                let message = match self.reconnect().await {
                    Ok(()) => "Supervisor request timed out; connection reset".to_string(),
                    Err(reconnect_error) => {
                        format!("Supervisor request timed out; reconnect failed: {reconnect_error}")
                    }
                };
                return Err(anyhow!(message));
            }
        };
        let value: Value = serde_json::from_slice(&response)?;
        if let Some(error) = value.get("error") {
            return Err(anyhow!("Supervisor error: {}", error));
        }
        Ok(value.get("result").cloned().unwrap_or(value))
    }
}

fn prepare_paths(paths: &SupervisorPaths) -> Result<()> {
    std::fs::create_dir_all(&paths.secure_dir)?;
    std::fs::create_dir_all(&paths.run_dir)?;
    get_or_create_token(&paths.token_path)?;
    Ok(())
}

fn get_or_create_token(path: &Path) -> Result<String> {
    if path.exists() {
        set_secret_file_permissions(path).ok();
        return read_token(path);
    }
    let token = Uuid::new_v4().to_string();
    write_secret_file(path, format!("{}\n", token))?;
    Ok(token)
}

fn read_token(path: &Path) -> Result<String> {
    let token = std::fs::read_to_string(path)
        .with_context(|| format!("Read supervisor token {}", path.display()))?
        .trim()
        .to_string();
    if token.is_empty() {
        return Err(anyhow!("Supervisor token is empty: {}", path.display()));
    }
    Ok(token)
}

fn is_browser_tool(name: &str) -> bool {
    matches!(
        name,
        "browser.session_open"
            | "browser.session_close"
            | "browser.targets"
            | "browser.snapshot"
            | "browser.execute_step"
            | "browser.static_command"
            | "browser.poll_events"
            | "rzn.supervisor.health"
    )
}

fn tool_call_arguments(params: &Value) -> Value {
    let mut args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let timeout = params
        .get("timeout_ms")
        .or_else(|| params.get("timeoutMs"))
        .cloned();
    if let (Some(timeout), Value::Object(map)) = (timeout, &mut args) {
        if !map.contains_key("timeout_ms") && !map.contains_key("timeoutMs") {
            map.insert("timeout_ms".to_string(), timeout);
        }
    }
    args
}

fn extension_timeout_ms(params: &Value) -> u64 {
    let requested = params
        .get("timeout_ms")
        .and_then(|value| value.as_u64())
        .or_else(|| params.get("timeoutMs").and_then(|value| value.as_u64()))
        .or_else(|| {
            params.get("step").and_then(|step| {
                step.get("timeout_ms")
                    .and_then(|value| value.as_u64())
                    .or_else(|| step.get("timeoutMs").and_then(|value| value.as_u64()))
            })
        });

    let Some(requested) = requested else {
        return EXTENSION_BRIDGE_TIMEOUT_MS;
    };

    let grace_ms = params
        .get("timeout_grace_ms")
        .and_then(|value| value.as_u64())
        .or_else(|| {
            params
                .get("timeoutGraceMs")
                .and_then(|value| value.as_u64())
        })
        .unwrap_or(EXTENSION_BRIDGE_GRACE_MS);

    clamp_caller_timeout_ms(requested)
        .saturating_add(grace_ms)
        .min(MAX_CALLER_TIMEOUT_MS)
}

fn clamp_caller_timeout_ms(timeout_ms: u64) -> u64 {
    timeout_ms.clamp(1, MAX_CALLER_TIMEOUT_MS)
}

fn heal_bridge_wait_ms(params: &Value) -> u64 {
    params
        .get("bridge_wait_ms")
        .and_then(|value| value.as_u64())
        .or_else(|| env_u64("RZN_SUPERVISOR_HEAL_BRIDGE_WAIT_MS"))
        .unwrap_or(HEAL_BRIDGE_WAIT_MS)
}

fn bridge_probe_timeout_ms(params: &Value) -> u64 {
    params
        .get("bridge_probe_timeout_ms")
        .and_then(|value| value.as_u64())
        .or_else(|| env_u64("RZN_SUPERVISOR_BRIDGE_PROBE_TIMEOUT_MS"))
        .unwrap_or(DEFAULT_BRIDGE_PROBE_TIMEOUT_MS)
}

fn bridge_probe_transport_ok(value: &Value) -> bool {
    value.get("success").and_then(|v| v.as_bool()) == Some(true)
        || value.pointer("/result/pong").and_then(|v| v.as_bool()) == Some(true)
}

fn bridge_probe_has_capability(value: &Value, capability: &str) -> bool {
    value
        .pointer(&format!("/result/capabilities/{}", capability))
        .and_then(|v| v.as_bool())
        == Some(true)
}

fn bridge_probe_contract_version(value: &Value) -> Option<u64> {
    value
        .pointer("/result/bridge_contract_version")
        .or_else(|| value.get("bridge_contract_version"))
        .and_then(Value::as_u64)
}

fn bridge_probe_contract_version_ok(value: &Value) -> bool {
    bridge_probe_contract_version(value).unwrap_or(0) >= EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION
}

fn bridge_probe_required_capabilities_map(value: &Value) -> Value {
    let mut map = serde_json::Map::new();
    for capability in REQUIRED_EXTENSION_BRIDGE_CAPABILITIES {
        map.insert(
            (*capability).to_string(),
            Value::Bool(bridge_probe_has_capability(value, capability)),
        );
    }
    Value::Object(map)
}

fn bridge_probe_required_capabilities_ok(probe: &Value) -> bool {
    let capabilities_ok = REQUIRED_EXTENSION_BRIDGE_CAPABILITIES
        .iter()
        .all(|capability| {
            probe
                .pointer(&format!("/required_capabilities/{}", capability))
                .and_then(Value::as_bool)
                == Some(true)
        });
    let contract_ok = probe
        .get("bridge_contract_version_ok")
        .and_then(Value::as_bool)
        == Some(true);
    capabilities_ok && contract_ok
}

fn bridge_probe_target_ok(probe: &Value) -> bool {
    probe
        .get("target_match_ok")
        .and_then(Value::as_bool)
        .unwrap_or(!bridge_probe_has_target_resolution_error(probe))
}

fn bridge_probe_has_target_resolution_error(probe: &Value) -> bool {
    probe.get("target_resolution_error").is_some()
        || probe
            .get("error_code")
            .and_then(Value::as_str)
            .is_some_and(is_browser_target_error_code)
}

fn bridge_readiness_capability_policy() -> Value {
    json!({
        "mode": "global_readiness_gate",
        "expected_bridge_contract_version": EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION,
        "required_capabilities": REQUIRED_EXTENSION_BRIDGE_CAPABILITIES,
        "reason": "The loaded extension must advertise the current bridge contract and reliability capabilities; a responsive extension without them is treated as a stale bundle, not as a transport failure."
    })
}

fn bridge_readiness_diagnostic(connected: bool, responsive: bool, probe: Option<&Value>) -> Value {
    if responsive {
        return Value::Null;
    }

    let cause = bridge_readiness_cause(connected, probe);
    let (message, action_text) = match cause {
        READINESS_CAUSE_BRIDGE_DOWN => (
            "Native-host bridge is not connected.",
            "Open the existing Chrome profile with the RZN extension enabled, then retry.",
        ),
        READINESS_CAUSE_STALE_EXTENSION_BUNDLE => (
            "Loaded extension answered readiness ping but is missing the current bridge contract or required reliability capabilities.",
            "Reload the RZN extension from the current extension/dist/chrome bundle, then retry.",
        ),
        READINESS_CAUSE_BROWSER_TARGET_UNRESOLVED => (
            "Requested browser target is not connected, or no connected bridge has reported enough browser identity metadata yet.",
            "Run `rzn-browser browser targets --json`, then pass a listed `--browser`, `--browser-instance`, or `--bridge`; if the expected browser is missing, reload that browser's RZN extension.",
        ),
        READINESS_CAUSE_BROWSER_TARGET_MISMATCH => (
            "Readiness ping reached a browser bridge, but it did not match the requested browser target.",
            "Run `rzn-browser browser targets --json` and select the reported browser/bridge, or open/reload the requested browser's RZN extension.",
        ),
        READINESS_CAUSE_TRANSPORT_TIMEOUT => (
            "Native-host bridge was connected, but the extension readiness ping timed out.",
            "Run `rzn-browser heal --json`, then retry. If it repeats, reload the RZN extension in the existing Chrome session.",
        ),
        READINESS_CAUSE_ZOMBIE_NATIVE_HOST => (
            "Native-host bridge is connected at the process/socket layer, but extension replies are no longer round-tripping.",
            "Run `rzn-browser heal --json` to force a native-host/native-port restart, then retry.",
        ),
        _ => (
            "Native-host bridge was connected, but the extension service worker did not complete the readiness contract.",
            "Wake or reload the RZN extension service worker, then run `rzn-browser heal --json` and retry.",
        ),
    };

    json!({
        "cause": cause,
        "inferred": cause != READINESS_CAUSE_BRIDGE_DOWN,
        "message": message,
        "action_text": action_text,
        "actions": [
            action_text,
            "Run `rzn-browser supervisor status --json` to inspect the current bridge state."
        ],
        "observed": {
            "native_host_bridge_connected": connected,
            "native_host_bridge_responsive": responsive,
            "probe_transport_ok": probe
                .and_then(|probe| probe.get("transport_ok"))
                .cloned()
                .unwrap_or(Value::Null),
            "probe_timeout_ms": probe
                .and_then(|probe| probe.get("timeout_ms"))
                .cloned()
                .unwrap_or(Value::Null),
            "probe_error": probe
                .and_then(|probe| probe.get("error"))
                .cloned()
                .unwrap_or(Value::Null),
            "probe_error_code": probe
                .and_then(|probe| probe.get("error_code"))
                .cloned()
                .unwrap_or(Value::Null),
            "target_match_ok": probe
                .and_then(|probe| probe.get("target_match_ok"))
                .cloned()
                .unwrap_or(Value::Null),
            "requested_browser": probe
                .and_then(|probe| probe.get("requested_browser"))
                .cloned()
                .unwrap_or(Value::Null),
            "loaded_browser": probe
                .and_then(|probe| probe.get("loaded_browser"))
                .cloned()
                .unwrap_or(Value::Null),
            "loaded_browser_hint": probe
                .and_then(|probe| probe.get("loaded_browser_hint"))
                .cloned()
                .unwrap_or(Value::Null),
            "target_resolution_error": probe
                .and_then(|probe| probe.get("target_resolution_error"))
                .cloned()
                .unwrap_or(Value::Null),
            "expected_bridge_contract_version": EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION,
            "loaded_bridge_contract_version": probe
                .and_then(|probe| probe.get("bridge_contract_version"))
                .cloned()
                .unwrap_or(Value::Null),
            "bridge_contract_version_ok": probe
                .and_then(|probe| probe.get("bridge_contract_version_ok"))
                .cloned()
                .unwrap_or(Value::Null),
            "expected_capabilities": REQUIRED_EXTENSION_BRIDGE_CAPABILITIES
                .iter()
                .map(|capability| ((*capability).to_string(), Value::Bool(true)))
                .collect::<serde_json::Map<String, Value>>(),
            "loaded_capabilities": probe
                .and_then(|probe| probe.pointer("/response/result/capabilities"))
                .cloned()
                .unwrap_or(Value::Null),
            "loaded_extension_build_signature": probe
                .and_then(|probe| probe.get("extension_build_signature"))
                .cloned()
                .unwrap_or(Value::Null)
        }
    })
}

fn bridge_readiness_cause(connected: bool, probe: Option<&Value>) -> &'static str {
    if !connected {
        return READINESS_CAUSE_BRIDGE_DOWN;
    }

    let Some(probe) = probe else {
        return READINESS_CAUSE_SERVICE_WORKER_UNRESPONSIVE;
    };

    if bridge_probe_has_target_resolution_error(probe) {
        return READINESS_CAUSE_BROWSER_TARGET_UNRESOLVED;
    }

    let required_capability_missing =
        REQUIRED_EXTENSION_BRIDGE_CAPABILITIES
            .iter()
            .any(|capability| {
                probe
                    .pointer(&format!("/required_capabilities/{}", capability))
                    .and_then(Value::as_bool)
                    == Some(false)
            });
    let contract_missing = probe
        .get("bridge_contract_version_ok")
        .and_then(Value::as_bool)
        == Some(false);
    if required_capability_missing
        || contract_missing
        || bridge_probe_error(probe).contains("bridge contract")
        || bridge_probe_error(probe).contains(REQUIRED_EXTENSION_KEEPALIVE_CAPABILITY)
    {
        return READINESS_CAUSE_STALE_EXTENSION_BUNDLE;
    }

    if probe.get("target_match_ok").and_then(Value::as_bool) == Some(false)
        && probe.get("transport_ok").and_then(Value::as_bool) == Some(true)
    {
        return READINESS_CAUSE_BROWSER_TARGET_MISMATCH;
    }

    if bridge_probe_error(probe).contains("timeout") {
        return READINESS_CAUSE_ZOMBIE_NATIVE_HOST;
    }

    READINESS_CAUSE_SERVICE_WORKER_UNRESPONSIVE
}

fn bridge_probe_error(probe: &Value) -> String {
    probe
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn native_host_bridge_error_message(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string())
}

fn native_bridge_disconnected_response(
    id: &str,
    bridge_id: &str,
    method: &str,
    deadline_at_ms: u64,
) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32000,
            "message": format!(
                "native host disconnected: retired bridge {} while {} was pending",
                bridge_id, method
            ),
            "data": {
                "error_code": "NATIVE_HOST_DISCONNECTED",
                "bridge_id": bridge_id,
                "method": method,
                "deadline_at_ms": deadline_at_ms
            }
        }
    })
}

fn native_host_bridge_transport_error_cause(error: &Value) -> Option<&'static str> {
    let message = native_host_bridge_error_message(error).to_ascii_lowercase();
    let code = error.get("code").and_then(Value::as_i64);
    let error_code = error
        .get("data")
        .and_then(|data| data.get("error_code"))
        .or_else(|| error.get("error_code"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_uppercase();
    if code == Some(-32003)
        || code == Some(-32002)
        || error_code.contains("TIMEOUT")
        || message.contains("timeout")
        || message.contains("timed out")
        || message.contains("response channel closed")
    {
        return Some(READINESS_CAUSE_ZOMBIE_NATIVE_HOST);
    }
    if error_code == "NATIVE_HOST_DISCONNECTED"
        || message.contains("disconnected")
        || message.contains("stdout channel closed")
        || message.contains("native port closed")
    {
        return Some(READINESS_CAUSE_BRIDGE_DOWN);
    }
    None
}

fn native_host_bridge_error_cause(error: &Value) -> &'static str {
    native_host_bridge_transport_error_cause(error)
        .unwrap_or(READINESS_CAUSE_SERVICE_WORKER_UNRESPONSIVE)
}

fn is_native_bridge_transient_dispatch_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("native-host bridge response channel closed")
        || message.contains("native-host extension bridge timeout")
        || message.contains("native_host_disconnected")
        || message.contains("native host disconnected")
        || message.contains("extension timeout")
        || message.contains("extension response channel closed")
}

fn native_bridge_transient_dispatch_error_code(message: &str) -> &'static str {
    let message = message.to_ascii_lowercase();
    if message.contains("timeout") {
        "EXTENSION_BRIDGE_TIMEOUT"
    } else {
        "NATIVE_HOST_DISCONNECTED"
    }
}

fn should_retry_heal_after_probe_reset(readiness: &Value) -> bool {
    if readiness.get("ready").and_then(|value| value.as_bool()) == Some(true) {
        return false;
    }

    let bridge_was_connected = readiness
        .pointer("/native_host_bridge/connected")
        .and_then(|value| value.as_bool())
        == Some(true);
    if !bridge_was_connected {
        return false;
    }

    let probe_error = readiness
        .pointer("/native_host_bridge/probe/error")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    !probe_error.contains(REQUIRED_EXTENSION_KEEPALIVE_CAPABILITY)
}

fn readiness_value_ready(value: &Value) -> bool {
    value.get("ok").and_then(Value::as_bool) == Some(true)
        || value.get("ready").and_then(Value::as_bool) == Some(true)
}

fn supervisor_request_timeout_ms(method: &str, params: &Value) -> u64 {
    let requested = params
        .get("timeout_ms")
        .and_then(|value| value.as_u64())
        .or_else(|| params.get("timeoutMs").and_then(|value| value.as_u64()))
        .or_else(|| {
            params.get("arguments").and_then(|arguments| {
                arguments
                    .get("timeout_ms")
                    .and_then(|value| value.as_u64())
                    .or_else(|| arguments.get("timeoutMs").and_then(|value| value.as_u64()))
                    .or_else(|| {
                        arguments.get("step").and_then(|step| {
                            step.get("timeout_ms")
                                .and_then(|value| value.as_u64())
                                .or_else(|| step.get("timeoutMs").and_then(|value| value.as_u64()))
                        })
                    })
            })
        });
    let default_timeout_ms = if method == "runtime.heal" {
        let probe_timeout_ms =
            bridge_probe_timeout_ms(params).saturating_add(BRIDGE_PROBE_TIMEOUT_GRACE_MS);

        heal_bridge_wait_ms(params)
            .saturating_add(HEAL_INITIAL_BRIDGE_WAIT_MS)
            .saturating_add(HEAL_POST_PROBE_RECONNECT_WAIT_MS)
            .saturating_add(probe_timeout_ms.saturating_mul(HEAL_READINESS_PROBE_COUNT))
    } else {
        REQUEST_TIMEOUT_MS
    };

    let bridge_timeout_grace_ms = if is_browser_tool(method) {
        EXTENSION_BRIDGE_GRACE_MS
    } else {
        0
    };
    let requested_or_default = requested
        .map(clamp_caller_timeout_ms)
        .unwrap_or(default_timeout_ms);
    requested_or_default
        .saturating_add(bridge_timeout_grace_ms)
        .min(MAX_CALLER_TIMEOUT_MS)
        .saturating_add(REQUEST_TIMEOUT_GRACE_MS)
        .max(default_timeout_ms)
}

fn value_id_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct JsonRpcRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    fn test_config() -> SupervisorConfig {
        SupervisorConfig {
            app_base: Some(PathBuf::from("/tmp/rzn-supervisor-test")),
        }
    }

    #[tokio::test]
    async fn supervisor_client_reconnects_after_bad_frame_read() {
        let app_base = PathBuf::from(format!("/tmp/rzn-scf-{}", Uuid::new_v4()));
        let config = SupervisorConfig {
            app_base: Some(app_base.clone()),
        };
        let paths = SupervisorPaths::for_config(&config);
        prepare_paths(&paths).expect("prepare supervisor paths");
        let _ = std::fs::remove_file(&paths.socket_path);
        let name = paths
            .socket_path
            .clone()
            .to_fs_name::<GenericFilePath>()
            .expect("socket path");
        let listener = ListenerOptions::new()
            .name(name)
            .create_tokio()
            .expect("listener");

        let server = tokio::spawn(async move {
            for index in 0..2 {
                let mut stream = listener.accept().await.expect("accept");
                let _handshake = read_required_frame(&mut stream)
                    .await
                    .expect("handshake frame");
                write_frame(
                    &mut stream,
                    &serde_json::to_vec(&json!({ "ok": true })).unwrap(),
                )
                .await
                .expect("handshake response");

                let request = read_required_frame(&mut stream)
                    .await
                    .expect("request frame");
                let request: Value = serde_json::from_slice(&request).expect("request json");
                if index == 0 {
                    stream
                        .write_all(&0u32.to_le_bytes())
                        .await
                        .expect("bad frame header");
                    stream.flush().await.expect("flush bad frame");
                    continue;
                }

                let response = json!({
                    "jsonrpc": "2.0",
                    "id": request.get("id").cloned().unwrap_or(Value::Null),
                    "result": { "ready": true }
                });
                write_frame(&mut stream, &serde_json::to_vec(&response).unwrap())
                    .await
                    .expect("response frame");
            }
        });

        let mut client = SupervisorClient::connect_with_paths(paths)
            .await
            .expect("connect client");
        let error = client
            .call("runtime.status", json!({}))
            .await
            .expect_err("bad frame should reset connection");
        assert!(error.to_string().contains("connection reset"));

        let response = client
            .call("runtime.status", json!({}))
            .await
            .expect("second call uses reconnected stream");
        assert_eq!(response.get("ready"), Some(&json!(true)));

        server.await.expect("server task");
        let _ = std::fs::remove_dir_all(app_base);
    }

    fn current_bridge_ping_response(id: Value, build: &str) -> Value {
        let capabilities = json!({
            "content_keepalive_port": true,
            "native_host_stdout_heartbeat": true,
            "native_roundtrip_ping_health": true,
            "native_port_epoch_fencing": true,
            "workflow_session_epoch_fencing": true,
            "broker_watchdog": true,
            "request_lease_cancellation": true,
            "watchdog_queue_unblock": true,
            "epoch_chain_identity": true,
            "native_control_epoch_fencing": true,
            "supervisor_bridge_response_fencing": true,
            "health_beacon_v2": true,
            "auxiliary_path_lease_guards": true,
            "port_scoped_disconnect_suppression": true,
            "native_message_frame_cap": true
        });
        let browser_diagnostics = json!({
            "user_agent": "diagnostic-only",
            "platform": "test-platform"
        });
        json!({
            "id": id,
            "result": {
                "success": true,
                "result": {
                    "pong": true,
                    "extension_build_signature": build,
                    "bridge_contract_version": EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION,
                    "extension_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "extension_origin": "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
                    "browser_instance_id": "browser-instance-test",
                    "extension_target": "chrome",
                    "extension_manifest_version": 3,
                    "extension_target_hint": "chromium-mv3",
                    "supervisor_bridge_epoch": 1,
                    "native_host_pid": 12345,
                    "native_host_boot_id": "native-host-test",
                    "extension_worker_boot_id": "extension-worker-test",
                    "native_port_epoch": 1,
                    "last_native_host_stdout_heartbeat_ms": now_ms(),
                    "last_native_host_stdout_heartbeat_age_ms": 10,
                    "last_native_host_stdout_heartbeat_seq": 7,
                    "last_native_roundtrip_ping_ms": now_ms(),
                    "last_native_roundtrip_ping_age_ms": 5,
                    "missed_native_roundtrip_pings": 0,
                    "browser_diagnostics": browser_diagnostics,
                    "capabilities": capabilities
                }
            }
        })
    }

    async fn complete_bridge_response_for_test(
        state: &SupervisorState,
        bridge_id: &str,
        value: &Value,
    ) -> bool {
        let bridge_epoch = state
            .native_bridges
            .lock()
            .await
            .get(bridge_id)
            .map(|bridge| bridge.epoch)
            .expect("test bridge is registered");
        state
            .complete_native_bridge_response(bridge_id, bridge_epoch, value)
            .await
    }

    async fn register_bridge_with_target_for_test(
        state: &SupervisorState,
        bridge_id: &str,
        tx: mpsc::UnboundedSender<Vec<u8>>,
        browser_instance_id: &str,
        browser: &str,
    ) -> u64 {
        state
            .register_native_bridge_with_metadata(
                bridge_id.to_string(),
                tx,
                NativeHostBridgeMetadata {
                    browser_instance_id: Some(browser_instance_id.to_string()),
                    extension_target: Some(browser.to_string()),
                    identity_status: "test".to_string(),
                    ..NativeHostBridgeMetadata::missing()
                },
            )
            .await
    }

    async fn register_bridge_with_identity_for_test(
        state: &SupervisorState,
        bridge_id: &str,
        tx: mpsc::UnboundedSender<Vec<u8>>,
        browser_instance_id: &str,
        browser: &str,
        caller_origin: &str,
    ) -> u64 {
        let metadata = NativeHostBridgeMetadata::from_hello_params(
            &json!({ "caller_origin": caller_origin }),
            false,
        )
        .expect("test caller origin parses");
        state
            .register_native_bridge_with_metadata(
                bridge_id.to_string(),
                tx,
                NativeHostBridgeMetadata {
                    browser_instance_id: Some(browser_instance_id.to_string()),
                    extension_target: Some(browser.to_string()),
                    ..metadata
                },
            )
            .await
    }

    #[test]
    fn parses_native_host_bridge_metadata_from_hello() {
        let metadata = NativeHostBridgeMetadata::from_hello_params(
            &json!({
                "native_host_pid": 12345,
                "native_host_boot_id": "native-host-test",
                "caller_origin": "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
                "launch": {
                    "parent_window": "0",
                    "has_socket_override": true,
                    "has_token_override": true,
                    "extra_arg_count": 2,
                    "token": "must-not-be-read"
                },
                "token": "supervisor-secret"
            }),
            false,
        )
        .unwrap();

        assert_eq!(metadata.native_host_pid, Some(12345));
        assert_eq!(
            metadata.native_host_boot_id.as_deref(),
            Some("native-host-test")
        );
        assert_eq!(
            metadata.caller_origin.as_deref(),
            Some("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/")
        );
        assert_eq!(
            metadata.caller_extension_id.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(metadata.caller_origin_status, "present");
        assert_eq!(metadata.identity_status, "verified_launch_origin");
        assert_eq!(metadata.launch.parent_window.as_deref(), Some("0"));
        assert!(metadata.launch.has_socket_override);
        assert!(metadata.launch.has_token_override);
        assert_eq!(metadata.launch.extra_arg_count, Some(2));

        let encoded = serde_json::to_string(&metadata).unwrap();
        assert!(!encoded.contains("supervisor-secret"));
        assert!(!encoded.contains("must-not-be-read"));
    }

    #[test]
    fn native_host_bridge_metadata_allows_missing_and_malformed_origin() {
        let missing = NativeHostBridgeMetadata::from_hello_params(&json!({}), false).unwrap();
        assert_eq!(missing.caller_origin, None);
        assert_eq!(missing.caller_origin_status, "missing");
        assert_eq!(missing.identity_status, "missing");

        let malformed = NativeHostBridgeMetadata::from_hello_params(
            &json!({
                "caller_origin": "chrome-extension://not-valid/"
            }),
            false,
        )
        .unwrap();
        assert_eq!(malformed.caller_origin, None);
        assert_eq!(malformed.caller_origin_status, "invalid_origin");
        assert_eq!(malformed.identity_status, "invalid_origin");
        assert!(NativeHostBridgeMetadata::from_hello_params(
            &json!({
                "caller_origin": "chrome-extension://not-valid/"
            }),
            true
        )
        .is_err());
    }

    #[test]
    fn native_host_bridge_metadata_matches_and_mismatches_extension_report() {
        let mut matched = NativeHostBridgeMetadata::from_hello_params(
            &json!({
                "caller_origin": "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/"
            }),
            false,
        )
        .unwrap();
        matched.update_extension_reported_metadata(&json!({
            "result": {
                "extension_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "browser_instance_id": "browser-instance-a",
                "extension_target": "chrome",
                "extension_manifest_version": 3,
                "extension_target_hint": "chromium-mv3",
                "extension_build_signature": "fresh-build",
                "bridge_contract_version": EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION,
                "extension_worker_boot_id": "extension-worker-a",
                "browser_diagnostics": {
                    "user_agent": "diagnostic-only"
                }
            }
        }));
        assert_eq!(matched.identity_match, Some(true));
        assert_eq!(matched.identity_status, "matched");
        assert_eq!(
            matched.extension_reported_origin.as_deref(),
            Some("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/")
        );
        assert_eq!(
            matched.browser_instance_id.as_deref(),
            Some("browser-instance-a")
        );
        assert_eq!(matched.extension_target.as_deref(), Some("chrome"));
        assert_eq!(matched.extension_manifest_version, Some(3));
        assert_eq!(
            matched.extension_target_hint.as_deref(),
            Some("chromium-mv3")
        );
        assert_eq!(
            matched.extension_build_signature,
            Some(json!("fresh-build"))
        );
        assert_eq!(
            matched.bridge_contract_version,
            Some(EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION)
        );
        assert_eq!(
            matched.extension_worker_boot_id.as_deref(),
            Some("extension-worker-a")
        );
        assert_eq!(
            matched
                .browser_diagnostics
                .as_ref()
                .and_then(|value| value.pointer("/user_agent"))
                .and_then(Value::as_str),
            Some("diagnostic-only")
        );

        let mut mismatched = NativeHostBridgeMetadata::from_hello_params(
            &json!({
                "caller_origin": "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/"
            }),
            false,
        )
        .unwrap();
        mismatched.update_extension_reported_identity(&json!({
            "result": {
                "extension_origin": "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/"
            }
        }));
        assert_eq!(
            mismatched.caller_origin.as_deref(),
            Some("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/")
        );
        assert_eq!(mismatched.identity_match, Some(false));
        assert_eq!(mismatched.identity_status, "mismatched");
    }

    #[tokio::test]
    async fn registered_bridge_status_includes_launch_metadata() {
        let state = SupervisorState::new(test_config());
        let (tx, _rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let metadata = NativeHostBridgeMetadata::from_hello_params(
            &json!({
                "native_host_pid": 12345,
                "native_host_boot_id": "native-host-test",
                "caller_origin": "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
                "launch": {
                    "parent_window": "0",
                    "has_socket_override": true,
                    "has_token_override": true,
                    "extra_arg_count": 0
                }
            }),
            false,
        )
        .unwrap();

        state
            .register_native_bridge_with_metadata("metadata-bridge".to_string(), tx, metadata)
            .await;
        let status = state.runtime_status().await;

        assert_eq!(
            status
                .pointer("/native_host_bridge/health/current_bridge_metadata/caller_origin")
                .and_then(Value::as_str),
            Some("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/")
        );
        assert_eq!(
            status
                .pointer("/native_host_bridge/health/current_bridge_metadata/caller_origin_status")
                .and_then(Value::as_str),
            Some("present")
        );
        assert_eq!(
            status
                .pointer("/native_host_bridge/health/current_bridge_metadata/native_host_pid")
                .and_then(Value::as_u64),
            Some(12345)
        );
        assert_eq!(
            status
                .pointer("/native_host_bridge/health/current_bridge_metadata/launch/parent_window")
                .and_then(Value::as_str),
            Some("0")
        );

        let encoded = serde_json::to_string(&status).unwrap();
        assert!(!encoded.contains("supervisor-secret"));
        assert!(!encoded.contains("must-not-be-read"));
    }

    #[tokio::test]
    async fn bridge_health_retains_extension_identity_metadata_per_registered_bridge() {
        let state = SupervisorState::new(test_config());
        let (first_tx, _first_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let first_metadata = NativeHostBridgeMetadata::from_hello_params(
            &json!({
                "caller_origin": "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/"
            }),
            false,
        )
        .unwrap();
        state
            .register_native_bridge_with_metadata(
                "first-bridge".to_string(),
                first_tx,
                first_metadata,
            )
            .await;
        state
            .record_native_bridge_ping_success(
                "first-bridge",
                &json!({
                    "result": {
                        "extension_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "extension_origin": "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
                        "browser_instance_id": "browser-instance-one",
                        "extension_target": "chrome",
                        "extension_build_signature": "build-one",
                        "bridge_contract_version": EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION,
                        "extension_worker_boot_id": "worker-one",
                        "extension_manifest_version": 3,
                        "extension_target_hint": "chromium-mv3",
                        "browser_diagnostics": {
                            "user_agent": "diagnostic-one"
                        }
                    }
                }),
                11,
            )
            .await;

        let status = state.runtime_status().await;
        assert_eq!(
            status
                .pointer("/native_host_bridge/health/current_bridge_metadata/browser_instance_id")
                .and_then(Value::as_str),
            Some("browser-instance-one")
        );
        assert_eq!(
            status
                .pointer("/native_host_bridge/health/current_bridge_metadata/identity_status")
                .and_then(Value::as_str),
            Some("matched")
        );

        let (second_tx, _second_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let second_metadata = NativeHostBridgeMetadata::from_hello_params(
            &json!({
                "caller_origin": "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/"
            }),
            false,
        )
        .unwrap();
        state
            .register_native_bridge_with_metadata(
                "second-bridge".to_string(),
                second_tx,
                second_metadata,
            )
            .await;
        state
            .record_native_bridge_ping_success(
                "second-bridge",
                &json!({
                    "result": {
                        "extension_id": "cccccccccccccccccccccccccccccccc",
                        "extension_origin": "chrome-extension://cccccccccccccccccccccccccccccccc/",
                        "browser_instance_id": "browser-instance-two",
                        "extension_target": "edge",
                        "extension_build_signature": "build-two",
                        "bridge_contract_version": EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION,
                        "extension_worker_boot_id": "worker-two",
                        "extension_manifest_version": 2,
                        "extension_target_hint": "firefox-mv2",
                        "browser_diagnostics": {
                            "user_agent": "diagnostic-two"
                        }
                    }
                }),
                13,
            )
            .await;

        let status = state.runtime_status().await;
        assert_eq!(
            status
                .pointer("/native_host_bridge/health/current_bridge_id")
                .cloned(),
            Some(Value::Null)
        );
        assert_eq!(
            status
                .pointer("/native_host_bridge/health/last_registered_bridge_id")
                .and_then(Value::as_str),
            Some("second-bridge")
        );
        assert_eq!(
            status
                .pointer(
                    "/native_host_bridge/health/bridges/first-bridge/current_bridge_metadata/browser_instance_id"
                )
                .and_then(Value::as_str),
            Some("browser-instance-one")
        );
        assert_eq!(
            status
                .pointer(
                    "/native_host_bridge/health/bridges/first-bridge/current_bridge_metadata/identity_status"
                )
                .and_then(Value::as_str),
            Some("matched")
        );
        assert_eq!(
            status
                .pointer(
                    "/native_host_bridge/health/bridges/second-bridge/current_bridge_metadata/browser_instance_id"
                )
                .and_then(Value::as_str),
            Some("browser-instance-two")
        );
        assert_eq!(
            status
                .pointer(
                    "/native_host_bridge/health/bridges/second-bridge/current_bridge_metadata/extension_target_hint"
                )
                .and_then(Value::as_str),
            Some("firefox-mv2")
        );
        assert_eq!(
            status
                .pointer(
                    "/native_host_bridge/health/bridges/second-bridge/current_bridge_metadata/extension_target"
                )
                .and_then(Value::as_str),
            Some("edge")
        );
        assert_eq!(
            status
                .pointer(
                    "/native_host_bridge/health/bridges/second-bridge/current_bridge_metadata/identity_status"
                )
                .and_then(Value::as_str),
            Some("mismatched")
        );
        assert_eq!(
            status
                .pointer(
                    "/native_host_bridge/health/bridges/second-bridge/current_bridge_metadata/identity_match"
                )
                .and_then(Value::as_bool),
            Some(false)
        );
    }

    #[tokio::test]
    async fn per_bridge_health_records_are_isolated_and_aggregated() {
        let state = SupervisorState::new(test_config());
        let (chrome_tx, _chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let chrome_metadata = NativeHostBridgeMetadata::from_hello_params(
            &json!({
                "caller_origin": "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/"
            }),
            false,
        )
        .unwrap();
        state
            .register_native_bridge_with_metadata(
                "chrome-bridge".to_string(),
                chrome_tx,
                chrome_metadata,
            )
            .await;

        let (edge_tx, _edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let edge_metadata = NativeHostBridgeMetadata::from_hello_params(
            &json!({
                "caller_origin": "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/"
            }),
            false,
        )
        .unwrap();
        state
            .register_native_bridge_with_metadata("edge-bridge".to_string(), edge_tx, edge_metadata)
            .await;

        state
            .record_native_bridge_ping_success(
                "chrome-bridge",
                &json!({
                    "result": {
                        "extension_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "browser_instance_id": "browser-instance-chrome",
                        "extension_target": "chrome",
                        "extension_build_signature": "chrome-build",
                        "bridge_contract_version": EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION
                    }
                }),
                7,
            )
            .await;
        state
            .record_native_bridge_ping_success(
                "edge-bridge",
                &json!({
                    "result": {
                        "extension_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        "browser_instance_id": "browser-instance-edge",
                        "extension_target": "edge",
                        "extension_build_signature": "edge-build",
                        "bridge_contract_version": EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION
                    }
                }),
                9,
            )
            .await;
        state
            .record_native_bridge_failure(
                Some("edge-bridge"),
                READINESS_CAUSE_ZOMBIE_NATIVE_HOST,
                "edge timed out",
                true,
            )
            .await;

        let status = state.runtime_status().await;
        let health = status
            .pointer("/native_host_bridge/health")
            .expect("native bridge health");
        assert_eq!(
            health.pointer("/bridges/chrome-bridge/last_successful_extension_build_signature"),
            Some(&json!("chrome-build"))
        );
        assert_eq!(
            health.pointer("/bridges/chrome-bridge/last_failure_cause"),
            Some(&Value::Null)
        );
        assert_eq!(
            health.pointer("/bridges/edge-bridge/last_successful_extension_build_signature"),
            Some(&json!("edge-build"))
        );
        assert_eq!(
            health
                .pointer("/bridges/edge-bridge/last_failure_cause")
                .and_then(Value::as_str),
            Some(READINESS_CAUSE_ZOMBIE_NATIVE_HOST)
        );
        assert_eq!(
            health
                .pointer("/connected_bridge_count")
                .and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(
            health
                .pointer("/healthy_bridge_count")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            health
                .pointer("/ambiguous_default_target")
                .and_then(Value::as_bool),
            Some(true)
        );

        state.clear_native_bridge("edge-bridge").await;
        let status = state.runtime_status().await;
        assert_eq!(
            status
                .pointer("/native_host_bridge/health/current_bridge_id")
                .and_then(Value::as_str),
            Some("chrome-bridge")
        );
        assert_eq!(
            status
                .pointer("/native_host_bridge/health/connected_bridge_count")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            status
                .pointer("/native_host_bridge/health/ambiguous_default_target")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            status.pointer("/native_host_bridge/health/last_successful_extension_build_signature"),
            Some(&json!("chrome-build"))
        );
    }

    #[test]
    fn default_paths_live_under_secure_and_run_dirs() {
        let config = test_config();
        let paths = SupervisorPaths::for_config(&config);
        assert_eq!(
            paths.socket_path,
            PathBuf::from("/tmp/rzn-supervisor-test/run/rzn-supervisor.sock")
        );
        assert_eq!(
            paths.token_path,
            PathBuf::from("/tmp/rzn-supervisor-test/secure/rzn-supervisor-token-v1")
        );
        assert_eq!(
            paths.lock_path,
            PathBuf::from("/tmp/rzn-supervisor-test/run/rzn-supervisor.lock")
        );
    }

    #[test]
    fn supervisor_process_lock_rejects_live_pid_lock() {
        let app_base =
            std::env::temp_dir().join(format!("rzn-supervisor-lock-live-{}", Uuid::new_v4()));
        let config = SupervisorConfig {
            app_base: Some(app_base.clone()),
        };
        let paths = SupervisorPaths::for_config(&config);
        prepare_paths(&paths).expect("prepare paths");
        std::fs::write(
            &paths.lock_path,
            serde_json::to_string(&json!({ "pid": std::process::id() })).unwrap(),
        )
        .expect("write lock");

        let error = acquire_supervisor_process_lock(&paths).expect_err("live pid lock rejects");
        assert!(error.to_string().contains("already running or starting"));
        let _ = std::fs::remove_dir_all(app_base);
    }

    #[test]
    fn supervisor_process_lock_replaces_stale_pid_lock() {
        let app_base =
            std::env::temp_dir().join(format!("rzn-supervisor-lock-stale-{}", Uuid::new_v4()));
        let config = SupervisorConfig {
            app_base: Some(app_base.clone()),
        };
        let paths = SupervisorPaths::for_config(&config);
        prepare_paths(&paths).expect("prepare paths");
        std::fs::write(&paths.lock_path, "{\"pid\":2000000000}\n").expect("write lock");

        let lock = acquire_supervisor_process_lock(&paths).expect("stale pid lock replaced");
        assert!(paths.lock_path.exists());
        drop(lock);
        assert!(!paths.lock_path.exists());
        let _ = std::fs::remove_dir_all(app_base);
    }

    #[test]
    fn browser_tool_allowlist_is_explicit() {
        assert!(is_browser_tool("browser.execute_step"));
        assert!(is_browser_tool("browser.static_command"));
        assert!(is_browser_tool("rzn.supervisor.health"));
        assert!(!is_browser_tool("rzn.worker.health"));
        assert!(!is_browser_tool("rzn.worker.shutdown"));
        assert!(!is_browser_tool("runtime.status"));
        assert!(!is_browser_tool("shell.exec"));
    }

    #[tokio::test]
    async fn supervisor_requires_native_bridge_without_fallback() {
        let state = SupervisorState::new(test_config());
        let response = state
            .dispatch("browser.session_open", json!({}))
            .await
            .expect("strict supervisor returns a structured browser-target error");
        assert_eq!(
            response.get("error_code").and_then(Value::as_str),
            Some("NO_BROWSER_BRIDGE_CONNECTED")
        );
        assert!(!response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("AMBIGUOUS_BROWSER_TARGET"));
    }

    #[tokio::test]
    async fn browser_tool_dispatch_retries_after_reconnect_readiness() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let state_for_bridge = state.clone();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
            state_for_bridge
                .register_native_bridge("late-bridge".to_string(), tx)
                .await;
            let bytes = rx.recv().await.expect("readiness ping frame");
            let request: Value = serde_json::from_slice(&bytes).expect("readiness ping json");
            let id = request.get("id").cloned().expect("request id");
            let response = current_bridge_ping_response(id, "fresh-build");
            assert!(
                complete_bridge_response_for_test(&state_for_bridge, "late-bridge", &response)
                    .await
            );
        });

        let response = state
            .dispatch("browser.session_open", json!({}))
            .await
            .expect("dispatch should retry after bridge readiness recovers");
        assert_eq!(response.get("ok").and_then(Value::as_bool), Some(true));
        assert!(!response
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty());
    }

    #[test]
    fn static_command_allowlist_is_explicit() {
        assert!(is_allowed_static_command("observe"));
        assert!(is_allowed_static_command("enable_debug"));
        assert!(!is_allowed_static_command("execute_step"));
        assert!(!is_allowed_static_command("runtime.shutdown"));
    }

    #[tokio::test]
    async fn static_command_requires_native_bridge_without_fallback() {
        let state = SupervisorState::new(test_config());
        let response = state
            .dispatch(
                "browser.static_command",
                json!({ "cmd": "observe", "payload": {} }),
            )
            .await
            .expect("static compatibility command returns structured browser-target error");
        assert_eq!(
            response.get("error_code").and_then(Value::as_str),
            Some("NO_BROWSER_BRIDGE_CONNECTED")
        );
    }

    #[tokio::test]
    async fn readiness_classifies_bridge_down() {
        let state = SupervisorState::new(test_config());

        let readiness = state
            .ensure_ready(json!({
                "bridge_wait_ms": 0
            }))
            .await
            .expect("readiness returns structured failure");

        assert_eq!(
            readiness.pointer("/diagnostic/cause"),
            Some(&json!(READINESS_CAUSE_BRIDGE_DOWN))
        );
        assert_eq!(
            readiness.pointer("/native_host_bridge/cause"),
            Some(&json!(READINESS_CAUSE_BRIDGE_DOWN))
        );
        assert!(readiness
            .pointer("/diagnostic/action_text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("Open the existing Chrome profile"));
    }

    #[tokio::test]
    async fn readiness_classifies_stale_bundle_with_loaded_metadata() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("stale-extension".to_string(), tx)
            .await;

        let state_for_response = state.clone();
        tokio::spawn(async move {
            let bytes = rx.recv().await.expect("native bridge request");
            let request: Value = serde_json::from_slice(&bytes).expect("json request");
            let id = request.get("id").cloned().expect("request id");
            let response = json!({
                "id": id,
                "result": {
                    "success": true,
                    "result": {
                        "pong": true,
                        "extension_build_signature": "old-build",
                        "capabilities": {}
                    }
                }
            });
            assert!(
                complete_bridge_response_for_test(
                    &state_for_response,
                    "stale-extension",
                    &response
                )
                .await
            );
        });

        let readiness = state
            .ensure_ready(json!({
                "bridge_wait_ms": 0,
                "bridge_probe_timeout_ms": 250
            }))
            .await
            .expect("readiness returns stale bundle diagnostic");

        assert_eq!(
            readiness.pointer("/diagnostic/cause"),
            Some(&json!(READINESS_CAUSE_STALE_EXTENSION_BUNDLE))
        );
        assert_eq!(
            readiness.pointer("/native_host_bridge/transport_ok"),
            Some(&json!(true))
        );
        assert_eq!(
            readiness.pointer("/native_host_bridge/required_capabilities_ok"),
            Some(&json!(false))
        );
        assert_eq!(
            readiness.pointer("/native_host_bridge/capability_policy/mode"),
            Some(&json!("global_readiness_gate"))
        );
        assert_eq!(
            readiness.pointer("/diagnostic/observed/loaded_extension_build_signature"),
            Some(&json!("old-build"))
        );
        assert_eq!(
            readiness.pointer("/diagnostic/observed/expected_capabilities/content_keepalive_port"),
            Some(&json!(true))
        );
        assert_eq!(
            readiness.pointer("/diagnostic/observed/loaded_capabilities"),
            Some(&json!({}))
        );
        assert!(readiness
            .pointer("/diagnostic/action_text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("extension/dist/chrome"));
    }

    #[test]
    fn readiness_classifies_service_worker_unresponsive() {
        let diagnostic = bridge_readiness_diagnostic(
            true,
            false,
            Some(&json!({
                "ok": false,
                "transport_ok": false,
                "error": "native-host bridge disappeared before ping"
            })),
        );

        assert_eq!(
            diagnostic.get("cause"),
            Some(&json!(READINESS_CAUSE_SERVICE_WORKER_UNRESPONSIVE))
        );
        assert!(diagnostic
            .get("action_text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("service worker"));
    }

    #[tokio::test]
    async fn readiness_does_not_treat_skipped_probe_as_capability_ok() {
        let state = SupervisorState::new(test_config());
        let (tx, _rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("unverified-bridge".to_string(), tx)
            .await;

        let readiness = state
            .ensure_ready(json!({
                "bridge_wait_ms": 0,
                "verify_bridge": false
            }))
            .await
            .expect("readiness returns structured unverified state");

        assert_eq!(readiness.get("ready").and_then(Value::as_bool), Some(false));
        assert_eq!(
            readiness.pointer("/native_host_bridge/probe"),
            Some(&Value::Null)
        );
        assert_eq!(
            readiness.pointer("/native_host_bridge/transport_ok"),
            Some(&json!(false))
        );
        assert_eq!(
            readiness.pointer("/native_host_bridge/required_capabilities_ok"),
            Some(&json!(false))
        );
        assert_eq!(
            readiness.pointer("/diagnostic/cause"),
            Some(&json!(READINESS_CAUSE_SERVICE_WORKER_UNRESPONSIVE))
        );
    }

    #[tokio::test]
    async fn readiness_bootstraps_single_unknown_bridge_for_explicit_browser_target() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("unknown-chrome".to_string(), tx)
            .await;

        let state_for_response = state.clone();
        tokio::spawn(async move {
            let bytes = rx.recv().await.expect("readiness ping frame");
            let request: Value = serde_json::from_slice(&bytes).expect("json request");
            assert_eq!(
                request.pointer("/params/resolved_browser_target/source"),
                Some(&json!("browser_bootstrap_single_bridge"))
            );
            let id = request.get("id").cloned().expect("request id");
            let response = current_bridge_ping_response(id, "fresh-build");
            assert!(
                complete_bridge_response_for_test(&state_for_response, "unknown-chrome", &response)
                    .await
            );
        });

        let readiness = state
            .ensure_ready(json!({
                "browser_target": { "browser": "chrome" },
                "bridge_wait_ms": 0,
                "bridge_probe_timeout_ms": 250
            }))
            .await
            .expect("readiness should bootstrap the single fresh bridge");

        assert_eq!(readiness.get("ready"), Some(&json!(true)));
        assert_eq!(
            readiness.pointer("/native_host_bridge/target_ok"),
            Some(&json!(true))
        );
        assert_eq!(
            readiness.pointer("/native_host_bridge/probe/target_match_ok"),
            Some(&json!(true))
        );
        assert_eq!(
            readiness.pointer("/native_host_bridge/probe/response/resolved_browser_target/source"),
            Some(&json!("browser_bootstrap_single_bridge"))
        );
        let bridges = state.native_bridges.lock().await;
        assert_eq!(
            bridges
                .get("unknown-chrome")
                .expect("bridge remains registered")
                .metadata
                .extension_target
                .as_deref(),
            Some("chrome")
        );
    }

    #[tokio::test]
    async fn readiness_bootstraps_single_unknown_bridge_with_known_nonmatching_bridge_connected() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "known-edge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("unknown-chrome".to_string(), chrome_tx)
            .await;

        let state_for_response = state.clone();
        tokio::spawn(async move {
            let bytes = chrome_rx.recv().await.expect("unknown chrome ping frame");
            let request: Value = serde_json::from_slice(&bytes).expect("json request");
            assert_eq!(
                request.pointer("/params/resolved_browser_target/source"),
                Some(&json!("browser_bootstrap_single_bridge"))
            );
            let id = request.get("id").cloned().expect("request id");
            let response = current_bridge_ping_response(id, "fresh-build");
            assert!(
                complete_bridge_response_for_test(&state_for_response, "unknown-chrome", &response)
                    .await
            );
        });

        let readiness = state
            .ensure_ready(json!({
                "browser_target": { "browser": "chrome" },
                "bridge_wait_ms": 0,
                "bridge_probe_timeout_ms": 250
            }))
            .await
            .expect("readiness should bootstrap the one unknown bridge");

        assert_eq!(readiness.get("ready"), Some(&json!(true)));
        assert!(timeout(Duration::from_millis(50), edge_rx.recv())
            .await
            .is_err());
        assert_eq!(
            readiness
                .pointer("/native_host_bridge/probe/response/resolved_browser_target/bridge_id"),
            Some(&json!("unknown-chrome"))
        );
    }

    #[tokio::test]
    async fn readiness_probes_multiple_unknown_bridges_then_resolves_requested_browser() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("unknown-edge".to_string(), edge_tx)
            .await;
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("unknown-chrome".to_string(), chrome_tx)
            .await;

        let state_for_edge = state.clone();
        tokio::spawn(async move {
            let bytes = edge_rx.recv().await.expect("edge identity ping frame");
            let request: Value = serde_json::from_slice(&bytes).expect("edge ping json");
            let id = request.get("id").cloned().expect("edge request id");
            let epoch = request
                .pointer("/params/supervisor_bridge_epoch")
                .cloned()
                .expect("edge bridge epoch");
            let mut response = current_bridge_ping_response(id, "edge-build");
            response["result"]["result"]["supervisor_bridge_epoch"] = epoch;
            response["result"]["result"]["browser_instance_id"] = json!("edge-instance-test");
            response["result"]["result"]["extension_target"] = json!("edge");
            response["result"]["result"]["extension_target_hint"] = json!("edge-mv3");
            assert!(
                complete_bridge_response_for_test(&state_for_edge, "unknown-edge", &response).await
            );
        });

        let state_for_chrome = state.clone();
        tokio::spawn(async move {
            for _ in 0..2 {
                let bytes = chrome_rx.recv().await.expect("chrome ping frame");
                let request: Value = serde_json::from_slice(&bytes).expect("chrome ping json");
                let id = request.get("id").cloned().expect("chrome request id");
                let epoch = request
                    .pointer("/params/supervisor_bridge_epoch")
                    .cloned()
                    .expect("chrome bridge epoch");
                let mut response = current_bridge_ping_response(id, "chrome-build");
                response["result"]["result"]["supervisor_bridge_epoch"] = epoch;
                assert!(
                    complete_bridge_response_for_test(
                        &state_for_chrome,
                        "unknown-chrome",
                        &response
                    )
                    .await
                );
            }
        });

        let readiness = state
            .ensure_ready(json!({
                "browser_target": { "browser": "chrome" },
                "bridge_wait_ms": 0,
                "bridge_probe_timeout_ms": 250
            }))
            .await
            .expect("readiness should identify unknown bridges and route chrome");

        assert_eq!(readiness.get("ready"), Some(&json!(true)));
        assert_eq!(
            readiness
                .pointer("/native_host_bridge/probe/response/resolved_browser_target/bridge_id"),
            Some(&json!("unknown-chrome"))
        );
    }

    #[tokio::test]
    async fn readiness_reports_browser_target_mismatch_after_bootstrap_probe() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("unknown-edge".to_string(), tx)
            .await;

        let state_for_response = state.clone();
        tokio::spawn(async move {
            let bytes = rx.recv().await.expect("readiness ping frame");
            let request: Value = serde_json::from_slice(&bytes).expect("json request");
            let id = request.get("id").cloned().expect("request id");
            let mut response = current_bridge_ping_response(id, "fresh-build");
            response["result"]["result"]["browser_instance_id"] = json!("edge-instance-test");
            response["result"]["result"]["extension_target"] = json!("edge");
            response["result"]["result"]["extension_target_hint"] = json!("edge-mv3");
            assert!(
                complete_bridge_response_for_test(&state_for_response, "unknown-edge", &response)
                    .await
            );
        });

        let readiness = state
            .ensure_ready(json!({
                "browser_target": { "browser": "chrome" },
                "bridge_wait_ms": 0,
                "bridge_probe_timeout_ms": 250
            }))
            .await
            .expect("readiness should return a target mismatch diagnostic");

        assert_eq!(readiness.get("ready"), Some(&json!(false)));
        assert_eq!(
            readiness.pointer("/diagnostic/cause"),
            Some(&json!(READINESS_CAUSE_BROWSER_TARGET_MISMATCH))
        );
        assert_eq!(
            readiness.pointer("/native_host_bridge/target_ok"),
            Some(&json!(false))
        );
        assert_eq!(
            readiness.pointer("/native_host_bridge/probe/target_match_ok"),
            Some(&json!(false))
        );
        assert_eq!(
            readiness.pointer("/diagnostic/observed/requested_browser"),
            Some(&json!("chrome"))
        );
        assert_eq!(
            readiness.pointer("/diagnostic/observed/loaded_browser"),
            Some(&json!("edge"))
        );
        let action = readiness
            .pointer("/diagnostic/action_text")
            .and_then(Value::as_str)
            .unwrap_or("");
        assert!(action.contains("browser targets"));
        assert!(!action.contains("service worker"));
    }

    #[tokio::test]
    async fn readiness_reports_unresolved_browser_target_without_service_worker_hint() {
        let state = SupervisorState::new(test_config());
        let (first_tx, _first_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("fresh-one".to_string(), first_tx)
            .await;
        let (second_tx, _second_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("fresh-two".to_string(), second_tx)
            .await;

        let readiness = state
            .ensure_ready(json!({
                "browser_target": { "browser": "chrome" },
                "bridge_wait_ms": 0,
                "bridge_probe_timeout_ms": 250
            }))
            .await
            .expect("readiness should return a target resolution diagnostic");

        assert_eq!(readiness.get("ready"), Some(&json!(false)));
        assert_eq!(
            readiness.pointer("/diagnostic/cause"),
            Some(&json!(READINESS_CAUSE_BROWSER_TARGET_UNRESOLVED))
        );
        assert_eq!(
            readiness.pointer("/native_host_bridge/probe/error_code"),
            Some(&json!("BROWSER_INSTANCE_NOT_CONNECTED"))
        );
        let action = readiness
            .pointer("/diagnostic/action_text")
            .and_then(Value::as_str)
            .unwrap_or("");
        assert!(action.contains("browser targets"));
        assert!(!action.contains("service worker"));
    }

    #[tokio::test]
    async fn native_bridge_retry_timeout_clears_reconnected_bridge() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (old_tx, old_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        drop(old_rx);
        state
            .register_native_bridge("old-bridge".to_string(), old_tx)
            .await;

        let state_for_reconnect = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let (new_tx, mut new_rx) = mpsc::unbounded_channel::<Vec<u8>>();
            state_for_reconnect
                .register_native_bridge("new-bridge".to_string(), new_tx)
                .await;
            let _ = new_rx.recv().await;
            tokio::time::sleep(Duration::from_millis(250)).await;
        });

        let err = state
            .try_call_native_bridge(
                "ping",
                json!({
                    "timeout_ms": 10,
                    "timeout_grace_ms": 0
                }),
            )
            .await
            .expect_err("reconnected bridge should time out without a response");

        assert!(err
            .to_string()
            .contains("Native-host extension bridge timeout after 10ms"));
        assert!(state.native_bridges.lock().await.is_empty());
    }

    #[tokio::test]
    async fn native_bridge_timeout_requests_native_host_shutdown() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("zombie-bridge".to_string(), tx)
            .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw(
                    "execute_step",
                    json!({ "step": { "type": "navigate_to_url" } }),
                    None,
                    Some(10),
                    None,
                )
                .await
        });

        let first = rx.recv().await.expect("extension call frame");
        let first: Value = serde_json::from_slice(&first).expect("extension call json");
        assert_eq!(
            first.get("method").and_then(Value::as_str),
            Some("native_host.extension_call")
        );

        let shutdown = timeout(Duration::from_millis(250), rx.recv())
            .await
            .expect("shutdown frame should be sent after timeout")
            .expect("shutdown frame");
        let shutdown: Value = serde_json::from_slice(&shutdown).expect("shutdown json");
        assert_eq!(
            shutdown.get("method").and_then(Value::as_str),
            Some(NATIVE_HOST_SHUTDOWN_METHOD)
        );
        assert!(shutdown
            .pointer("/params/reason")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("execute_step"));

        let err = call
            .await
            .expect("call task completes")
            .expect_err("unanswered extension call times out");
        assert!(err
            .to_string()
            .contains("Native-host extension bridge timeout after 10ms"));
        assert!(state.native_bridges.lock().await.is_empty());
        let health_by_bridge = state.native_bridge_health.lock().await;
        assert_eq!(
            health_by_bridge
                .get("zombie-bridge")
                .expect("zombie bridge health")
                .last_failure_cause
                .as_deref(),
            Some(READINESS_CAUSE_ZOMBIE_NATIVE_HOST)
        );
    }

    #[tokio::test]
    async fn native_host_extension_timeout_error_requests_shutdown() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("native-timeout-bridge".to_string(), tx)
            .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw(
                    "execute_step",
                    json!({ "step": { "type": "navigate_to_url" } }),
                    None,
                    Some(1_000),
                    None,
                )
                .await
        });

        let first = rx.recv().await.expect("extension call frame");
        let first: Value = serde_json::from_slice(&first).expect("extension call json");
        let request_id = first
            .get("id")
            .and_then(value_id_string)
            .expect("extension call id");

        let response = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "error": {
                "code": -32003,
                "message": "Extension timeout after 1000ms"
            }
        });
        assert!(
            complete_bridge_response_for_test(&state, "native-timeout-bridge", &response).await
        );

        let shutdown = timeout(Duration::from_millis(250), rx.recv())
            .await
            .expect("shutdown frame should be sent after native-host timeout error")
            .expect("shutdown frame");
        let shutdown: Value = serde_json::from_slice(&shutdown).expect("shutdown json");
        assert_eq!(
            shutdown.get("method").and_then(Value::as_str),
            Some(NATIVE_HOST_SHUTDOWN_METHOD)
        );

        let err = call
            .await
            .expect("call task completes")
            .expect_err("native-host timeout error is surfaced");
        assert!(err
            .to_string()
            .contains("Native-host extension bridge error"));
        assert!(state.native_bridges.lock().await.is_empty());
        let health_by_bridge = state.native_bridge_health.lock().await;
        assert_eq!(
            health_by_bridge
                .get("native-timeout-bridge")
                .expect("native timeout bridge health")
                .last_failure_cause
                .as_deref(),
            Some(READINESS_CAUSE_ZOMBIE_NATIVE_HOST)
        );
    }

    #[tokio::test]
    async fn native_bridge_registry_keeps_other_bridges_and_pending_calls() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (old_tx, _old_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let old_epoch = state
            .register_native_bridge("old-bridge".to_string(), old_tx)
            .await;
        let (fresh_tx, mut fresh_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let fresh_epoch = state
            .register_native_bridge("fresh-bridge".to_string(), fresh_tx)
            .await;

        assert_eq!(state.native_bridges.lock().await.len(), 2);

        let (old_pending_tx, old_pending_rx) = oneshot::channel();
        let (fresh_pending_tx, _fresh_pending_rx) = oneshot::channel();
        {
            let mut pending = state.native_bridge_pending.lock().await;
            pending.insert(
                "old-call".to_string(),
                PendingNativeCall {
                    bridge_id: "old-bridge".to_string(),
                    bridge_epoch: old_epoch,
                    method: "execute_step".to_string(),
                    deadline_at_ms: now_ms() + 1_000,
                    responder: old_pending_tx,
                },
            );
            pending.insert(
                "fresh-call".to_string(),
                PendingNativeCall {
                    bridge_id: "fresh-bridge".to_string(),
                    bridge_epoch: fresh_epoch,
                    method: "execute_step".to_string(),
                    deadline_at_ms: now_ms() + 1_000,
                    responder: fresh_pending_tx,
                },
            );
        }

        state.clear_native_bridge("old-bridge").await;

        assert!(old_pending_rx.await.is_ok());
        let bridges = state.native_bridges.lock().await;
        assert_eq!(bridges.len(), 1);
        assert!(bridges.contains_key("fresh-bridge"));
        drop(bridges);
        let pending = state.native_bridge_pending.lock().await;
        assert_eq!(pending.len(), 1);
        assert!(pending.contains_key("fresh-call"));
        drop(pending);

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw("ping", json!({}), None, Some(1_000), None)
                .await
        });
        let first = fresh_rx.recv().await.expect("fresh bridge receives call");
        let first: Value = serde_json::from_slice(&first).expect("extension call json");
        assert_eq!(
            first
                .pointer("/params/supervisor_bridge_id")
                .and_then(Value::as_str),
            Some("fresh-bridge")
        );
        let request_id = first
            .get("id")
            .and_then(value_id_string)
            .expect("extension call id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "fresh-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "result": { "pong": true }
                    }
                })
            )
            .await
        );
        let result = call
            .await
            .expect("task joins")
            .expect("call succeeds")
            .expect("bridge returned result");
        assert_eq!(result.pointer("/result/pong"), Some(&json!(true)));
    }

    #[tokio::test]
    async fn bridge_target_explicit_id_routes_only_to_selected_bridge() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let chrome_epoch = state
            .register_native_bridge("chrome-bridge".to_string(), chrome_tx)
            .await;
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("edge-bridge".to_string(), edge_tx)
            .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw_with_target(
                    "ping",
                    json!({}),
                    None,
                    Some(1_000),
                    None,
                    BridgeTarget::BridgeId("chrome-bridge".to_string()),
                )
                .await
        });

        let frame = chrome_rx.recv().await.expect("chrome bridge receives call");
        let edge_recv: Result<Option<Vec<u8>>, _> =
            timeout(Duration::from_millis(50), edge_rx.recv()).await;
        assert!(edge_recv.is_err());
        let request: Value = serde_json::from_slice(&frame).expect("extension call json");
        assert_eq!(
            request
                .pointer("/params/supervisor_bridge_id")
                .and_then(Value::as_str),
            Some("chrome-bridge")
        );
        assert_eq!(
            request
                .pointer("/params/supervisor_bridge_epoch")
                .and_then(Value::as_u64),
            Some(chrome_epoch)
        );
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("extension call id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "result": { "pong": true }
                    }
                })
            )
            .await
        );
        assert_eq!(
            call.await
                .expect("task joins")
                .expect("call succeeds")
                .expect("response")
                .pointer("/result/pong"),
            Some(&json!(true))
        );
    }

    #[tokio::test]
    async fn bridge_target_singleton_defaults_and_multi_bridge_default_is_ambiguous() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (only_tx, mut only_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("only-bridge".to_string(), only_tx)
            .await;

        let state_for_call = state.clone();
        let singleton = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw("ping", json!({}), None, Some(1_000), None)
                .await
        });
        let request: Value = serde_json::from_slice(
            &only_rx
                .recv()
                .await
                .expect("singleton bridge receives call"),
        )
        .expect("extension call json");
        assert_eq!(
            request
                .pointer("/params/supervisor_bridge_id")
                .and_then(Value::as_str),
            Some("only-bridge")
        );
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("extension call id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "only-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "result": { "pong": true }
                    }
                })
            )
            .await
        );
        assert!(singleton
            .await
            .expect("task joins")
            .expect("singleton call succeeds")
            .is_some());

        let (second_tx, mut second_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("second-bridge".to_string(), second_tx)
            .await;
        let err = state
            .try_call_native_bridge_raw("ping", json!({}), None, Some(1_000), None)
            .await
            .expect_err("multi-bridge default is ambiguous");
        assert!(err.to_string().contains("AMBIGUOUS_BROWSER_TARGET"));
        assert!(timeout(Duration::from_millis(50), only_rx.recv())
            .await
            .is_err());
        assert!(timeout(Duration::from_millis(50), second_rx.recv())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn browser_target_bridge_id_selects_or_reports_missing() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_identity_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
        )
        .await;
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw(
                    "ping",
                    json!({ "bridge_id": "edge-bridge" }),
                    None,
                    Some(1_000),
                    None,
                )
                .await
        });

        let edge_frame: Vec<u8> = edge_rx.recv().await.expect("edge bridge receives call");
        let request: Value = serde_json::from_slice(&edge_frame).expect("extension call json");
        assert!(timeout(Duration::from_millis(50), chrome_rx.recv())
            .await
            .is_err());
        assert_eq!(
            request
                .pointer("/params/resolved_browser_target/source")
                .and_then(Value::as_str),
            Some("bridge_id")
        );
        assert_eq!(
            request
                .pointer("/params/payload/resolved_browser_target/bridge_id")
                .and_then(Value::as_str),
            Some("edge-bridge")
        );
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("extension call id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "edge-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "result": { "pong": true }
                    }
                })
            )
            .await
        );
        assert!(call
            .await
            .expect("task joins")
            .expect("call succeeds")
            .is_some());

        let err = state
            .try_call_native_bridge_raw(
                "ping",
                json!({ "bridge_id": "missing-bridge" }),
                None,
                Some(1_000),
                None,
            )
            .await
            .expect_err("missing bridge id is an error");
        assert!(err.to_string().contains("BRIDGE_NOT_FOUND"));
    }

    #[tokio::test]
    async fn browser_target_browser_instance_selects_active_bridge_or_reports_missing() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw(
                    "ping",
                    json!({ "browser_instance_id": "chrome-instance" }),
                    None,
                    Some(1_000),
                    None,
                )
                .await
        });

        let request: Value = serde_json::from_slice(
            &chrome_rx
                .recv()
                .await
                .expect("chrome instance bridge receives call"),
        )
        .expect("extension call json");
        let edge_recv: Result<Option<Vec<u8>>, _> =
            timeout(Duration::from_millis(50), edge_rx.recv()).await;
        assert!(edge_recv.is_err());
        assert_eq!(
            request
                .pointer("/params/resolved_browser_target/source")
                .and_then(Value::as_str),
            Some("browser_instance_id")
        );
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("extension call id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "result": { "pong": true }
                    }
                })
            )
            .await
        );
        assert!(call
            .await
            .expect("task joins")
            .expect("call succeeds")
            .is_some());

        let err = state
            .try_call_native_bridge_raw(
                "ping",
                json!({ "browser_instance_id": "missing-instance" }),
                None,
                Some(1_000),
                None,
            )
            .await
            .expect_err("missing browser instance is an error");
        assert!(err.to_string().contains("BROWSER_INSTANCE_NOT_CONNECTED"));
    }

    #[tokio::test]
    async fn browser_target_kind_selects_unique_match_or_reports_candidates() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw(
                    "ping",
                    json!({ "browser": "edge" }),
                    None,
                    Some(1_000),
                    None,
                )
                .await
        });

        let request: Value =
            serde_json::from_slice(&edge_rx.recv().await.expect("edge browser receives call"))
                .expect("extension call json");
        assert!(timeout(Duration::from_millis(50), chrome_rx.recv())
            .await
            .is_err());
        assert_eq!(
            request
                .pointer("/params/resolved_browser_target/source")
                .and_then(Value::as_str),
            Some("browser")
        );
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("extension call id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "edge-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "result": { "pong": true }
                    }
                })
            )
            .await
        );
        assert!(call
            .await
            .expect("task joins")
            .expect("call succeeds")
            .is_some());

        let (edge_two_tx, _edge_two_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge-two",
            edge_two_tx,
            "edge-instance-two",
            "edge",
        )
        .await;
        let err = state
            .try_call_native_bridge_raw(
                "ping",
                json!({ "browser": "edge" }),
                None,
                Some(1_000),
                None,
            )
            .await
            .expect_err("multiple edge bridges are ambiguous");
        let error = err.to_string();
        assert!(error.contains("AMBIGUOUS_BROWSER_TARGET"));
        assert!(error.contains("edge-bridge"));
        assert!(error.contains("edge-bridge-two"));
    }

    #[tokio::test]
    async fn browser_target_error_no_bridge_and_missing_targets_are_structured() {
        let empty_state = SupervisorState::new(test_config());
        let no_bridge = empty_state
            .dispatch(
                "browser.execute_step",
                json!({ "step": { "type": "get_current_url" } }),
            )
            .await
            .expect("no bridge returns structured error result");
        assert_eq!(
            no_bridge.get("error_code").and_then(Value::as_str),
            Some("NO_BROWSER_BRIDGE_CONNECTED")
        );
        assert!(!no_bridge
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("AMBIGUOUS_BROWSER_TARGET"));

        let state = SupervisorState::new(test_config());
        let (chrome_tx, _chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;

        let missing_bridge = state
            .dispatch(
                "browser.execute_step",
                json!({
                    "bridge_id": "missing-bridge",
                    "step": { "type": "get_current_url" }
                }),
            )
            .await
            .expect("missing bridge returns structured error result");
        assert_eq!(
            missing_bridge.get("error_code").and_then(Value::as_str),
            Some("BRIDGE_NOT_FOUND")
        );

        let missing_instance = state
            .dispatch(
                "browser.execute_step",
                json!({
                    "browser_instance_id": "missing-instance",
                    "step": { "type": "get_current_url" }
                }),
            )
            .await
            .expect("missing browser instance returns structured error result");
        assert_eq!(
            missing_instance.get("error_code").and_then(Value::as_str),
            Some("BROWSER_INSTANCE_NOT_CONNECTED")
        );
    }

    #[tokio::test]
    async fn browser_target_error_ambiguous_result_includes_candidate_payloads() {
        let state = SupervisorState::new(test_config());
        let (chrome_tx, _chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;
        state
            .record_native_bridge_ping_success(
                "chrome-bridge",
                &json!({
                    "result": {
                        "extension_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "browser_instance_id": "chrome-instance",
                        "extension_target": "chrome",
                        "extension_target_hint": "chromium-mv3",
                        "extension_build_signature": "chrome-build",
                        "bridge_contract_version": EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION
                    }
                }),
                7,
            )
            .await;

        let (edge_tx, _edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;
        state
            .record_native_bridge_ping_success(
                "edge-bridge",
                &json!({
                    "result": {
                        "extension_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        "browser_instance_id": "edge-instance",
                        "extension_target": "edge",
                        "extension_target_hint": "edge-mv3",
                        "extension_build_signature": "edge-build",
                        "bridge_contract_version": EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION
                    }
                }),
                9,
            )
            .await;
        let (edge_two_tx, _edge_two_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge-two",
            edge_two_tx,
            "edge-instance-two",
            "edge",
        )
        .await;
        state
            .record_native_bridge_ping_success(
                "edge-bridge-two",
                &json!({
                    "result": {
                        "extension_id": "cccccccccccccccccccccccccccccccc",
                        "browser_instance_id": "edge-instance-two",
                        "extension_target": "edge",
                        "extension_target_hint": "edge-mv3",
                        "extension_build_signature": "edge-build-two",
                        "bridge_contract_version": EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION
                    }
                }),
                11,
            )
            .await;

        let response = state
            .dispatch(
                "browser.execute_step",
                json!({
                    "browser": "edge",
                    "step": { "type": "get_current_url" }
                }),
            )
            .await
            .expect("ambiguous target returns structured result");
        assert_eq!(
            response.get("error_code").and_then(Value::as_str),
            Some("AMBIGUOUS_BROWSER_TARGET")
        );
        let candidates = response
            .get("candidates")
            .and_then(Value::as_array)
            .expect("candidate list");
        assert_eq!(candidates.len(), 2);
        let edge = candidates
            .iter()
            .find(|candidate| {
                candidate.get("bridge_id").and_then(Value::as_str) == Some("edge-bridge")
            })
            .expect("edge candidate");
        assert_eq!(
            edge.get("browser_instance_id").and_then(Value::as_str),
            Some("edge-instance")
        );
        assert_eq!(
            edge.get("extension_target_hint").and_then(Value::as_str),
            Some("edge-mv3")
        );
        assert_eq!(
            edge.get("extension_id").and_then(Value::as_str),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
        assert!(edge
            .get("last_health_at_ms")
            .and_then(Value::as_u64)
            .is_some());
        let next_steps = response
            .get("next_steps")
            .and_then(Value::as_array)
            .expect("next steps");
        let joined = next_steps
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("--browser "));
        assert!(joined.contains("--browser-instance "));
        assert!(joined.contains("--bridge "));
    }

    #[tokio::test]
    async fn tab_ref_is_added_when_result_has_tab_id_and_browser_instance() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw(
                    "ping",
                    json!({ "browser_instance_id": "chrome-instance" }),
                    None,
                    Some(1_000),
                    None,
                )
                .await
        });

        let request: Value =
            serde_json::from_slice(&chrome_rx.recv().await.expect("chrome bridge receives call"))
                .expect("extension call json");
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "current_tab_id": 123
                    }
                })
            )
            .await
        );

        let response = call
            .await
            .expect("task joins")
            .expect("call succeeds")
            .expect("response");
        assert_eq!(response.get("current_tab_id"), Some(&json!(123)));
        assert_eq!(
            response.get("tab_ref").and_then(Value::as_str),
            Some("rzn://browser/chrome-instance/tab/123")
        );
        assert_eq!(
            response.get("current_tab_ref").and_then(Value::as_str),
            Some("rzn://browser/chrome-instance/tab/123")
        );
        assert_eq!(
            response
                .pointer("/run_result/output/tab_ref")
                .and_then(Value::as_str),
            Some("rzn://browser/chrome-instance/tab/123")
        );
    }

    #[tokio::test]
    async fn tab_ref_is_added_to_open_new_tab_result_output() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw(
                    "execute_step",
                    json!({
                        "browser_instance_id": "chrome-instance",
                        "step": { "type": "open_new_tab", "url": "https://example.test/" }
                    }),
                    None,
                    Some(1_000),
                    None,
                )
                .await
        });

        let request: Value =
            serde_json::from_slice(&chrome_rx.recv().await.expect("chrome bridge receives call"))
                .expect("extension call json");
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "result": {
                            "tabId": 123,
                            "opened": true,
                            "url": "https://example.test/"
                        },
                        "current_tab_id": 123
                    }
                })
            )
            .await
        );

        let response = call
            .await
            .expect("task joins")
            .expect("call succeeds")
            .expect("response");
        let expected = "rzn://browser/chrome-instance/tab/123";
        assert_eq!(response.pointer("/result/tab_id"), Some(&json!(123)));
        assert_eq!(
            response
                .pointer("/result/browser_instance_id")
                .and_then(Value::as_str),
            Some("chrome-instance")
        );
        assert_eq!(
            response
                .pointer("/result/bridge_id")
                .and_then(Value::as_str),
            Some("chrome-bridge")
        );
        assert_eq!(
            response.pointer("/result/tab_ref").and_then(Value::as_str),
            Some(expected)
        );
        assert_eq!(
            response
                .pointer("/run_result/output/tab_ref")
                .and_then(Value::as_str),
            Some(expected)
        );
        assert_eq!(response.get("current_tab_id"), Some(&json!(123)));
    }

    #[tokio::test]
    async fn tab_ref_distinguishes_same_numeric_tab_id_across_browsers() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;

        let chrome_state = state.clone();
        let chrome_call = tokio::spawn(async move {
            chrome_state
                .try_call_native_bridge_raw(
                    "execute_step",
                    json!({ "browser": "chrome", "step": { "type": "open_new_tab" } }),
                    None,
                    Some(1_000),
                    None,
                )
                .await
        });
        let chrome_request: Value =
            serde_json::from_slice(&chrome_rx.recv().await.expect("chrome request"))
                .expect("chrome request json");
        let chrome_request_id = chrome_request
            .get("id")
            .and_then(value_id_string)
            .expect("chrome request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": chrome_request_id,
                    "result": {
                        "success": true,
                        "result": { "tab_id": 7, "opened": true },
                        "current_tab_id": 7
                    }
                })
            )
            .await
        );
        let chrome_response = chrome_call
            .await
            .expect("chrome task joins")
            .expect("chrome call succeeds")
            .expect("chrome response");

        let edge_state = state.clone();
        let edge_call = tokio::spawn(async move {
            edge_state
                .try_call_native_bridge_raw(
                    "execute_step",
                    json!({ "browser": "edge", "step": { "type": "open_new_tab" } }),
                    None,
                    Some(1_000),
                    None,
                )
                .await
        });
        let edge_request: Value =
            serde_json::from_slice(&edge_rx.recv().await.expect("edge request"))
                .expect("edge request json");
        let edge_request_id = edge_request
            .get("id")
            .and_then(value_id_string)
            .expect("edge request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "edge-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": edge_request_id,
                    "result": {
                        "success": true,
                        "result": { "tab_id": 7, "opened": true },
                        "current_tab_id": 7
                    }
                })
            )
            .await
        );
        let edge_response = edge_call
            .await
            .expect("edge task joins")
            .expect("edge call succeeds")
            .expect("edge response");

        assert_eq!(
            chrome_response
                .pointer("/result/tab_ref")
                .and_then(Value::as_str),
            Some("rzn://browser/chrome-instance/tab/7")
        );
        assert_eq!(
            edge_response
                .pointer("/result/tab_ref")
                .and_then(Value::as_str),
            Some("rzn://browser/edge-instance/tab/7")
        );
        assert_ne!(
            chrome_response.pointer("/result/tab_ref"),
            edge_response.pointer("/result/tab_ref")
        );
    }

    #[tokio::test]
    async fn tab_ref_is_added_to_snapshot_result_when_current_tab_is_known() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw(
                    "get_dom_snapshot",
                    json!({ "browser_instance_id": "chrome-instance" }),
                    None,
                    Some(1_000),
                    None,
                )
                .await
        });

        let request: Value =
            serde_json::from_slice(&chrome_rx.recv().await.expect("chrome bridge receives call"))
                .expect("extension call json");
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "result": { "dom_hash": "abc123" },
                        "current_tab_id": 55
                    }
                })
            )
            .await
        );

        let response = call
            .await
            .expect("task joins")
            .expect("call succeeds")
            .expect("response");
        let expected = "rzn://browser/chrome-instance/tab/55";
        assert_eq!(response.pointer("/result/current_tab_id"), Some(&json!(55)));
        assert_eq!(
            response
                .pointer("/result/current_tab_ref")
                .and_then(Value::as_str),
            Some(expected)
        );
        assert_eq!(
            response
                .pointer("/run_result/output/current_tab_ref")
                .and_then(Value::as_str),
            Some(expected)
        );
    }

    #[tokio::test]
    async fn tab_ref_input_routes_by_instance_and_forwards_numeric_tab_only() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;
        let (edge_tx, _edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw(
                    "execute_step",
                    json!({
                        "tab_ref": "rzn://browser/chrome-instance/tab/321",
                        "step": { "type": "click" }
                    }),
                    None,
                    Some(1_000),
                    None,
                )
                .await
        });

        let request: Value =
            serde_json::from_slice(&chrome_rx.recv().await.expect("chrome bridge receives call"))
                .expect("extension call json");
        assert_eq!(
            request
                .pointer("/params/resolved_browser_target/browser_instance_id")
                .and_then(Value::as_str),
            Some("chrome-instance")
        );
        assert_eq!(
            request
                .pointer("/params/payload/current_tab_id")
                .and_then(Value::as_u64),
            Some(321)
        );
        assert_eq!(
            request
                .pointer("/params/payload/tab_id")
                .and_then(Value::as_u64),
            Some(321)
        );
        assert!(request.pointer("/params/payload/tab_ref").is_none());

        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": { "success": true, "current_tab_id": 321 }
                })
            )
            .await
        );
        let response = call
            .await
            .expect("task joins")
            .expect("call succeeds")
            .expect("response");
        assert_eq!(
            response.get("tab_ref").and_then(Value::as_str),
            Some("rzn://browser/chrome-instance/tab/321")
        );
    }

    #[tokio::test]
    async fn tab_ref_input_numeric_tab_id_with_browser_target_is_forwarded_after_resolution() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, _chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw(
                    "execute_step",
                    json!({
                        "browser": "edge",
                        "current_tab_id": 123,
                        "step": { "type": "click" }
                    }),
                    None,
                    Some(1_000),
                    None,
                )
                .await
        });

        let request: Value =
            serde_json::from_slice(&edge_rx.recv().await.expect("edge bridge receives call"))
                .expect("extension call json");
        assert_eq!(
            request
                .pointer("/params/resolved_browser_target/browser")
                .and_then(Value::as_str),
            Some("edge")
        );
        assert_eq!(
            request
                .pointer("/params/payload/current_tab_id")
                .and_then(Value::as_u64),
            Some(123)
        );
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "edge-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": { "success": true, "current_tab_id": 123 }
                })
            )
            .await
        );
        assert!(call
            .await
            .expect("task joins")
            .expect("call succeeds")
            .is_some());
    }

    #[tokio::test]
    async fn tab_ref_input_numeric_tab_id_without_target_is_ambiguous_with_multiple_bridges() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;

        let response = state
            .try_dispatch_supervisor_browser_tool(
                "browser.execute_step",
                json!({
                    "current_tab_id": 123,
                    "step": { "type": "click" }
                }),
            )
            .await
            .expect("dispatch returns structured error")
            .expect("error response");
        assert_eq!(
            response.get("error_code").and_then(Value::as_str),
            Some("AMBIGUOUS_BROWSER_TARGET")
        );
        assert!(timeout(Duration::from_millis(50), chrome_rx.recv())
            .await
            .is_err());
        assert!(timeout(Duration::from_millis(50), edge_rx.recv())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn tab_ref_input_malformed_tab_ref_returns_invalid_tab_ref_error() {
        let state = SupervisorState::new(test_config());
        let response = state
            .try_dispatch_supervisor_browser_tool(
                "browser.execute_step",
                json!({
                    "tab_ref": "https://browser/chrome-instance/tab/nope",
                    "step": { "type": "click" }
                }),
            )
            .await
            .expect("dispatch returns structured error")
            .expect("error response");
        assert_eq!(
            response.get("error_code").and_then(Value::as_str),
            Some("INVALID_TAB_REF")
        );
        assert_eq!(
            response.get("format_example").and_then(Value::as_str),
            Some("rzn://browser/<browser_instance_id>/tab/123")
        );
    }

    #[tokio::test]
    async fn browser_targets_returns_empty_status_without_connected_bridges() {
        let state = SupervisorState::new(test_config());
        let response = state.browser_targets().await;

        assert_eq!(response.get("ok").and_then(Value::as_bool), Some(true));
        assert_eq!(
            response.get("status").and_then(Value::as_str),
            Some("no_bridges_connected")
        );
        assert_eq!(
            response.get("target_count").and_then(Value::as_u64),
            Some(0)
        );
        assert_eq!(
            response
                .get("targets")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
    }

    #[tokio::test]
    async fn browser_targets_lists_two_connected_bridges_with_target_identifiers() {
        let state = SupervisorState::new(test_config());
        let (chrome_tx, _chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;
        let (edge_tx, _edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;
        state
            .record_native_bridge_ping_success(
                "chrome-bridge",
                &json!({
                    "result": {
                        "extension_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "browser_instance_id": "chrome-instance",
                        "extension_target": "chrome",
                        "extension_target_hint": "chromium-mv3"
                    }
                }),
                7,
            )
            .await;
        state
            .record_native_bridge_ping_success(
                "edge-bridge",
                &json!({
                    "result": {
                        "extension_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        "browser_instance_id": "edge-instance",
                        "extension_target": "edge",
                        "extension_target_hint": "edge-mv3"
                    }
                }),
                9,
            )
            .await;

        let response = state
            .try_dispatch_supervisor_browser_tool("browser.targets", json!({}))
            .await
            .expect("browser.targets succeeds")
            .expect("browser targets response");
        assert_eq!(
            response.get("status").and_then(Value::as_str),
            Some("connected")
        );
        assert_eq!(
            response.get("target_count").and_then(Value::as_u64),
            Some(2)
        );
        let targets = response
            .get("targets")
            .and_then(Value::as_array)
            .expect("targets array");
        let chrome = targets
            .iter()
            .find(|target| target.get("bridge_id").and_then(Value::as_str) == Some("chrome-bridge"))
            .expect("chrome target");
        let edge = targets
            .iter()
            .find(|target| target.get("bridge_id").and_then(Value::as_str) == Some("edge-bridge"))
            .expect("edge target");

        assert_eq!(
            chrome.get("browser_instance_id").and_then(Value::as_str),
            Some("chrome-instance")
        );
        assert_eq!(
            chrome
                .pointer("/target_flags/bridge")
                .and_then(Value::as_str),
            Some("chrome-bridge")
        );
        assert_eq!(
            chrome
                .pointer("/target_flags/browser_instance")
                .and_then(Value::as_str),
            Some("chrome-instance")
        );
        assert_eq!(
            edge.get("extension_id").and_then(Value::as_str),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
        assert_eq!(
            edge.get("last_ping_status").and_then(Value::as_str),
            Some("ok")
        );
    }

    #[test]
    fn browser_target_flags_reserved_object_resolves_with_bridge_precedence() {
        assert_eq!(
            BridgeTarget::from_params(&json!({
                "browser_target": {
                    "bridge_id": "edge-bridge"
                }
            })),
            BridgeTarget::BridgeId("edge-bridge".to_string())
        );
        assert_eq!(
            BridgeTarget::from_params(&json!({
                "browser_target": {
                    "browser_instance_id": "edge-instance"
                }
            })),
            BridgeTarget::BrowserInstanceId("edge-instance".to_string())
        );
        assert_eq!(
            BridgeTarget::from_params(&json!({
                "browser_target": {
                    "browser": "microsoft-edge"
                }
            })),
            BridgeTarget::BrowserKind("edge".to_string())
        );
        assert_eq!(
            BridgeTarget::from_params(&json!({
                "browser_target": {
                    "preferred": {
                        "browser": "chromium"
                    },
                    "fallback": "single_connected"
                }
            })),
            BridgeTarget::Preferred(Box::new(BridgeTarget::BrowserKind("chromium".to_string())))
        );
    }

    #[tokio::test]
    async fn preferred_browser_target_uses_default_then_single_connected_fallback() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw(
                    "ping",
                    json!({
                        "browser_target": {
                            "preferred": { "browser": "chromium" },
                            "fallback": "single_connected"
                        }
                    }),
                    None,
                    Some(1_000),
                    None,
                )
                .await
        });

        let request: Value =
            serde_json::from_slice(&edge_rx.recv().await.expect("fallback bridge receives ping"))
                .expect("extension call json");
        assert_eq!(
            request
                .pointer("/params/resolved_browser_target/source")
                .and_then(Value::as_str),
            Some("single_bridge_default")
        );
        let request_id = request.get("id").cloned().expect("request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "edge-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "result": { "pong": true }
                    }
                })
            )
            .await
        );
        assert!(call
            .await
            .expect("task joins")
            .expect("call succeeds")
            .is_some());
    }

    #[tokio::test]
    async fn preferred_browser_target_resolves_saved_default_with_multiple_bridges() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw(
                    "ping",
                    json!({
                        "browser_target": {
                            "preferred": { "browser": "edge" },
                            "fallback": "single_connected"
                        }
                    }),
                    None,
                    Some(1_000),
                    None,
                )
                .await
        });
        let request: Value =
            serde_json::from_slice(&edge_rx.recv().await.expect("edge bridge receives call"))
                .expect("extension call json");
        assert!(timeout(Duration::from_millis(50), chrome_rx.recv())
            .await
            .is_err());
        assert_eq!(
            request
                .pointer("/params/resolved_browser_target/source")
                .and_then(Value::as_str),
            Some("preferred_browser")
        );
        let request_id = request.get("id").cloned().expect("request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "edge-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "result": { "pong": true }
                    }
                })
            )
            .await
        );
        assert!(call
            .await
            .expect("task joins")
            .expect("call succeeds")
            .is_some());
    }

    #[tokio::test]
    async fn preferred_browser_target_missing_saved_default_is_ambiguous_with_multiple_bridges() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;

        let err = state
            .try_call_native_bridge_raw(
                "ping",
                json!({
                    "browser_target": {
                        "preferred": { "browser": "chromium" },
                        "fallback": "single_connected"
                    }
                }),
                None,
                Some(1_000),
                None,
            )
            .await
            .expect_err("missing saved default with multiple bridges is ambiguous");
        assert!(err.to_string().contains("AMBIGUOUS_BROWSER_TARGET"));
        assert!(timeout(Duration::from_millis(50), chrome_rx.recv())
            .await
            .is_err());
        assert!(timeout(Duration::from_millis(50), edge_rx.recv())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn browser_kind_target_remains_ambiguous_for_duplicate_kind() {
        let state = SupervisorState::new(test_config());
        let (edge_tx, _edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;
        let (edge_two_tx, _edge_two_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge-two",
            edge_two_tx,
            "edge-instance-two",
            "edge",
        )
        .await;

        let err = state
            .try_call_native_bridge_raw(
                "ping",
                json!({ "browser": "edge" }),
                None,
                Some(1_000),
                None,
            )
            .await
            .expect_err("duplicate browser kind remains ambiguous");
        assert!(err.to_string().contains("AMBIGUOUS_BROWSER_TARGET"));
    }

    #[tokio::test]
    async fn bridge_status_api_returns_stable_json_without_secrets() {
        let state = SupervisorState::new(test_config());
        let (chrome_tx, _chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;
        state
            .record_native_bridge_ping_success(
                "chrome-bridge",
                &json!({
                    "result": {
                        "extension_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "browser_instance_id": "chrome-instance",
                        "extension_target": "chrome",
                        "extension_target_hint": "chromium-mv3"
                    }
                }),
                7,
            )
            .await;
        let session = BrowserSessionRecord {
            session_id: "session-1".to_string(),
            bridge_id: "chrome-bridge".to_string(),
            bridge_epoch: 1,
            browser_instance_id: Some("chrome-instance".to_string()),
            browser: Some("chrome".to_string()),
            caller_origin: Some("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/".to_string()),
            caller_extension_id: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            created_at_ms: 1,
            last_activity_at_ms: 2,
            disconnected_at_ms: None,
            reconnected_at_ms: None,
            reconnect_count: 0,
            suspicious_identity_at_ms: None,
        };
        state
            .sessions
            .lock()
            .await
            .insert("session-1".to_string(), session);

        let response = state
            .dispatch("runtime.bridges", json!({}))
            .await
            .expect("runtime.bridges succeeds");
        assert_eq!(
            response.get("version").and_then(Value::as_str),
            Some("rzn.runtime.bridges.v1")
        );
        assert_eq!(
            response
                .pointer("/capabilities/bridge_status_api")
                .and_then(Value::as_bool),
            Some(true)
        );
        let bridge = response
            .get("bridges")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .expect("bridge record");
        for key in [
            "bridge_id",
            "browser_instance_id",
            "caller_origin",
            "extension_id",
            "extension_target",
            "connected_since_ms",
            "last_successful_ping_at_ms",
            "stale_bridge_response_drop_count",
            "native_host_restart_count",
            "timeout_count",
            "active_request_count",
        ] {
            assert!(bridge.get(key).is_some(), "missing bridge key {key}");
        }
        assert_eq!(
            bridge.get("active_session_count").and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            bridge
                .pointer("/active_sessions/0/session_id")
                .and_then(Value::as_str),
            Some("session-1")
        );
        let serialized = serde_json::to_string(&response).expect("bridge status serializes");
        assert!(!serialized.contains("token_path"));
        assert!(!serialized.contains("RZN_SUPERVISOR_TOKEN"));
        assert!(!serialized.contains("socket_path"));
    }

    #[tokio::test]
    async fn bridge_lifecycle_metrics_increment_and_surface_in_bridge_status() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let chrome_epoch = register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;

        let first = state.browser_targets().await;
        let bridge = first
            .get("bridges")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .expect("bridge record");
        assert_eq!(
            bridge
                .get("bridge_registration_count")
                .and_then(Value::as_u64),
            Some(1)
        );

        assert!(
            !state
                .complete_native_bridge_response(
                    "chrome-bridge",
                    chrome_epoch,
                    &json!({ "id": "stale-response" }),
                )
                .await
        );
        let after_stale = state.browser_targets().await;
        assert_eq!(
            after_stale
                .pointer("/bridges/0/stale_bridge_response_drop_count")
                .and_then(Value::as_u64),
            Some(1)
        );

        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;
        let default_call = tokio::spawn({
            let state = state.clone();
            async move {
                state
                    .try_dispatch_supervisor_browser_tool(
                        "browser.execute_step",
                        json!({
                            "browser": "chrome",
                            "step": { "type": "click" }
                        }),
                    )
                    .await
            }
        });
        let request: Value =
            serde_json::from_slice(&chrome_rx.recv().await.expect("chrome bridge receives call"))
                .expect("extension call json");
        assert!(timeout(Duration::from_millis(50), edge_rx.recv())
            .await
            .is_err());
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": { "success": true, "current_tab_id": 123 }
                })
            )
            .await
        );
        let default_response = default_call
            .await
            .expect("default task joins")
            .expect("default dispatch succeeds")
            .expect("default response");
        assert_eq!(
            default_response
                .pointer("/resolved_browser_target/source")
                .and_then(Value::as_str),
            Some("browser")
        );
        let status = state.browser_targets().await;
        let total_resolutions: u64 = status
            .get("bridges")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|bridge| {
                bridge
                    .get("target_resolution_count")
                    .and_then(Value::as_u64)
            })
            .sum();
        assert!(total_resolutions > 0);
    }

    #[tokio::test]
    async fn one_browser_compat_old_payloads_route_without_target_flags() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;

        let session_open = state
            .try_dispatch_supervisor_browser_tool("browser.session_open", json!({}))
            .await
            .expect("session open succeeds")
            .expect("session response");
        let session_id = session_open
            .get("session_id")
            .and_then(Value::as_str)
            .expect("session id")
            .to_string();

        let snapshot_state = state.clone();
        let snapshot_call = tokio::spawn(async move {
            snapshot_state
                .try_dispatch_supervisor_browser_tool("browser.snapshot", json!({}))
                .await
        });
        let snapshot_request: Value =
            serde_json::from_slice(&chrome_rx.recv().await.expect("snapshot request"))
                .expect("snapshot request json");
        assert_eq!(
            snapshot_request
                .pointer("/params/resolved_browser_target/source")
                .and_then(Value::as_str),
            Some("single_bridge_default")
        );
        let snapshot_request_id = snapshot_request
            .get("id")
            .and_then(value_id_string)
            .expect("snapshot request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": snapshot_request_id,
                    "result": {
                        "success": true,
                        "result": { "dom_hash": "abc" },
                        "current_tab_id": 42
                    }
                })
            )
            .await
        );
        let snapshot_response = snapshot_call
            .await
            .expect("snapshot task joins")
            .expect("snapshot dispatch succeeds")
            .expect("snapshot response");
        assert_eq!(
            snapshot_response.pointer("/result/current_tab_id"),
            Some(&json!(42))
        );

        let execute_state = state.clone();
        let execute_call = tokio::spawn(async move {
            execute_state
                .try_dispatch_supervisor_browser_tool(
                    "browser.execute_step",
                    json!({ "step": { "type": "click" } }),
                )
                .await
        });
        let execute_request: Value =
            serde_json::from_slice(&chrome_rx.recv().await.expect("execute request"))
                .expect("execute request json");
        assert_eq!(
            execute_request
                .pointer("/params/resolved_browser_target/source")
                .and_then(Value::as_str),
            Some("single_bridge_default")
        );
        let execute_request_id = execute_request
            .get("id")
            .and_then(value_id_string)
            .expect("execute request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": execute_request_id,
                    "result": {
                        "success": true,
                        "current_tab_id": 43
                    }
                })
            )
            .await
        );
        let execute_response = execute_call
            .await
            .expect("execute task joins")
            .expect("execute dispatch succeeds")
            .expect("execute response");
        assert_eq!(execute_response.get("current_tab_id"), Some(&json!(43)));

        let close_state = state.clone();
        let close_session_id = session_id.clone();
        let close_call = tokio::spawn(async move {
            close_state
                .try_dispatch_supervisor_browser_tool(
                    "browser.session_close",
                    json!({ "session_id": close_session_id }),
                )
                .await
        });
        let close_request: Value =
            serde_json::from_slice(&chrome_rx.recv().await.expect("close request"))
                .expect("close request json");
        assert_eq!(
            close_request
                .pointer("/params/resolved_browser_target/source")
                .and_then(Value::as_str),
            Some("session_id")
        );
        let close_request_id = close_request
            .get("id")
            .and_then(value_id_string)
            .expect("close request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": close_request_id,
                    "result": {
                        "success": true,
                        "result": { "tab_closed": true, "tab_id": 43 }
                    }
                })
            )
            .await
        );
        let close_response = close_call
            .await
            .expect("close task joins")
            .expect("close dispatch succeeds")
            .expect("close response");
        assert_eq!(
            close_response.get("tab_closed").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[tokio::test]
    async fn one_browser_compat_zero_errors_and_multi_bridge_defaults_are_ambiguous() {
        let empty = SupervisorState::new(test_config());
        let no_bridge = empty
            .try_dispatch_supervisor_browser_tool(
                "browser.execute_step",
                json!({ "step": { "type": "click" } }),
            )
            .await
            .expect("no bridge returns structured error")
            .expect("no bridge response");
        assert_eq!(
            no_bridge.get("error_code").and_then(Value::as_str),
            Some("NO_BROWSER_BRIDGE_CONNECTED")
        );

        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;
        let response = state
            .try_dispatch_supervisor_browser_tool(
                "browser.execute_step",
                json!({ "step": { "type": "click" } }),
            )
            .await
            .expect("multi-bridge default returns structured error")
            .expect("multi-bridge error response");
        assert_eq!(
            response.get("error_code").and_then(Value::as_str),
            Some("AMBIGUOUS_BROWSER_TARGET")
        );
        assert!(timeout(Duration::from_millis(50), chrome_rx.recv())
            .await
            .is_err());
        assert!(timeout(Duration::from_millis(50), edge_rx.recv())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn multi_bridge_routing_registers_two_and_explicit_bridge_is_isolated() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;

        {
            let bridges = state.native_bridges.lock().await;
            assert_eq!(bridges.len(), 2);
            assert!(bridges.contains_key("chrome-bridge"));
            assert!(bridges.contains_key("edge-bridge"));
        }

        let targets = state.browser_targets().await;
        assert_eq!(targets.get("target_count").and_then(Value::as_u64), Some(2));
        assert_eq!(targets.get("bridge_count").and_then(Value::as_u64), Some(2));

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw_with_target(
                    "ping",
                    json!({}),
                    None,
                    Some(1_000),
                    None,
                    BridgeTarget::BridgeId("chrome-bridge".to_string()),
                )
                .await
        });

        let request: Value = serde_json::from_slice(
            &chrome_rx
                .recv()
                .await
                .expect("explicit bridge target routes to chrome bridge"),
        )
        .expect("chrome request json");
        assert!(timeout(Duration::from_millis(50), edge_rx.recv())
            .await
            .is_err());
        assert_eq!(
            request
                .pointer("/params/resolved_browser_target/bridge_id")
                .and_then(Value::as_str),
            Some("chrome-bridge")
        );
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "result": { "pong": true }
                    }
                })
            )
            .await
        );
        assert_eq!(
            call.await
                .expect("task joins")
                .expect("call succeeds")
                .expect("response")
                .pointer("/result/pong"),
            Some(&json!(true))
        );

        let bridges = state.native_bridges.lock().await;
        assert!(bridges.contains_key("chrome-bridge"));
        assert!(bridges.contains_key("edge-bridge"));
    }

    #[tokio::test]
    async fn browser_target_session_binding_routes_without_global_default() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;

        let opened = state
            .dispatch("browser.session_open", json!({ "browser": "chrome" }))
            .await
            .expect("session open succeeds");
        let session_id = opened
            .get("session_id")
            .and_then(Value::as_str)
            .expect("session id");

        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;

        let state_for_call = state.clone();
        let session_id_for_call = session_id.to_string();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw(
                    "ping",
                    json!({ "session_id": session_id_for_call }),
                    None,
                    Some(1_000),
                    None,
                )
                .await
        });

        let request: Value = serde_json::from_slice(
            &chrome_rx
                .recv()
                .await
                .expect("session binding routes to chrome bridge"),
        )
        .expect("extension call json");
        assert!(timeout(Duration::from_millis(50), edge_rx.recv())
            .await
            .is_err());
        assert_eq!(
            request
                .pointer("/params/resolved_browser_target/source")
                .and_then(Value::as_str),
            Some("session_id")
        );
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("extension call id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "result": { "pong": true }
                    }
                })
            )
            .await
        );
        assert!(call
            .await
            .expect("task joins")
            .expect("call succeeds")
            .is_some());
    }

    #[tokio::test]
    async fn browser_session_target_records_route_conflict_and_close() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;

        let chrome_opened = state
            .dispatch("browser.session_open", json!({ "browser": "chrome" }))
            .await
            .expect("chrome session opens");
        assert_eq!(
            chrome_opened
                .pointer("/resolved_browser_target/browser")
                .and_then(Value::as_str),
            Some("chrome")
        );
        let chrome_session_id = chrome_opened
            .get("session_id")
            .and_then(Value::as_str)
            .expect("chrome session id")
            .to_string();
        let edge_opened = state
            .dispatch("browser.session_open", json!({ "browser": "edge" }))
            .await
            .expect("edge session opens");
        assert_eq!(
            edge_opened
                .pointer("/resolved_browser_target/browser")
                .and_then(Value::as_str),
            Some("edge")
        );
        let edge_session_id = edge_opened
            .get("session_id")
            .and_then(Value::as_str)
            .expect("edge session id")
            .to_string();

        {
            let sessions = state.sessions.lock().await;
            let chrome_session = sessions
                .get(&chrome_session_id)
                .expect("chrome session record");
            assert_eq!(chrome_session.session_id, chrome_session_id);
            assert_eq!(chrome_session.bridge_id, "chrome-bridge");
            assert_eq!(
                chrome_session.browser_instance_id.as_deref(),
                Some("chrome-instance")
            );
            assert_eq!(chrome_session.browser.as_deref(), Some("chrome"));
            assert!(chrome_session.created_at_ms > 0);
            assert!(chrome_session.last_activity_at_ms >= chrome_session.created_at_ms);
            let edge_session = sessions.get(&edge_session_id).expect("edge session record");
            assert_eq!(edge_session.bridge_id, "edge-bridge");
            assert_eq!(
                edge_session.browser_instance_id.as_deref(),
                Some("edge-instance")
            );
        }

        let state_for_chrome_step = state.clone();
        let chrome_session_for_step = chrome_session_id.clone();
        let chrome_step = tokio::spawn(async move {
            state_for_chrome_step
                .dispatch(
                    "browser.execute_step",
                    json!({
                        "session_id": chrome_session_for_step,
                        "step": { "type": "get_current_url" }
                    }),
                )
                .await
        });
        let chrome_request: Value = serde_json::from_slice(
            &chrome_rx
                .recv()
                .await
                .expect("chrome session routes to chrome bridge"),
        )
        .expect("chrome request json");
        assert!(timeout(Duration::from_millis(50), edge_rx.recv())
            .await
            .is_err());
        assert_eq!(
            chrome_request
                .pointer("/params/resolved_browser_target/browser_instance_id")
                .and_then(Value::as_str),
            Some("chrome-instance")
        );
        let chrome_request_id = chrome_request
            .get("id")
            .and_then(value_id_string)
            .expect("chrome request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": chrome_request_id,
                    "result": {
                        "success": true,
                        "result": { "url": "https://chrome.test/" }
                    }
                })
            )
            .await
        );
        assert_eq!(
            chrome_step
                .await
                .expect("chrome task joins")
                .expect("chrome step succeeds")
                .pointer("/result/url")
                .and_then(Value::as_str),
            Some("https://chrome.test/")
        );

        let conflict = state
            .dispatch(
                "browser.execute_step",
                json!({
                    "session_id": chrome_session_id,
                    "browser": "edge",
                    "step": { "type": "get_current_url" }
                }),
            )
            .await
            .expect("conflicting explicit target returns structured error");
        assert_eq!(
            conflict.get("error_code").and_then(Value::as_str),
            Some("SESSION_TARGET_CONFLICT")
        );

        let state_for_edge_step = state.clone();
        let edge_session_for_step = edge_session_id.clone();
        let edge_step = tokio::spawn(async move {
            state_for_edge_step
                .dispatch(
                    "browser.execute_step",
                    json!({
                        "session_id": edge_session_for_step,
                        "step": { "type": "get_current_url" }
                    }),
                )
                .await
        });
        let edge_request: Value = serde_json::from_slice(
            &edge_rx
                .recv()
                .await
                .expect("edge session routes to edge bridge"),
        )
        .expect("edge request json");
        assert_eq!(
            edge_request
                .pointer("/params/resolved_browser_target/browser_instance_id")
                .and_then(Value::as_str),
            Some("edge-instance")
        );
        let edge_request_id = edge_request
            .get("id")
            .and_then(value_id_string)
            .expect("edge request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "edge-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": edge_request_id,
                    "result": {
                        "success": true,
                        "result": { "url": "https://edge.test/" }
                    }
                })
            )
            .await
        );
        assert_eq!(
            edge_step
                .await
                .expect("edge task joins")
                .expect("edge step succeeds")
                .pointer("/result/url")
                .and_then(Value::as_str),
            Some("https://edge.test/")
        );

        let state_for_close = state.clone();
        let chrome_session_for_close = chrome_session_id.clone();
        let close = tokio::spawn(async move {
            state_for_close
                .dispatch(
                    "browser.session_close",
                    json!({ "session_id": chrome_session_for_close }),
                )
                .await
        });
        let close_request: Value = serde_json::from_slice(
            &chrome_rx
                .recv()
                .await
                .expect("close routes through chrome binding"),
        )
        .expect("close request json");
        let close_request_id = close_request
            .get("id")
            .and_then(value_id_string)
            .expect("close request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": close_request_id,
                    "result": {
                        "success": true,
                        "tab_closed": true
                    }
                })
            )
            .await
        );
        assert_eq!(
            close
                .await
                .expect("close task joins")
                .expect("close succeeds")
                .get("tab_closed")
                .and_then(Value::as_bool),
            Some(true)
        );
        let sessions = state.sessions.lock().await;
        assert!(!sessions.contains_key(&chrome_session_id));
        assert!(sessions.contains_key(&edge_session_id));
    }

    #[tokio::test]
    async fn browser_session_reconnect_allows_same_instance_and_rejects_identity_conflict() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let chrome_origin = "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/";
        let conflicting_origin = "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/";
        let (old_chrome_tx, _old_chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_identity_for_test(
            &state,
            "chrome-old",
            old_chrome_tx,
            "chrome-instance",
            "chrome",
            chrome_origin,
        )
        .await;

        let opened = state
            .dispatch("browser.session_open", json!({ "browser": "chrome" }))
            .await
            .expect("chrome session opens");
        let session_id = opened
            .get("session_id")
            .and_then(Value::as_str)
            .expect("session id")
            .to_string();

        state.clear_native_bridge("chrome-old").await;
        {
            let sessions = state.sessions.lock().await;
            let session = sessions.get(&session_id).expect("session remains bound");
            assert_eq!(session.bridge_id, "chrome-old");
            assert_eq!(
                session.browser_instance_id.as_deref(),
                Some("chrome-instance")
            );
            assert!(session.disconnected_at_ms.is_some());
        }

        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_identity_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
            "chrome-extension://cccccccccccccccccccccccccccccccc/",
        )
        .await;
        let disconnected = state
            .dispatch(
                "browser.execute_step",
                json!({
                    "session_id": session_id,
                    "step": { "type": "get_current_url" }
                }),
            )
            .await
            .expect("disconnected session returns structured error");
        assert_eq!(
            disconnected.get("error_code").and_then(Value::as_str),
            Some("BROWSER_INSTANCE_NOT_CONNECTED")
        );
        assert!(timeout(Duration::from_millis(50), edge_rx.recv())
            .await
            .is_err());

        let (other_chrome_tx, mut other_chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_identity_for_test(
            &state,
            "chrome-other-instance",
            other_chrome_tx,
            "chrome-other-instance",
            "chrome",
            chrome_origin,
        )
        .await;
        let wrong_instance = state
            .dispatch(
                "browser.execute_step",
                json!({
                    "session_id": session_id,
                    "step": { "type": "get_current_url" }
                }),
            )
            .await
            .expect("different browser instance does not resume session");
        assert_eq!(
            wrong_instance.get("error_code").and_then(Value::as_str),
            Some("BROWSER_INSTANCE_NOT_CONNECTED")
        );
        assert!(timeout(Duration::from_millis(50), other_chrome_rx.recv())
            .await
            .is_err());

        let (new_chrome_tx, mut new_chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_identity_for_test(
            &state,
            "chrome-new",
            new_chrome_tx,
            "chrome-instance",
            "chrome",
            chrome_origin,
        )
        .await;
        let state_for_step = state.clone();
        let session_for_step = session_id.clone();
        let step = tokio::spawn(async move {
            state_for_step
                .dispatch(
                    "browser.execute_step",
                    json!({
                        "session_id": session_for_step,
                        "step": { "type": "get_current_url" }
                    }),
                )
                .await
        });
        let request: Value = serde_json::from_slice(
            &new_chrome_rx
                .recv()
                .await
                .expect("same-instance reconnect routes to new chrome bridge"),
        )
        .expect("new chrome request json");
        assert_eq!(
            request
                .pointer("/params/resolved_browser_target/bridge_id")
                .and_then(Value::as_str),
            Some("chrome-new")
        );
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("request id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-new",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "result": { "url": "https://chrome-reconnected.test/" }
                    }
                })
            )
            .await
        );
        assert_eq!(
            step.await
                .expect("step task joins")
                .expect("step succeeds")
                .pointer("/result/url")
                .and_then(Value::as_str),
            Some("https://chrome-reconnected.test/")
        );
        {
            let sessions = state.sessions.lock().await;
            let session = sessions.get(&session_id).expect("session remains");
            assert_eq!(session.bridge_id, "chrome-new");
            assert!(session.disconnected_at_ms.is_none());
            assert!(session.reconnected_at_ms.is_some());
            assert_eq!(session.reconnect_count, 1);
        }

        state.clear_native_bridge("chrome-new").await;
        let (suspicious_tx, _suspicious_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_identity_for_test(
            &state,
            "chrome-suspicious",
            suspicious_tx,
            "chrome-instance",
            "chrome",
            conflicting_origin,
        )
        .await;
        let suspicious = state
            .dispatch(
                "browser.execute_step",
                json!({
                    "session_id": session_id,
                    "step": { "type": "get_current_url" }
                }),
            )
            .await
            .expect("suspicious identity returns structured error");
        assert_eq!(
            suspicious.get("error_code").and_then(Value::as_str),
            Some("SESSION_TARGET_CONFLICT")
        );
        let sessions = state.sessions.lock().await;
        let session = sessions
            .get(&session_id)
            .expect("session remains after conflict");
        assert!(session.suspicious_identity_at_ms.is_some());
        assert_eq!(session.bridge_id, "chrome-new");
    }

    #[tokio::test]
    async fn bridge_target_send_failure_clears_only_selected_bridge() {
        let state = SupervisorState::new(test_config());
        let (closed_tx, closed_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        drop(closed_rx);
        state
            .register_native_bridge("closed-bridge".to_string(), closed_tx)
            .await;
        let (live_tx, mut live_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("live-bridge".to_string(), live_tx)
            .await;

        let result = state
            .try_call_native_bridge_raw_inner(
                "ping",
                json!({}),
                None,
                Some(1_000),
                None,
                false,
                BridgeTarget::BridgeId("closed-bridge".to_string()),
            )
            .await
            .expect("closed selected bridge returns no response");
        assert!(result.is_none());
        let bridges = state.native_bridges.lock().await;
        assert!(!bridges.contains_key("closed-bridge"));
        assert!(bridges.contains_key("live-bridge"));
        drop(bridges);

        let state_for_call = Arc::new(state);
        let state_for_ping = state_for_call.clone();
        let call = tokio::spawn(async move {
            state_for_ping
                .try_call_native_bridge_raw_with_target(
                    "ping",
                    json!({}),
                    None,
                    Some(1_000),
                    None,
                    BridgeTarget::BridgeId("live-bridge".to_string()),
                )
                .await
        });
        let request: Value = serde_json::from_slice(
            &live_rx
                .recv()
                .await
                .expect("live bridge remains usable after selected send failure"),
        )
        .expect("extension call json");
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("extension call id");
        assert!(
            complete_bridge_response_for_test(
                &state_for_call,
                "live-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "result": { "pong": true }
                    }
                })
            )
            .await
        );
        assert_eq!(
            call.await
                .expect("task joins")
                .expect("call succeeds")
                .expect("response")
                .pointer("/result/pong"),
            Some(&json!(true))
        );
    }

    #[tokio::test]
    async fn native_bridge_command_jsonrpc_error_does_not_restart_bridge() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("chrome-bridge".to_string(), tx)
            .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw_with_target(
                    "browser.execute_step",
                    json!({ "step": { "type": "click" } }),
                    None,
                    Some(1_000),
                    None,
                    BridgeTarget::BridgeId("chrome-bridge".to_string()),
                )
                .await
        });

        let request: Value = serde_json::from_slice(
            &timeout(Duration::from_millis(250), rx.recv())
                .await
                .expect("bridge receives request")
                .expect("request frame"),
        )
        .expect("extension call json");
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("extension call id");
        assert!(
            complete_bridge_response_for_test(
                &state,
                "chrome-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "error": {
                        "code": -32602,
                        "message": "selector not found",
                        "data": { "error_code": "SELECTOR_NOT_FOUND" }
                    }
                })
            )
            .await
        );

        let error = call
            .await
            .expect("task joins")
            .expect_err("command error propagates");
        assert!(error.to_string().contains("selector not found"));
        assert!(timeout(Duration::from_millis(50), rx.recv()).await.is_err());
        assert!(state
            .native_bridges
            .lock()
            .await
            .contains_key("chrome-bridge"));
        let health_by_bridge = state.native_bridge_health.lock().await;
        let health = health_by_bridge
            .get("chrome-bridge")
            .expect("bridge health");
        assert_eq!(health.native_host_restart_count, 0);
        assert_eq!(health.last_restart_reason.as_deref(), None);
    }

    #[tokio::test]
    async fn native_bridge_pending_deadline_starts_after_reconnect_retry() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (closed_tx, closed_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        drop(closed_rx);
        state
            .register_native_bridge("closed-bridge".to_string(), closed_tx)
            .await;

        let state_for_call = state.clone();
        let started_at_ms = now_ms();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw_with_target(
                    "ping",
                    json!({}),
                    None,
                    Some(1_000),
                    None,
                    BridgeTarget::Default,
                )
                .await
        });

        tokio::time::sleep(Duration::from_millis(75)).await;
        let (fresh_tx, mut fresh_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let reconnected_at_ms = now_ms();
        state
            .register_native_bridge("fresh-bridge".to_string(), fresh_tx)
            .await;

        let request: Value = serde_json::from_slice(
            &timeout(Duration::from_millis(1_000), fresh_rx.recv())
                .await
                .expect("fresh bridge receives retry")
                .expect("retry frame"),
        )
        .expect("extension call json");
        let request_id = request
            .get("id")
            .and_then(value_id_string)
            .expect("extension call id");
        let deadline_at_ms = state
            .native_bridge_pending
            .lock()
            .await
            .get(&request_id)
            .expect("pending call remains while waiting")
            .deadline_at_ms;
        assert!(
            deadline_at_ms >= reconnected_at_ms.saturating_add(900),
            "deadline {deadline_at_ms} should be based on reconnect time {reconnected_at_ms}"
        );
        assert!(
            deadline_at_ms > started_at_ms.saturating_add(1_000),
            "deadline {deadline_at_ms} should not be based on initial call time {started_at_ms}"
        );

        assert!(
            complete_bridge_response_for_test(
                &state,
                "fresh-bridge",
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": {
                        "success": true,
                        "result": { "pong": true }
                    }
                })
            )
            .await
        );
        assert_eq!(
            call.await
                .expect("task joins")
                .expect("call succeeds")
                .expect("response")
                .pointer("/result/pong"),
            Some(&json!(true))
        );
    }

    #[tokio::test]
    async fn native_bridge_restart_scopes_shutdown_pending_drain_and_health_to_epoch() {
        let state = SupervisorState::new(test_config());
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let chrome_epoch = state
            .register_native_bridge("chrome-bridge".to_string(), chrome_tx)
            .await;
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let edge_epoch = state
            .register_native_bridge("edge-bridge".to_string(), edge_tx)
            .await;

        let (chrome_pending_tx, _chrome_pending_rx) = oneshot::channel();
        let (edge_pending_tx, edge_pending_rx) = oneshot::channel();
        {
            let mut pending = state.native_bridge_pending.lock().await;
            pending.insert(
                "chrome-call".to_string(),
                PendingNativeCall {
                    bridge_id: "chrome-bridge".to_string(),
                    bridge_epoch: chrome_epoch,
                    method: "ping".to_string(),
                    deadline_at_ms: now_ms() + 1_000,
                    responder: chrome_pending_tx,
                },
            );
            pending.insert(
                "edge-call".to_string(),
                PendingNativeCall {
                    bridge_id: "edge-bridge".to_string(),
                    bridge_epoch: edge_epoch,
                    method: "ping".to_string(),
                    deadline_at_ms: now_ms() + 1_000,
                    responder: edge_pending_tx,
                },
            );
        }

        state
            .request_native_bridge_restart(
                "chrome-bridge",
                chrome_epoch.saturating_add(10),
                "stale restart request",
            )
            .await;

        assert!(timeout(Duration::from_millis(50), chrome_rx.recv())
            .await
            .is_err());
        assert!(state
            .native_bridges
            .lock()
            .await
            .contains_key("chrome-bridge"));
        assert!(state
            .native_bridge_pending
            .lock()
            .await
            .contains_key("chrome-call"));

        state
            .record_native_bridge_failure(
                Some("edge-bridge"),
                READINESS_CAUSE_ZOMBIE_NATIVE_HOST,
                "edge timed out",
                true,
            )
            .await;
        state
            .request_native_bridge_restart("edge-bridge", edge_epoch, "edge timed out")
            .await;

        let shutdown = timeout(Duration::from_millis(250), edge_rx.recv())
            .await
            .expect("edge shutdown frame should be sent")
            .expect("edge shutdown frame");
        let shutdown: Value = serde_json::from_slice(&shutdown).expect("shutdown json");
        assert_eq!(
            shutdown.get("method").and_then(Value::as_str),
            Some(NATIVE_HOST_SHUTDOWN_METHOD)
        );
        assert_eq!(
            shutdown
                .pointer("/params/bridge_id")
                .and_then(Value::as_str),
            Some("edge-bridge")
        );
        assert!(timeout(Duration::from_millis(50), chrome_rx.recv())
            .await
            .is_err());
        assert!(edge_pending_rx.await.is_ok());

        let bridges = state.native_bridges.lock().await;
        assert!(bridges.contains_key("chrome-bridge"));
        assert!(!bridges.contains_key("edge-bridge"));
        drop(bridges);
        let pending = state.native_bridge_pending.lock().await;
        assert!(pending.contains_key("chrome-call"));
        assert!(!pending.contains_key("edge-call"));
        drop(pending);

        let health_by_bridge = state.native_bridge_health.lock().await;
        let edge_health = health_by_bridge
            .get("edge-bridge")
            .expect("edge bridge health remains for diagnostics");
        assert_eq!(
            edge_health.current_bridge_id.as_deref(),
            Some("edge-bridge")
        );
        assert_eq!(edge_health.native_host_restart_count, 1);
        assert_eq!(edge_health.timeout_count, 1);
        assert_eq!(
            edge_health.last_failure_cause.as_deref(),
            Some(READINESS_CAUSE_ZOMBIE_NATIVE_HOST)
        );
        assert_eq!(
            edge_health.last_restart_reason.as_deref(),
            Some("edge timed out")
        );
        let chrome_health = health_by_bridge
            .get("chrome-bridge")
            .expect("chrome bridge health");
        assert_eq!(chrome_health.native_host_restart_count, 0);
        assert_eq!(chrome_health.timeout_count, 0);
    }

    #[tokio::test]
    async fn native_bridge_response_completion_is_bridge_epoch_fenced() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let bridge_epoch = state
            .register_native_bridge("epoch-bridge".to_string(), tx)
            .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_call_native_bridge_raw("ping", json!({}), None, Some(1_000), None)
                .await
        });

        let first = rx.recv().await.expect("extension call frame");
        let first: Value = serde_json::from_slice(&first).expect("extension call json");
        let request_id = first
            .get("id")
            .and_then(value_id_string)
            .expect("extension call id");
        let response = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "success": true,
                "result": { "pong": true }
            }
        });

        assert!(
            !state
                .complete_native_bridge_response("wrong-bridge", bridge_epoch, &response)
                .await
        );
        assert!(
            !state
                .complete_native_bridge_response(
                    "epoch-bridge",
                    bridge_epoch.saturating_add(1),
                    &response
                )
                .await
        );
        assert_eq!(state.native_bridge_pending.lock().await.len(), 1);

        assert!(
            state
                .complete_native_bridge_response("epoch-bridge", bridge_epoch, &response)
                .await
        );
        let value = call
            .await
            .expect("call joins")
            .expect("call succeeds")
            .expect("call returns response");
        assert_eq!(
            value.pointer("/result/pong").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            state
                .native_bridge_health
                .lock()
                .await
                .get("epoch-bridge")
                .expect("epoch bridge health")
                .bridge_response_mismatch_drop_count,
            2
        );
    }

    #[tokio::test]
    async fn execute_step_channel_close_returns_transient_step_failure() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (old_tx, mut old_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("old-bridge".to_string(), old_tx)
            .await;

        let state_for_call = state.clone();
        let call = tokio::spawn(async move {
            state_for_call
                .try_dispatch_supervisor_browser_tool(
                    "browser.execute_step",
                    json!({ "step": { "type": "get_current_url" } }),
                )
                .await
        });

        let first = old_rx.recv().await.expect("extension call frame");
        let first: Value = serde_json::from_slice(&first).expect("extension call json");
        assert_eq!(
            first.get("method").and_then(Value::as_str),
            Some("native_host.extension_call")
        );

        let (fresh_tx, _fresh_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("fresh-bridge".to_string(), fresh_tx)
            .await;
        state.clear_native_bridge("old-bridge").await;

        let value = timeout(Duration::from_millis(250), call)
            .await
            .expect("drained call should finish promptly")
            .expect("task joins")
            .expect("dispatch should convert transient transport errors")
            .expect("execute_step returns a failure response");

        assert_eq!(value.get("success").and_then(Value::as_bool), Some(false));
        assert_eq!(
            value.get("error_code").and_then(Value::as_str),
            Some("NATIVE_HOST_DISCONNECTED")
        );
        assert_eq!(
            value.pointer("/run_result/status").and_then(Value::as_str),
            Some("failed")
        );
    }

    #[tokio::test]
    async fn native_bridge_ping_success_preserves_last_failure_beacon() {
        let state = SupervisorState::new(test_config());
        let (tx, _rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("health-bridge".to_string(), tx)
            .await;
        state
            .record_native_bridge_failure(
                Some("health-bridge"),
                READINESS_CAUSE_ZOMBIE_NATIVE_HOST,
                "extension call timed out",
                true,
            )
            .await;

        state
            .record_native_bridge_ping_success(
                "health-bridge",
                &json!({
                    "result": {
                        "extension_build_signature": "fresh-build",
                        "bridge_contract_version": EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION,
                        "supervisor_bridge_epoch": 4,
                        "native_host_pid": 12345,
                        "native_host_boot_id": "native-host-fresh",
                        "extension_worker_boot_id": "extension-worker-fresh",
                        "native_port_epoch": 7,
                        "last_native_host_stdout_heartbeat_ms": 100,
                        "last_native_host_stdout_heartbeat_age_ms": 9,
                        "last_native_host_stdout_heartbeat_seq": 12,
                        "last_native_roundtrip_ping_ms": 101,
                        "last_native_roundtrip_ping_age_ms": 3,
                        "missed_native_roundtrip_pings": 0
                    }
                }),
                17,
            )
            .await;

        let health_by_bridge = state.native_bridge_health.lock().await;
        let health = health_by_bridge
            .get("health-bridge")
            .expect("health bridge record");
        assert!(health.last_successful_ping_at_ms.is_some());
        assert_eq!(
            health.last_successful_extension_build_signature,
            Some(json!("fresh-build"))
        );
        assert_eq!(
            health.last_successful_bridge_contract_version,
            Some(EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION)
        );
        assert_eq!(health.last_successful_native_port_epoch, Some(7));
        assert_eq!(health.last_successful_ping_latency_ms, Some(17));
        assert_eq!(health.last_successful_supervisor_bridge_epoch, Some(4));
        assert_eq!(health.last_successful_native_host_pid, Some(12345));
        assert_eq!(
            health.last_successful_native_host_boot_id.as_deref(),
            Some("native-host-fresh")
        );
        assert_eq!(
            health.last_successful_extension_worker_boot_id.as_deref(),
            Some("extension-worker-fresh")
        );
        assert_eq!(health.last_successful_stdout_heartbeat_age_ms, Some(9));
        assert_eq!(health.last_successful_stdout_heartbeat_seq, Some(12));
        assert_eq!(health.last_successful_roundtrip_ping_age_ms, Some(3));
        assert_eq!(health.missed_roundtrip_count, Some(0));
        assert_eq!(
            health.last_failure_cause.as_deref(),
            Some(READINESS_CAUSE_ZOMBIE_NATIVE_HOST)
        );
        assert_eq!(
            health.last_failure_error.as_deref(),
            Some("extension call timed out")
        );
        assert!(health.last_step_timeout_at_ms.is_some());
    }

    #[tokio::test]
    async fn readiness_probe_timeout_clears_cached_bridge_and_status() {
        let state = SupervisorState::new(test_config());
        let (stale_tx, _stale_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("stale-bridge".to_string(), stale_tx)
            .await;

        let readiness = state
            .ensure_ready(json!({
                "bridge_wait_ms": 0,
                "bridge_probe_timeout_ms": 10
            }))
            .await
            .expect("readiness returns structured failure");

        assert_eq!(readiness.get("ready").and_then(Value::as_bool), Some(false));
        assert!(readiness
            .pointer("/native_host_bridge/probe/error")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("Native-host extension bridge timeout"));
        assert_eq!(
            readiness.pointer("/diagnostic/cause"),
            Some(&json!(READINESS_CAUSE_ZOMBIE_NATIVE_HOST))
        );
        assert!(readiness
            .pointer("/diagnostic/action_text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("native-host/native-port restart"));
        assert_eq!(
            readiness.pointer("/native_host_bridge/health/last_failure_cause"),
            Some(&json!(READINESS_CAUSE_ZOMBIE_NATIVE_HOST))
        );
        assert!(readiness
            .pointer("/native_host_bridge/health/last_restart_reason")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("extension call 'ping' timed out"));
        assert!(state.native_bridges.lock().await.is_empty());

        let status = state.runtime_status().await;
        assert_eq!(
            status
                .pointer("/native_host_bridge/connected")
                .and_then(Value::as_bool),
            Some(false)
        );

        let (fresh_tx, _fresh_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("fresh-bridge".to_string(), fresh_tx)
            .await;
        let status = state.runtime_status().await;
        assert_eq!(
            status
                .pointer("/native_host_bridge/connected")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[tokio::test]
    async fn readiness_probe_closed_sender_does_not_wait_for_reconnect() {
        let state = SupervisorState::new(test_config());
        let (stale_tx, stale_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        drop(stale_rx);
        state
            .register_native_bridge("closed-sender".to_string(), stale_tx)
            .await;

        let readiness = timeout(
            Duration::from_millis(250),
            state.ensure_ready(json!({
                "bridge_wait_ms": 0,
                "bridge_probe_timeout_ms": 10
            })),
        )
        .await
        .expect("readiness probe should not wait for reconnect")
        .expect("readiness returns structured failure");

        assert_eq!(readiness.get("ready").and_then(Value::as_bool), Some(false));
        assert_eq!(
            readiness
                .pointer("/native_host_bridge/probe/error")
                .and_then(Value::as_str),
            Some("native-host bridge disappeared before ping")
        );
        assert!(state.native_bridges.lock().await.is_empty());
    }

    #[tokio::test]
    async fn readiness_probe_respects_explicit_browser_target() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (chrome_tx, mut chrome_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "chrome-bridge",
            chrome_tx,
            "chrome-instance",
            "chrome",
        )
        .await;
        let (edge_tx, mut edge_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        register_bridge_with_target_for_test(
            &state,
            "edge-bridge",
            edge_tx,
            "edge-instance",
            "edge",
        )
        .await;

        let state_for_readiness = state.clone();
        let readiness = tokio::spawn(async move {
            state_for_readiness
                .ensure_ready(json!({
                    "browser_target": { "browser": "edge" },
                    "bridge_wait_ms": 0,
                    "bridge_probe_timeout_ms": 1_000
                }))
                .await
        });

        let request: Value =
            serde_json::from_slice(&edge_rx.recv().await.expect("edge bridge receives ping"))
                .expect("edge ping request json");
        assert!(timeout(Duration::from_millis(50), chrome_rx.recv())
            .await
            .is_err());
        assert_eq!(
            request
                .pointer("/params/resolved_browser_target/browser")
                .and_then(Value::as_str),
            Some("edge")
        );
        let request_id = request.get("id").cloned().expect("ping request id");
        let mut response = current_bridge_ping_response(request_id, "edge-build");
        response["result"]["result"]["browser_instance_id"] = json!("edge-instance");
        response["result"]["result"]["extension_target"] = json!("edge");
        response["result"]["result"]["extension_target_hint"] = json!("edge-mv3");
        assert!(complete_bridge_response_for_test(&state, "edge-bridge", &response).await);

        let readiness = readiness
            .await
            .expect("readiness task joins")
            .expect("readiness succeeds");
        assert_eq!(readiness.get("ready").and_then(Value::as_bool), Some(true));
        assert_eq!(
            readiness
                .pointer("/native_host_bridge/probe/response/resolved_browser_target/browser")
                .and_then(Value::as_str),
            Some("edge")
        );
    }

    #[tokio::test]
    async fn heal_requires_stable_bridge_after_ready_probe() {
        let state = Arc::new(SupervisorState::new(test_config()));
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        state
            .register_native_bridge("one-shot-bridge".to_string(), tx)
            .await;

        let state_for_response = state.clone();
        tokio::spawn(async move {
            let bytes = rx.recv().await.expect("initial heal ping frame");
            let request: Value = serde_json::from_slice(&bytes).expect("initial heal ping json");
            let id = request.get("id").cloned().expect("request id");
            let response = current_bridge_ping_response(id, "fresh-build");
            assert!(
                complete_bridge_response_for_test(
                    &state_for_response,
                    "one-shot-bridge",
                    &response
                )
                .await
            );
            let _ = rx.recv().await;
        });

        let healed = state
            .runtime_heal(json!({
                "bridge_probe_timeout_ms": 25,
                "bridge_wait_ms": 50
            }))
            .await
            .expect("heal returns structured status");

        assert_eq!(healed.get("ready").and_then(Value::as_bool), Some(false));
        assert_eq!(
            healed.pointer("/stability_readiness/native_host_bridge/cause"),
            Some(&json!(READINESS_CAUSE_ZOMBIE_NATIVE_HOST))
        );
        assert!(healed
            .pointer("/stability_readiness/native_host_bridge/probe/error")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("Native-host extension bridge timeout"));
        assert!(state.native_bridges.lock().await.is_empty());
    }

    #[test]
    fn extension_timeout_uses_step_timeout_with_grace() {
        let params = json!({
            "step": {
                "type": "click",
                "timeout_ms": 12_000
            }
        });
        assert_eq!(
            extension_timeout_ms(&params),
            12_000 + EXTENSION_BRIDGE_GRACE_MS
        );
    }

    #[test]
    fn extension_timeout_respects_explicit_short_probe_timeout() {
        let params = json!({
            "timeout_ms": 1_500,
                    "timeout_grace_ms": BRIDGE_PROBE_TIMEOUT_GRACE_MS
        });
        assert_eq!(extension_timeout_ms(&params), 2_000);
    }

    #[test]
    fn extension_timeout_defaults_when_unspecified() {
        assert_eq!(
            extension_timeout_ms(&json!({})),
            EXTENSION_BRIDGE_TIMEOUT_MS
        );
    }

    #[test]
    fn extension_timeout_clamps_caller_timeout_to_ten_minutes() {
        assert_eq!(
            extension_timeout_ms(&json!({
                "timeout_ms": u64::MAX
            })),
            MAX_CALLER_TIMEOUT_MS
        );
    }

    #[test]
    fn browser_tool_request_timeout_clamps_caller_timeout_to_ten_minutes() {
        assert_eq!(
            supervisor_request_timeout_ms(
                "browser.execute_step",
                &json!({
                    "timeout_ms": u64::MAX
                })
            ),
            MAX_CALLER_TIMEOUT_MS + REQUEST_TIMEOUT_GRACE_MS
        );
    }

    #[test]
    fn bridge_probe_requires_current_contract_and_capabilities() {
        let stale_ping = json!({
            "success": true,
            "result": { "pong": true }
        });
        assert!(bridge_probe_transport_ok(&stale_ping));
        let stale_probe = json!({
            "required_capabilities": bridge_probe_required_capabilities_map(&stale_ping),
            "bridge_contract_version_ok": bridge_probe_contract_version_ok(&stale_ping),
        });
        assert!(!bridge_probe_required_capabilities_ok(&stale_probe));

        let current_ping = json!({
            "success": true,
            "result": {
                "pong": true,
                "bridge_contract_version": EXPECTED_EXTENSION_BRIDGE_CONTRACT_VERSION,
                "capabilities": {
                    "content_keepalive_port": true,
                    "native_host_stdout_heartbeat": true,
                    "native_roundtrip_ping_health": true,
                    "native_port_epoch_fencing": true,
                    "workflow_session_epoch_fencing": true,
                    "broker_watchdog": true,
                    "request_lease_cancellation": true,
                    "watchdog_queue_unblock": true,
                    "epoch_chain_identity": true,
                    "native_control_epoch_fencing": true,
                    "supervisor_bridge_response_fencing": true,
                    "health_beacon_v2": true,
                    "auxiliary_path_lease_guards": true,
                    "port_scoped_disconnect_suppression": true,
                    "native_message_frame_cap": true
                }
            }
        });
        assert!(bridge_probe_transport_ok(&current_ping));
        let current_probe = json!({
            "required_capabilities": bridge_probe_required_capabilities_map(&current_ping),
            "bridge_contract_version_ok": bridge_probe_contract_version_ok(&current_ping),
        });
        assert!(bridge_probe_required_capabilities_ok(&current_probe));
    }

    #[test]
    fn heal_retries_probe_failures_but_not_stale_extension_contracts() {
        let timeout_readiness = json!({
            "ready": false,
            "native_host_bridge": {
                "connected": true,
                "probe": {
                    "error": "Native-host extension bridge timeout after 1500ms"
                }
            }
        });
        assert!(should_retry_heal_after_probe_reset(&timeout_readiness));

        let stale_extension_readiness = json!({
            "ready": false,
            "native_host_bridge": {
                "connected": true,
                "probe": {
                    "error": "loaded extension is missing content_keepalive_port capability"
                }
            }
        });
        assert!(!should_retry_heal_after_probe_reset(
            &stale_extension_readiness
        ));
    }

    #[test]
    fn supervisor_heal_request_timeout_covers_mv3_keepalive_wait() {
        let timeout_ms = supervisor_request_timeout_ms("runtime.heal", &json!({}));
        let expected = HEAL_BRIDGE_WAIT_MS
            + HEAL_INITIAL_BRIDGE_WAIT_MS
            + HEAL_POST_PROBE_RECONNECT_WAIT_MS
            + (DEFAULT_BRIDGE_PROBE_TIMEOUT_MS + BRIDGE_PROBE_TIMEOUT_GRACE_MS)
                * HEAL_READINESS_PROBE_COUNT
            + REQUEST_TIMEOUT_GRACE_MS;
        assert_eq!(timeout_ms, expected);
    }

    #[test]
    fn supervisor_heal_request_timeout_tracks_custom_probe_timeout() {
        let params = json!({
            "bridge_wait_ms": 10_000,
            "bridge_probe_timeout_ms": 7_000
        });
        let expected = 10_000
            + HEAL_INITIAL_BRIDGE_WAIT_MS
            + HEAL_POST_PROBE_RECONNECT_WAIT_MS
            + (7_000 + BRIDGE_PROBE_TIMEOUT_GRACE_MS) * HEAL_READINESS_PROBE_COUNT
            + REQUEST_TIMEOUT_GRACE_MS;

        assert_eq!(
            supervisor_request_timeout_ms("runtime.heal", &params),
            expected
        );
    }

    #[test]
    fn supervisor_regular_request_timeout_stays_short() {
        assert_eq!(
            supervisor_request_timeout_ms("runtime.status", &json!({})),
            REQUEST_TIMEOUT_MS + REQUEST_TIMEOUT_GRACE_MS
        );
    }

    #[test]
    fn browser_tool_request_timeout_outlives_inner_bridge_timeout() {
        assert_eq!(
            supervisor_request_timeout_ms(
                "browser.execute_step",
                &json!({
                    "timeout_ms": 42_000
                })
            ),
            42_000 + EXTENSION_BRIDGE_GRACE_MS + REQUEST_TIMEOUT_GRACE_MS
        );
    }

    #[test]
    fn tools_call_timeout_is_forwarded_into_arguments() {
        let args = tool_call_arguments(&json!({
            "name": "browser.execute_step",
            "timeout_ms": 42_000,
            "arguments": {
                "session_id": "s1",
                "step": { "type": "click" }
            }
        }));
        assert_eq!(args.get("timeout_ms"), Some(&json!(42_000)));
    }

    #[test]
    fn run_result_selects_output_warnings_artifacts_and_debug() {
        let response = json!({
            "ok": true,
            "workflow_id": "workflow/example",
            "result": { "answer": 42 },
            "warnings": [{ "code": "TRUNCATED", "message": "shortened" }],
            "downloads": [{ "path": "/tmp/file.txt" }]
        });

        let run_result = run_result_for_tool("browser.execute_step", &response);
        let typed = serde_json::from_value::<RunResultV2>(run_result.clone())
            .expect("supervisor emits contract-compatible RunResultV2");

        assert_eq!(typed.version.as_str(), RUN_RESULT_VERSION);
        assert_eq!(typed.workflow_id.as_str(), "workflow/example");
        assert_eq!(typed.status, RunStatusV2::Succeeded);
        assert_eq!(typed.output, Some(json!({ "answer": 42 })));
        assert_eq!(typed.warnings[0].code, "TRUNCATED");
        assert_eq!(typed.artifacts[0].kind, ArtifactKindV1::Download);
        assert_eq!(typed.artifacts[0].uri, "/tmp/file.txt");
        assert!(typed.debug.expect("debug").raw.is_some());
        assert_eq!(
            run_result.get("run_envelope"),
            None,
            "supervisor must not emit the old typed-looking run envelope"
        );
    }

    #[test]
    fn run_result_can_inherit_workflow_id_from_step_params() {
        let mut response = json!({
            "ok": true,
            "result": { "answer": 42 }
        });
        inherit_run_context_from_params(
            &mut response,
            &json!({
                "workflow_id": "x.open",
                "workflow_version": "0.1.0",
                "system": "x",
                "capability": "x.read.unified"
            }),
        );

        let run_result = run_result_for_tool("browser.execute_step", &response);
        let typed = serde_json::from_value::<RunResultV2>(run_result)
            .expect("supervisor emits contract-compatible RunResultV2");

        assert_eq!(typed.workflow_id, "x.open");
    }

    #[test]
    fn run_result_ignores_non_v2_embedded_envelope() {
        let response = json!({
            "ok": true,
            "workflow_id": "x.open",
            "result": { "answer": 42 },
            "run_result": { "version": "old", "workflow_id": "wrong" }
        });

        let run_result = run_result_for_tool("browser.execute_step", &response);
        let typed = serde_json::from_value::<RunResultV2>(run_result)
            .expect("supervisor emits contract-compatible RunResultV2");

        assert_eq!(typed.workflow_id, "x.open");
        assert_eq!(typed.status, RunStatusV2::Succeeded);
        assert_eq!(typed.output, Some(json!({ "answer": 42 })));
    }

    #[test]
    fn manifest_policy_blocks_undeclared_step_side_effects() {
        let params = json!({
            "session_id": "s1",
            "workflow_id": "x.open",
            "workflow_version": "0.1.0",
            "system": "x",
            "capability": "x.read",
            "step": { "type": "execute_javascript" },
            "side_effect_policy": {
                "enforce": true,
                "declared_side_effects": ["read_only"]
            }
        });

        let blocked = enforce_manifest_side_effect_policy("browser.execute_step", &params)
            .expect("browser state mutation must be declared");
        let typed = serde_json::from_value::<RunResultV2>(
            blocked
                .get("run_result")
                .cloned()
                .expect("blocked response includes run_result"),
        )
        .expect("blocked response emits contract-compatible RunResultV2");

        assert_eq!(blocked.get("ok"), Some(&json!(false)));
        assert_eq!(typed.workflow_id, "x.open");
        assert_eq!(
            blocked.get("error_code"),
            Some(&json!("POLICY_SIDE_EFFECT_UNDECLARED"))
        );
        assert_eq!(typed.status, RunStatusV2::PolicyBlocked);
        assert_eq!(
            blocked.pointer("/policy/undeclared_side_effects/0"),
            Some(&json!("browser_state"))
        );
    }

    #[test]
    fn manifest_policy_allows_declared_step_side_effects() {
        let params = json!({
            "session_id": "s1",
            "step": { "type": "execute_javascript" },
            "side_effect_policy": {
                "enforce": true,
                "declared_side_effects": ["browser_state"]
            }
        });

        assert!(enforce_manifest_side_effect_policy("browser.execute_step", &params).is_none());
    }

    #[test]
    fn observed_side_effects_use_manifest_v2_taxonomy() {
        let observed = observed_side_effects(
            "browser.execute_step",
            &json!({
                "step": { "type": "download_file" }
            }),
        );

        assert_eq!(
            observed,
            vec![
                "browser_state".to_string(),
                "download".to_string(),
                "external_read".to_string(),
                "file_write".to_string(),
                "network_access".to_string()
            ]
        );
        assert!(!observed.iter().any(|effect| effect == "browser_write"));
        assert!(!observed.iter().any(|effect| effect == "local_file_write"));
    }

    #[test]
    fn observed_side_effects_cover_static_commands() {
        assert_eq!(
            observed_side_effects(
                "browser.static_command",
                &json!({
                    "cmd": "observe"
                }),
            ),
            vec!["read_only".to_string()]
        );

        assert_eq!(
            observed_side_effects(
                "browser.static_command",
                &json!({
                    "cmd": "enable_debug"
                }),
            ),
            vec!["browser_state".to_string()]
        );
    }

    #[test]
    fn manifest_policy_blocks_undeclared_static_command_side_effects() {
        let params = json!({
            "cmd": "enable_debug",
            "payload": { "mode": "rescue" },
            "side_effect_policy": {
                "enforce": true,
                "declared_side_effects": ["read_only"]
            }
        });

        let blocked = enforce_manifest_side_effect_policy("browser.static_command", &params)
            .expect("enable_debug must declare browser_state");
        assert_eq!(
            blocked.pointer("/policy/undeclared_side_effects/0"),
            Some(&json!("browser_state"))
        );
    }
}
