use anyhow::{anyhow, Context, Result};
use interprocess::local_socket::{
    tokio::Stream as LocalSocketStream,
    traits::tokio::{Listener as _, Stream as _},
    GenericFilePath, ListenerOptions, ToFsName,
};
use rzn_broker_endpoint::prune_stale_broker_endpoint;
use rzn_contracts::v1::{
    ActionResultV1, CapabilitiesV1, CloudBrowserCommandV1, CloudCommandEnvelopeV1,
    CloudCommandResultV1, CLOUD_CONTRACT_VERSION,
};
use rzn_contracts::v2::{
    ArtifactKindV1, ArtifactV1, DebugBundleV1, DebugEventV1, RunErrorV1, RunResultV2, RunStatusV2,
    RunWarningV1, SideEffectClassV2, RUN_RESULT_VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::{timeout, Duration};
use uuid::Uuid;

use crate::native_runner::{self, NativeClient, NativeRunConfig, NativeRunMode, SnapshotMode};
use crate::supervisor_cloud::{self, CloudDispatchRequest, SupervisorCloudActor};

pub(crate) const RZN_LOCAL_PROTOCOL_VERSION: &str = "rzn.local.v1";
const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;
const HANDSHAKE_TIMEOUT_MS: u64 = 2_000;
const REQUEST_TIMEOUT_MS: u64 = 30_000;
const REQUEST_TIMEOUT_GRACE_MS: u64 = 5_000;
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
const REQUIRED_EXTENSION_KEEPALIVE_CAPABILITY: &str = "content_keepalive_port";
const SUPERVISOR_SOCKET_FILENAME: &str = "rzn-supervisor.sock";
const SUPERVISOR_TOKEN_FILENAME: &str = "rzn-supervisor-token-v1";

#[derive(Clone, Debug)]
pub(crate) struct SupervisorConfig {
    pub app_base: Option<PathBuf>,
    pub endpoint_path: Option<String>,
    pub mode: NativeRunMode,
    pub worker_cmd: Option<String>,
    pub worker_args: Vec<String>,
    pub allow_legacy_worker_fallback: bool,
}

impl SupervisorConfig {
    pub(crate) fn app_base_dir(&self) -> PathBuf {
        self.app_base
            .clone()
            .or_else(|| env_path("RZN_SUPERVISOR_APP_BASE"))
            .or_else(|| env_path("RZN_NATIVE_APP_BASE"))
            .or_else(|| env_path("RZN_APP_BASE"))
            .or_else(|| env_path("APP_BASE"))
            .unwrap_or_else(default_app_base_dir)
    }

    fn native_run_config(&self) -> NativeRunConfig {
        NativeRunConfig {
            workflow_path: String::new(),
            params: HashMap::new(),
            mode: self.mode,
            snapshot_mode: SnapshotMode::OnError,
            app_base: Some(self.app_base_dir().to_string_lossy().to_string()),
            endpoint_path: self.endpoint_path.clone(),
            worker_cmd: self.worker_cmd.clone(),
            worker_args: self.worker_args.clone(),
        }
    }

    fn allows_legacy_worker_fallback(&self) -> bool {
        self.allow_legacy_worker_fallback || env_flag("RZN_SUPERVISOR_ALLOW_LEGACY_WORKER_FALLBACK")
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SupervisorPaths {
    pub app_base: PathBuf,
    pub secure_dir: PathBuf,
    pub run_dir: PathBuf,
    pub socket_path: PathBuf,
    pub token_path: PathBuf,
}

impl SupervisorPaths {
    pub(crate) fn for_config(config: &SupervisorConfig) -> Self {
        let app_base = config.app_base_dir();
        let secure_dir = app_base.join("secure");
        let run_dir = app_base.join("run");
        Self {
            app_base,
            socket_path: run_dir.join(SUPERVISOR_SOCKET_FILENAME),
            token_path: secure_dir.join(SUPERVISOR_TOKEN_FILENAME),
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
    browser_client: Mutex<Option<NativeClient>>,
    native_bridge: Mutex<Option<NativeHostBridge>>,
    native_bridge_pending: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    cloud_actor: Option<SupervisorCloudActor>,
    sessions: Mutex<HashSet<String>>,
    shutdown: AtomicBool,
}

#[derive(Clone)]
struct NativeHostBridge {
    id: String,
    tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl SupervisorState {
    fn new(config: SupervisorConfig) -> Self {
        let paths = SupervisorPaths::for_config(&config);
        Self {
            config,
            paths,
            browser_client: Mutex::new(None),
            native_bridge: Mutex::new(None),
            native_bridge_pending: Mutex::new(HashMap::new()),
            cloud_actor: None,
            sessions: Mutex::new(HashSet::new()),
            shutdown: AtomicBool::new(false),
        }
    }

    fn with_cloud_actor(config: SupervisorConfig, cloud_actor: SupervisorCloudActor) -> Self {
        let mut state = Self::new(config);
        state.cloud_actor = Some(cloud_actor);
        state
    }

    async fn dispatch(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "runtime.hello" | "runtime.status" => Ok(self.runtime_status(false).await),
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
            "rzn.supervisor.health" => Ok(self.runtime_status(false).await),
            "rzn.worker.health" => {
                let worker = self
                    .call_browser_worker("rzn.worker.health", params, None)
                    .await?;
                Ok(json!({
                    "ok": true,
                    "supervisor": self.runtime_status(false).await,
                    "worker": worker
                }))
            }
            "rzn.worker.shutdown" => {
                let mut guard = self.browser_client.lock().await;
                let had_worker = guard.is_some();
                if let Some(client) = guard.as_mut() {
                    client.shutdown().await;
                }
                *guard = None;
                Ok(json!({
                    "ok": true,
                    "legacy_worker_shutdown": had_worker,
                    "supervisor_shutdown": false
                }))
            }
            "browser.session_open"
            | "browser.session_close"
            | "browser.snapshot"
            | "browser.execute_step"
            | "browser.poll_events" => {
                if let Some(value) = self
                    .try_dispatch_supervisor_browser_tool(name, params.clone())
                    .await?
                {
                    Ok(value)
                } else if self.config.allows_legacy_worker_fallback() {
                    self.call_browser_worker(name, params, None).await
                } else {
                    Err(self.native_bridge_required_error(name))
                }
            }
            _ => Err(anyhow!("Unknown supervisor tool: {}", name)),
        }
    }

    async fn runtime_status(&self, include_worker: bool) -> Value {
        let legacy_worker_fallback_allowed = self.config.allows_legacy_worker_fallback();
        let mut status = json!({
            "ok": true,
            "protocol": RZN_LOCAL_PROTOCOL_VERSION,
            "pid": std::process::id(),
            "app_base": self.paths.app_base.to_string_lossy(),
            "socket_path": self.paths.socket_path.to_string_lossy(),
            "token_path": self.paths.token_path.to_string_lossy(),
            "browser_proxy": {
                "mode": if legacy_worker_fallback_allowed {
                    "native_host_bridge_preferred_legacy_worker_fallback"
                } else {
                    "native_host_bridge_required"
                },
                "authority": "supervisor_handshake",
                "legacy_worker_fallback_allowed": legacy_worker_fallback_allowed
            },
            "native_host_bridge": {
                "connected": self.native_bridge.lock().await.is_some()
            },
            "cloud": self.cloud_status().await
        });

        if include_worker {
            let worker = self
                .call_browser_worker("rzn.worker.health", json!({}), Some(2_500))
                .await;
            status["worker"] = match worker {
                Ok(value) => value,
                Err(err) => json!({
                    "ok": false,
                    "error": err.to_string()
                }),
            };
        }

        status
    }

    async fn ensure_ready(&self, params: Value) -> Result<Value> {
        let prune = prune_stale_broker_endpoint(&self.paths.app_base).ok();
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
            Some(self.probe_native_bridge(bridge_probe_timeout_ms).await)
        } else {
            None
        };
        let native_host_bridge_ready = native_host_bridge_connected
            && bridge_probe
                .as_ref()
                .map(|probe| probe.get("ok").and_then(|value| value.as_bool()) == Some(true))
                .unwrap_or(true);
        let legacy_worker_fallback_allowed = self.config.allows_legacy_worker_fallback();
        let worker = if !native_host_bridge_ready && legacy_worker_fallback_allowed {
            Some(
                self.call_browser_worker("rzn.worker.health", json!({}), Some(5_000))
                    .await,
            )
        } else {
            None
        };
        let worker_ready = worker.as_ref().map(Result::is_ok).unwrap_or(false);
        let ready = native_host_bridge_ready || (legacy_worker_fallback_allowed && worker_ready);
        Ok(json!({
            "ok": ready,
            "ready": ready,
            "protocol": RZN_LOCAL_PROTOCOL_VERSION,
            "pid": std::process::id(),
            "app_base": self.paths.app_base.to_string_lossy(),
            "browser_proxy": {
                "mode": if legacy_worker_fallback_allowed {
                    "native_host_bridge_preferred_legacy_worker_fallback"
                } else {
                    "native_host_bridge_required"
                },
                "legacy_worker_fallback_allowed": legacy_worker_fallback_allowed
            },
            "native_host_bridge": {
                "connected": native_host_bridge_connected,
                "responsive": native_host_bridge_ready,
                "probe": bridge_probe,
                "wait_ms": wait_ms
            },
            "prune": prune,
            "worker": match worker {
                Some(Ok(value)) => value,
                Some(Err(err)) => json!({ "ok": false, "error": err.to_string() }),
                None => json!({
                    "ok": native_host_bridge_ready,
                    "skipped": true,
                    "reason": if native_host_bridge_ready {
                        "native_host_bridge_connected"
                    } else {
                        "legacy_worker_fallback_disabled"
                    }
                }),
            },
            "error": if ready {
                Value::Null
            } else if native_host_bridge_connected {
                json!("Native-host bridge is connected but the loaded extension failed the readiness contract. It either did not answer the ping or is an older bundle missing the content keepalive bridge capability. Reload the RZN extension, then retry.")
            } else {
                json!("Native-host bridge is not connected. Open Chrome with the RZN extension enabled, then retry. Use --allow-legacy-worker-fallback only for compatibility debugging.")
            },
            "remediation": if ready {
                json!([])
            } else {
                json!([
                    "Open the existing Chrome profile with the RZN extension enabled.",
                    "Reload the extension if Chrome has suspended or restarted the service worker.",
                    "Run `rzn-browser heal --json` to prune stale runtime files and re-check the supervisor bridge.",
                    "Run `rzn-browser supervisor status --json` and confirm native_host_bridge.connected is true."
                ])
            }
        }))
    }

    async fn probe_native_bridge(&self, timeout_ms: u64) -> Value {
        match self
            .try_call_native_bridge(
                "ping",
                json!({
                    "source": "supervisor.ensure_ready",
                    "timeout_ms": timeout_ms.max(1),
                    "timeout_grace_ms": 500
                }),
            )
            .await
        {
            Ok(Some(value)) => {
                let transport_ok = bridge_probe_transport_ok(&value);
                let keepalive_capability_ok = bridge_probe_has_required_keepalive(&value);
                let extension_build_signature = value
                    .pointer("/result/extension_build_signature")
                    .cloned()
                    .unwrap_or(Value::Null);
                json!({
                    "ok": transport_ok && keepalive_capability_ok,
                    "transport_ok": transport_ok,
                    "required_capabilities": {
                        "content_keepalive_port": keepalive_capability_ok
                    },
                    "extension_build_signature": extension_build_signature,
                    "timeout_ms": timeout_ms,
                    "error": if transport_ok && !keepalive_capability_ok {
                        json!("loaded extension is missing content_keepalive_port capability; reload the extension so the current bridge hardening code is active")
                    } else {
                        Value::Null
                    },
                    "response": value
                })
            }
            Ok(None) => json!({
                "ok": false,
                "timeout_ms": timeout_ms,
                "error": "native-host bridge disappeared before ping"
            }),
            Err(err) => json!({
                "ok": false,
                "timeout_ms": timeout_ms,
                "error": err.to_string()
            }),
        }
    }

    async fn wait_for_native_bridge(&self, wait_ms: u64) -> bool {
        if self.native_bridge.lock().await.is_some() {
            return true;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms);
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if self.native_bridge.lock().await.is_some() {
                return true;
            }
        }
        false
    }

    fn native_bridge_required_error(&self, tool_name: &str) -> anyhow::Error {
        anyhow!(
            "Browser tool '{}' requires the supervisor native-host bridge, but no bridge is connected. Open Chrome with the RZN extension enabled, reload the extension if needed, then retry. Legacy worker fallback is disabled on the supervisor default path.",
            tool_name
        )
    }

    async fn runtime_heal(&self, params: Value) -> Result<Value> {
        let prune = prune_stale_broker_endpoint(&self.paths.app_base)
            .map_err(|err| anyhow!("Prune stale broker endpoint failed: {}", err))?;
        let heal_bridge_wait_ms = heal_bridge_wait_ms(&params);
        let bridge_probe_timeout_ms = bridge_probe_timeout_ms(&params);
        let initial_readiness = self
            .ensure_ready(json!({
                "bridge_wait_ms": HEAL_INITIAL_BRIDGE_WAIT_MS,
                "bridge_probe_timeout_ms": bridge_probe_timeout_ms
            }))
            .await?;
        let mut final_readiness = if initial_readiness
            .get("ready")
            .and_then(|value| value.as_bool())
            == Some(true)
        {
            initial_readiness.clone()
        } else {
            self.ensure_ready(json!({
                "bridge_wait_ms": heal_bridge_wait_ms,
                "bridge_probe_timeout_ms": bridge_probe_timeout_ms
            }))
            .await?
        };
        let post_probe_recovery_attempted = should_retry_heal_after_probe_reset(&final_readiness);
        if post_probe_recovery_attempted {
            final_readiness = self
                .ensure_ready(json!({
                    "bridge_wait_ms": HEAL_POST_PROBE_RECONNECT_WAIT_MS,
                    "bridge_probe_timeout_ms": bridge_probe_timeout_ms
                }))
                .await?;
        }
        let status = self
            .runtime_status(self.config.allows_legacy_worker_fallback())
            .await;
        let ready = final_readiness
            .get("ready")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        Ok(json!({
            "ok": ready,
            "ready": ready,
            "protocol": RZN_LOCAL_PROTOCOL_VERSION,
            "prune": prune,
            "heal_wait_ms": heal_bridge_wait_ms,
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
        let native_host_bridge_connected = self.native_bridge.lock().await.is_some();
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
        let native_host_bridge_connected = self.native_bridge.lock().await.is_some();
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
        let native_host_bridge_connected = self.native_bridge.lock().await.is_some();
        actor.clear_config(native_host_bridge_connected).await
    }

    async fn call_browser_worker(
        &self,
        name: &str,
        params: Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value> {
        let mut guard = self.browser_client.lock().await;
        if guard.is_none() {
            let client = native_runner::connect_native(&self.config.native_run_config()).await?;
            *guard = Some(client);
        }

        let client = guard
            .as_mut()
            .ok_or_else(|| anyhow!("Browser client unavailable after connect"))?;

        let result = match timeout_ms {
            Some(ms) => client.send_request_with_timeout(name, params, ms).await,
            None => client.send_request(name, params).await,
        };

        if result.is_err() {
            *guard = None;
        }

        result
    }

    async fn try_dispatch_supervisor_browser_tool(
        &self,
        name: &str,
        params: Value,
    ) -> Result<Option<Value>> {
        if let Some(blocked) = enforce_manifest_side_effect_policy(name, &params) {
            return Ok(Some(blocked));
        }

        match name {
            "browser.session_open" => self.try_session_open(params).await,
            "browser.session_close" => self.try_session_close(params).await,
            "browser.poll_events" => self.try_poll_events(params).await,
            "browser.snapshot" => {
                self.try_call_native_bridge("get_dom_snapshot", params)
                    .await
            }
            "browser.execute_step" => self.try_call_native_bridge("execute_step", params).await,
            _ => Ok(None),
        }
    }

    async fn try_session_open(&self, params: Value) -> Result<Option<Value>> {
        if self.native_bridge.lock().await.is_none() {
            return Ok(None);
        }

        let session_id = Uuid::new_v4().to_string();
        self.sessions.lock().await.insert(session_id.clone());
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
            "url": ""
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
        let response = json!({
            "ok": true,
            "session_id": session_id,
            "events": []
        });
        Ok(Some(with_run_result("browser.poll_events", response, true)))
    }

    async fn try_call_native_bridge(&self, cmd: &str, params: Value) -> Result<Option<Value>> {
        self.try_call_native_bridge_raw(cmd, params, None, None, None)
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
        let mut bridge = self.native_bridge.lock().await.clone();
        if bridge.is_none()
            && self
                .wait_for_native_bridge(BRIDGE_RECONNECT_BEFORE_CALL_WAIT_MS)
                .await
        {
            bridge = self.native_bridge.lock().await.clone();
        }
        let Some(bridge) = bridge else {
            return Ok(None);
        };
        let mut active_bridge_id = bridge.id.clone();

        let id = format!("native-call-{}", Uuid::new_v4());
        let req_id = req_id_override.unwrap_or_else(|| format!("supervisor-{}", Uuid::new_v4()));
        let timeout_ms = timeout_ms_override.unwrap_or_else(|| extension_timeout_ms(&payload));
        let (tx, rx) = oneshot::channel();
        self.native_bridge_pending
            .lock()
            .await
            .insert(id.clone(), tx);

        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "native_host.extension_call",
            "params": {
                "cmd": cmd,
                "payload": payload,
                "req_id": req_id,
                "timeout_ms": timeout_ms
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
            let pending_tx = self.native_bridge_pending.lock().await.remove(&request_id);
            self.clear_native_bridge(&bridge.id).await;
            if !self
                .wait_for_native_bridge(BRIDGE_RECONNECT_BEFORE_CALL_WAIT_MS)
                .await
            {
                return Ok(None);
            }

            let Some(reconnected_bridge) = self.native_bridge.lock().await.clone() else {
                return Ok(None);
            };
            if let Some(pending_tx) = pending_tx {
                self.native_bridge_pending
                    .lock()
                    .await
                    .insert(request_id.clone(), pending_tx);
            } else {
                return Ok(None);
            }
            let retry_bytes = serde_json::to_vec(&request)?;
            if reconnected_bridge.tx.send(retry_bytes).is_err() {
                self.native_bridge_pending.lock().await.remove(&request_id);
                self.clear_native_bridge(&reconnected_bridge.id).await;
                return Ok(None);
            }
            active_bridge_id = reconnected_bridge.id.clone();
        }

        let response = match timeout(Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                self.native_bridge_pending.lock().await.remove(&request_id);
                self.clear_native_bridge(&active_bridge_id).await;
                return Ok(None);
            }
            Err(_) => {
                self.native_bridge_pending.lock().await.remove(&request_id);
                self.clear_native_bridge(&active_bridge_id).await;
                return Err(anyhow!(
                    "Native-host extension bridge timeout after {}ms",
                    timeout_ms
                ));
            }
        };

        if let Some(error) = response.get("error") {
            return Err(anyhow!("Native-host extension bridge error: {}", error));
        }
        let mut result = response.get("result").cloned().unwrap_or(response);
        inherit_run_context_from_params(&mut result, &payload);
        Ok(Some(with_run_result(cmd, result, true)))
    }

    async fn register_native_bridge(&self, id: String, tx: mpsc::UnboundedSender<Vec<u8>>) {
        *self.native_bridge.lock().await = Some(NativeHostBridge { id, tx });
    }

    async fn clear_native_bridge(&self, id: &str) {
        let mut guard = self.native_bridge.lock().await;
        if guard.as_ref().map(|bridge| bridge.id.as_str()) == Some(id) {
            *guard = None;
        }
    }

    async fn complete_native_bridge_response(&self, value: &Value) -> bool {
        let Some(id) = value.get("id").and_then(value_id_string) else {
            return false;
        };
        let tx = self.native_bridge_pending.lock().await.remove(&id);
        if let Some(tx) = tx {
            let _ = tx.send(value.clone());
            true
        } else {
            false
        }
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
        if !map
            .get("run_result")
            .is_some_and(|value| is_run_result_v2(value))
        {
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

pub(crate) async fn serve(config: SupervisorConfig) -> Result<SupervisorServeReport> {
    let (cloud_dispatch_tx, cloud_dispatch_rx) = mpsc::unbounded_channel::<CloudDispatchRequest>();
    let cloud_actor = supervisor_cloud::spawn_cloud_actor(cloud_dispatch_tx);
    let state = Arc::new(SupervisorState::with_cloud_actor(config, cloud_actor));
    tokio::spawn(handle_cloud_dispatch_requests(
        state.clone(),
        cloud_dispatch_rx,
    ));
    prepare_paths(&state.paths)?;

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
            let mut guard = state.browser_client.lock().await;
            if let Some(client) = guard.as_mut() {
                client.shutdown().await;
            }
            break;
        }
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => {
                let mut guard = state.browser_client.lock().await;
                if let Some(client) = guard.as_mut() {
                    client.shutdown().await;
                }
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
    match config.mode {
        NativeRunMode::Auto => {}
        NativeRunMode::Attach => {
            command.arg("--mode").arg("attach");
        }
        NativeRunMode::Spawn => {
            command.arg("--mode").arg("spawn");
        }
    }
    if let Some(endpoint_path) = config.endpoint_path.as_ref() {
        command.arg("--endpoint-path").arg(endpoint_path);
    }
    if let Some(worker_cmd) = config.worker_cmd.as_ref() {
        command.arg("--worker-cmd").arg(worker_cmd);
    }
    for arg in &config.worker_args {
        command.arg("--worker-arg").arg(arg);
    }
    if config.allow_legacy_worker_fallback {
        command.arg("--allow-legacy-worker-fallback");
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
        read_frame(&mut stream),
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
        send_frame(&mut stream, &serde_json::to_vec(&response)?).await?;
        return Err(anyhow!("Invalid supervisor handshake"));
    }

    let response = json!({
        "ok": true,
        "protocol": RZN_LOCAL_PROTOCOL_VERSION,
        "pid": std::process::id()
    });
    send_frame(&mut stream, &serde_json::to_vec(&response)?).await?;

    loop {
        let frame = match read_frame(&mut stream).await {
            Ok(frame) => frame,
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
        send_frame(&mut stream, &serde_json::to_vec(&response)?).await?;
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
        send_frame(&mut stream, &serde_json::to_vec(&response)?).await?;
        return Err(anyhow!("Invalid native-host bridge hello"));
    }

    let bridge_id = format!("native-host-{}", Uuid::new_v4());
    let (mut reader, mut writer) = stream.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    state.register_native_bridge(bridge_id.clone(), tx).await;

    let response = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "ok": true,
            "protocol": RZN_LOCAL_PROTOCOL_VERSION,
            "bridge_id": bridge_id,
            "pid": std::process::id(),
            "accepts": ["native_host.extension_call"]
        }
    });
    send_frame(&mut writer, &serde_json::to_vec(&response)?).await?;

    let writer_task = tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if send_frame(&mut writer, &bytes).await.is_err() {
                break;
            }
        }
    });

    loop {
        let frame = match read_frame(&mut reader).await {
            Ok(frame) => frame,
            Err(_) => break,
        };
        let value: Value = match serde_json::from_slice(&frame) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if state.complete_native_bridge_response(&value).await {
            continue;
        }
    }

    state.clear_native_bridge(&bridge_id).await;
    let _ = writer_task.await;
    Ok(())
}

struct SupervisorClient {
    stream: LocalSocketStream,
}

impl SupervisorClient {
    async fn connect_with_paths(paths: SupervisorPaths) -> Result<Self> {
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
        send_frame(&mut stream, &serde_json::to_vec(&handshake)?).await?;
        let response = timeout(
            Duration::from_millis(HANDSHAKE_TIMEOUT_MS),
            read_frame(&mut stream),
        )
        .await
        .context("Supervisor handshake response timeout")??;
        let value: Value = serde_json::from_slice(&response)?;
        if value.get("ok").and_then(|value| value.as_bool()) != Some(true) {
            return Err(anyhow!("Supervisor handshake failed: {}", value));
        }
        Ok(Self { stream })
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
        send_frame(&mut self.stream, &serde_json::to_vec(&request)?).await?;
        let response = timeout(
            Duration::from_millis(timeout_ms),
            read_frame(&mut self.stream),
        )
        .await
        .context("Supervisor request timeout")??;
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
        return read_token(path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let token = Uuid::new_v4().to_string();
    std::fs::write(path, format!("{}\n", token))?;
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

async fn send_frame<W: AsyncWrite + Unpin>(writer: &mut W, bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_FRAME_SIZE {
        return Err(anyhow!("Supervisor frame too large: {}", bytes.len()));
    }
    writer
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .await?;
    writer.write_all(bytes).await?;
    writer.flush().await?;
    Ok(())
}

async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    if len == 0 {
        return Err(anyhow!("Supervisor frame is empty"));
    }
    if len > MAX_FRAME_SIZE {
        return Err(anyhow!("Supervisor frame exceeds limit: {}", len));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

fn is_browser_tool(name: &str) -> bool {
    matches!(
        name,
        "browser.session_open"
            | "browser.session_close"
            | "browser.snapshot"
            | "browser.execute_step"
            | "browser.poll_events"
            | "rzn.worker.health"
            | "rzn.worker.shutdown"
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

    requested.max(1).saturating_add(grace_ms)
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

fn bridge_probe_has_required_keepalive(value: &Value) -> bool {
    value
        .pointer(&format!(
            "/result/capabilities/{}",
            REQUIRED_EXTENSION_KEEPALIVE_CAPABILITY
        ))
        .and_then(|v| v.as_bool())
        == Some(true)
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

    requested
        .unwrap_or(default_timeout_ms)
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

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn env_flag(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn default_app_base_dir() -> PathBuf {
    if let Some(root) = dirs::data_local_dir().or_else(dirs::data_dir) {
        return root.join("rzn-browser");
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".rzn-browser");
    }
    PathBuf::from(".rzn-browser")
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

    fn test_config() -> SupervisorConfig {
        SupervisorConfig {
            app_base: Some(PathBuf::from("/tmp/rzn-supervisor-test")),
            endpoint_path: None,
            mode: NativeRunMode::Auto,
            worker_cmd: None,
            worker_args: Vec::new(),
            allow_legacy_worker_fallback: false,
        }
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
    }

    #[test]
    fn browser_tool_allowlist_is_explicit() {
        assert!(is_browser_tool("browser.execute_step"));
        assert!(is_browser_tool("rzn.worker.health"));
        assert!(is_browser_tool("rzn.worker.shutdown"));
        assert!(!is_browser_tool("runtime.status"));
        assert!(!is_browser_tool("shell.exec"));
    }

    #[tokio::test]
    async fn strict_supervisor_does_not_fall_back_to_worker_without_native_bridge() {
        let state = SupervisorState::new(test_config());
        let err = state
            .dispatch("browser.session_open", json!({}))
            .await
            .expect_err("strict supervisor should require native-host bridge");
        assert!(err
            .to_string()
            .contains("requires the supervisor native-host bridge"));
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
        assert!(state.native_bridge.lock().await.is_none());
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
    fn bridge_probe_requires_content_keepalive_capability() {
        let stale_ping = json!({
            "success": true,
            "result": { "pong": true }
        });
        assert!(bridge_probe_transport_ok(&stale_ping));
        assert!(!bridge_probe_has_required_keepalive(&stale_ping));

        let current_ping = json!({
            "success": true,
            "result": {
                "pong": true,
                "capabilities": {
                    "content_keepalive_port": true
                }
            }
        });
        assert!(bridge_probe_transport_ok(&current_ping));
        assert!(bridge_probe_has_required_keepalive(&current_ping));
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
}
