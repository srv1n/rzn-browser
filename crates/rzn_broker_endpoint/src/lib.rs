use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

const ENDPOINT_FILENAME: &str = "broker_endpoint_v1.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerEndpointV1 {
    pub v: u32,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub app_base_dir: Option<String>,
    #[serde(default)]
    pub broker: Option<BrokerEndpointBroker>,
    #[serde(default)]
    pub browser_native: Option<BrokerEndpointBrowserNative>,
    #[serde(default)]
    pub browser_bridge: Option<BrokerEndpointBrowserBridge>,
    #[serde(default)]
    pub browser_worker: Option<BrokerEndpointBrowserWorker>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: Map<String, Value>,
}

impl Default for BrokerEndpointV1 {
    fn default() -> Self {
        Self {
            v: 1,
            updated_at: None,
            app_base_dir: None,
            broker: None,
            browser_native: None,
            browser_bridge: None,
            browser_worker: None,
            extra: Map::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerEndpointBroker {
    pub socket: String,
    pub token_path: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerEndpointBrowserNative {
    pub name: String,
    pub extension_ids: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub browsers: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerEndpointBrowserBridge {
    pub socket: String,
    pub token_path: String,
    #[serde(default)]
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerEndpointBrowserWorker {
    pub socket: String,
    pub token_path: String,
    #[serde(default)]
    pub pid: Option<u32>,
}

pub fn broker_endpoint_path(app_base: &Path) -> PathBuf {
    app_base.join("secure").join(ENDPOINT_FILENAME)
}

pub fn read_broker_endpoint_from_path(path: &Path) -> Option<BrokerEndpointV1> {
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn read_broker_endpoint(app_base: &Path) -> Option<BrokerEndpointV1> {
    read_broker_endpoint_from_path(&broker_endpoint_path(app_base))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrokerEndpointPruneReport {
    pub app_base_dir: String,
    pub endpoint_path: String,
    pub endpoint_existed: bool,
    pub removed_broker: bool,
    pub removed_browser_bridge: bool,
    pub removed_browser_worker: bool,
    pub removed_socket_paths: Vec<String>,
    pub reasons: Vec<String>,
}

impl BrokerEndpointPruneReport {
    pub fn changed(&self) -> bool {
        self.removed_broker
            || self.removed_browser_bridge
            || self.removed_browser_worker
            || !self.removed_socket_paths.is_empty()
    }
}

#[derive(Debug, Clone)]
struct StaleEndpointDecision {
    reason: String,
    remove_socket: bool,
}

pub fn endpoint_pid_is_live(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid_i32) = i32::try_from(pid) else {
            return false;
        };
        if pid_i32 <= 0 {
            return false;
        }

        let rc = unsafe { libc::kill(pid_i32, 0) };
        if rc == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    #[cfg(target_os = "windows")]
    {
        if pid == 0 {
            return false;
        }
        let filter = format!("PID eq {}", pid);
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &filter, "/NH"])
            .output();
        let Ok(output) = output else {
            return false;
        };
        if !output.status.success() {
            return false;
        }
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .any(|part| part == pid.to_string())
    }

    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = pid;
        false
    }
}

fn stale_endpoint_decision(
    section: &str,
    socket: &str,
    token_path: &str,
    pid: Option<u32>,
) -> Option<StaleEndpointDecision> {
    let socket_path = Path::new(socket);
    let token_path = Path::new(token_path);
    let socket_exists = socket_path.exists();
    let token_exists = token_path.exists();
    let pid_dead = pid.map(|pid| !endpoint_pid_is_live(pid)).unwrap_or(false);

    if socket_exists && token_exists && !pid_dead {
        return None;
    }

    let mut parts = Vec::new();
    if !socket_exists {
        parts.push(format!("socket missing: {}", socket_path.display()));
    }
    if !token_exists {
        parts.push(format!("token missing: {}", token_path.display()));
    }
    if pid_dead {
        if let Some(pid) = pid {
            parts.push(format!("pid is not live: {}", pid));
        }
    }

    Some(StaleEndpointDecision {
        reason: format!("{} stale ({})", section, parts.join(", ")),
        remove_socket: socket_exists && (pid_dead || pid.is_none()),
    })
}

pub fn prune_stale_broker_endpoint(app_base: &Path) -> Result<BrokerEndpointPruneReport, String> {
    let path = broker_endpoint_path(app_base);
    let mut report = BrokerEndpointPruneReport {
        app_base_dir: app_base.to_string_lossy().to_string(),
        endpoint_path: path.to_string_lossy().to_string(),
        ..BrokerEndpointPruneReport::default()
    };

    let mut current = match read_broker_endpoint(app_base) {
        Some(current) => {
            report.endpoint_existed = true;
            current
        }
        None => return Ok(report),
    };

    let mut changed = false;
    let mut sockets_to_remove = Vec::new();

    if let Some(endpoint) = current.broker.as_ref() {
        if let Some(decision) = stale_endpoint_decision(
            "broker",
            &endpoint.socket,
            &endpoint.token_path,
            endpoint.pid,
        ) {
            report.reasons.push(decision.reason);
            if decision.remove_socket {
                sockets_to_remove.push(endpoint.socket.clone());
            }
            current.broker = None;
            report.removed_broker = true;
            changed = true;
        }
    }

    if let Some(endpoint) = current.browser_bridge.as_ref() {
        if let Some(decision) = stale_endpoint_decision(
            "browser_bridge",
            &endpoint.socket,
            &endpoint.token_path,
            endpoint.pid,
        ) {
            report.reasons.push(decision.reason);
            if decision.remove_socket {
                sockets_to_remove.push(endpoint.socket.clone());
            }
            current.browser_bridge = None;
            report.removed_browser_bridge = true;
            changed = true;
        }
    }

    if let Some(endpoint) = current.browser_worker.as_ref() {
        if let Some(decision) = stale_endpoint_decision(
            "browser_worker",
            &endpoint.socket,
            &endpoint.token_path,
            endpoint.pid,
        ) {
            report.reasons.push(decision.reason);
            if decision.remove_socket {
                sockets_to_remove.push(endpoint.socket.clone());
            }
            current.browser_worker = None;
            report.removed_browser_worker = true;
            changed = true;
        }
    }

    if changed {
        current.updated_at = Some(Utc::now().to_rfc3339());
        current.app_base_dir = Some(app_base.to_string_lossy().to_string());

        if endpoint_is_empty(&current) {
            if path.exists() {
                fs::remove_file(&path).map_err(|e| format!("remove endpoint {:?}: {}", path, e))?;
            }
        } else {
            let bytes = serde_json::to_vec_pretty(&current)
                .map_err(|e| format!("serialize endpoint: {}", e))?;
            save_atomic_json(&path, &bytes)?;
        }
    }

    sockets_to_remove.sort();
    sockets_to_remove.dedup();
    for socket in sockets_to_remove {
        let socket_path = Path::new(&socket);
        match fs::remove_file(socket_path) {
            Ok(()) => report.removed_socket_paths.push(socket),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => report.reasons.push(format!(
                "failed to remove stale socket {}: {}",
                socket_path.display(),
                err
            )),
        }
    }

    Ok(report)
}

fn save_atomic_json(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create dir {:?}: {}", parent, e))?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes).map_err(|e| format!("write tmp {:?}: {}", tmp, e))?;
    let bak = path.with_extension("json.bak");
    if path.exists() {
        if bak.exists() {
            let _ = fs::remove_file(&bak);
        }
        fs::rename(path, &bak).map_err(|e| format!("rename to bak {:?}: {}", bak, e))?;
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        if bak.exists() {
            let _ = fs::rename(&bak, path);
        }
        return Err(format!("rename tmp to final {:?}: {}", path, e));
    }
    if bak.exists() {
        let _ = fs::remove_file(&bak);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

fn update_endpoint<F>(app_base: &Path, f: F) -> Result<(), String>
where
    F: FnOnce(BrokerEndpointV1) -> BrokerEndpointV1,
{
    let mut current = read_broker_endpoint(app_base).unwrap_or_default();
    current.v = 1;
    current.updated_at = Some(Utc::now().to_rfc3339());
    current.app_base_dir = Some(app_base.to_string_lossy().to_string());

    let updated = f(current);
    let bytes =
        serde_json::to_vec_pretty(&updated).map_err(|e| format!("serialize endpoint: {}", e))?;
    save_atomic_json(&broker_endpoint_path(app_base), &bytes)?;
    Ok(())
}

fn endpoint_is_empty(ep: &BrokerEndpointV1) -> bool {
    ep.broker.is_none()
        && ep.browser_native.is_none()
        && ep.browser_bridge.is_none()
        && ep.browser_worker.is_none()
        && ep.extra.is_empty()
}

fn clear_endpoint_if<F>(app_base: &Path, f: F) -> Result<(), String>
where
    F: FnOnce(&mut BrokerEndpointV1) -> bool,
{
    let path = broker_endpoint_path(app_base);
    let mut current = match read_broker_endpoint(app_base) {
        Some(current) => current,
        None => return Ok(()),
    };

    let changed = f(&mut current);
    if !changed {
        return Ok(());
    }

    current.updated_at = Some(Utc::now().to_rfc3339());
    current.app_base_dir = Some(app_base.to_string_lossy().to_string());

    if endpoint_is_empty(&current) {
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("remove endpoint {:?}: {}", path, e))?;
        }
        return Ok(());
    }

    let bytes =
        serde_json::to_vec_pretty(&current).map_err(|e| format!("serialize endpoint: {}", e))?;
    save_atomic_json(&path, &bytes)?;
    Ok(())
}

pub fn update_broker_endpoint_broker(
    app_base: &Path,
    socket: String,
    token_path: String,
    profile: Option<String>,
) -> Result<(), String> {
    update_endpoint(app_base, |mut ep| {
        ep.broker = Some(BrokerEndpointBroker {
            socket,
            token_path,
            profile,
            pid: Some(std::process::id()),
        });
        ep
    })
}

pub fn update_broker_endpoint_browser_native(
    app_base: &Path,
    config: BrokerEndpointBrowserNative,
) -> Result<(), String> {
    update_endpoint(app_base, |mut ep| {
        ep.browser_native = Some(config);
        ep
    })
}

pub fn update_broker_endpoint_browser_bridge(
    app_base: &Path,
    socket: String,
    token_path: String,
    pid: Option<u32>,
) -> Result<(), String> {
    update_endpoint(app_base, |mut ep| {
        ep.browser_bridge = Some(BrokerEndpointBrowserBridge {
            socket,
            token_path,
            pid,
        });
        ep
    })
}

pub fn update_broker_endpoint_browser_worker(
    app_base: &Path,
    socket: String,
    token_path: String,
    pid: Option<u32>,
) -> Result<(), String> {
    update_endpoint(app_base, |mut ep| {
        ep.browser_worker = Some(BrokerEndpointBrowserWorker {
            socket,
            token_path,
            pid,
        });
        ep
    })
}

pub fn clear_broker_endpoint_browser_bridge(
    app_base: &Path,
    socket: &str,
    pid: Option<u32>,
) -> Result<(), String> {
    clear_endpoint_if(app_base, |ep| {
        let Some(current) = ep.browser_bridge.as_ref() else {
            return false;
        };
        if current.socket != socket {
            return false;
        }
        if let Some(pid) = pid {
            if current.pid != Some(pid) {
                return false;
            }
        }
        ep.browser_bridge = None;
        true
    })
}

pub fn clear_broker_endpoint_browser_worker(
    app_base: &Path,
    socket: &str,
    pid: Option<u32>,
) -> Result<(), String> {
    clear_endpoint_if(app_base, |ep| {
        let Some(current) = ep.browser_worker.as_ref() else {
            return false;
        };
        if current.socket != socket {
            return false;
        }
        if let Some(pid) = pid {
            if current.pid != Some(pid) {
                return false;
            }
        }
        ep.browser_worker = None;
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), unique));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_marker(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create marker parent");
        }
        fs::write(path, "ok\n").expect("write marker");
    }

    #[test]
    fn clear_browser_bridge_preserves_other_endpoint_sections() {
        let temp = TempDirGuard::new("rzn-endpoint-preserve");
        update_broker_endpoint_browser_bridge(
            &temp.path,
            "/tmp/bridge.sock".to_string(),
            "/tmp/bridge.token".to_string(),
            Some(11),
        )
        .expect("write bridge");
        update_broker_endpoint_browser_worker(
            &temp.path,
            "/tmp/worker.sock".to_string(),
            "/tmp/worker.token".to_string(),
            Some(12),
        )
        .expect("write worker");

        clear_broker_endpoint_browser_bridge(&temp.path, "/tmp/bridge.sock", Some(11))
            .expect("clear bridge");

        let endpoint = read_broker_endpoint(&temp.path).expect("endpoint should remain");
        assert!(endpoint.browser_bridge.is_none());
        assert_eq!(
            endpoint
                .browser_worker
                .as_ref()
                .map(|worker| worker.socket.as_str()),
            Some("/tmp/worker.sock")
        );
    }

    #[test]
    fn clear_last_endpoint_section_removes_file() {
        let temp = TempDirGuard::new("rzn-endpoint-remove");
        update_broker_endpoint_browser_worker(
            &temp.path,
            "/tmp/worker.sock".to_string(),
            "/tmp/worker.token".to_string(),
            Some(21),
        )
        .expect("write worker");

        clear_broker_endpoint_browser_worker(&temp.path, "/tmp/worker.sock", Some(21))
            .expect("clear worker");

        assert!(!broker_endpoint_path(&temp.path).exists());
    }

    #[test]
    fn prune_stale_endpoint_removes_dead_sections_and_orphan_sockets() {
        let temp = TempDirGuard::new("rzn-endpoint-prune");
        let run_dir = temp.path.join("run");
        let secure_dir = temp.path.join("secure");
        let broker_socket = run_dir.join("broker.sock");
        let bridge_socket = run_dir.join("bridge.sock");
        let worker_socket = run_dir.join("worker.sock");
        let broker_token = secure_dir.join("broker.token");
        let bridge_token = secure_dir.join("bridge.token");
        let worker_token = secure_dir.join("worker.token");
        for path in [
            &broker_socket,
            &bridge_socket,
            &worker_socket,
            &broker_token,
            &bridge_token,
            &worker_token,
        ] {
            write_marker(path);
        }

        let endpoint = BrokerEndpointV1 {
            v: 1,
            broker: Some(BrokerEndpointBroker {
                socket: broker_socket.to_string_lossy().to_string(),
                token_path: broker_token.to_string_lossy().to_string(),
                profile: None,
                pid: Some(u32::MAX),
            }),
            browser_bridge: Some(BrokerEndpointBrowserBridge {
                socket: bridge_socket.to_string_lossy().to_string(),
                token_path: bridge_token.to_string_lossy().to_string(),
                pid: Some(u32::MAX),
            }),
            browser_worker: Some(BrokerEndpointBrowserWorker {
                socket: worker_socket.to_string_lossy().to_string(),
                token_path: worker_token.to_string_lossy().to_string(),
                pid: Some(u32::MAX),
            }),
            ..BrokerEndpointV1::default()
        };
        let bytes = serde_json::to_vec_pretty(&endpoint).expect("serialize endpoint");
        save_atomic_json(&broker_endpoint_path(&temp.path), &bytes).expect("write endpoint");

        let report = prune_stale_broker_endpoint(&temp.path).expect("prune endpoint");

        assert!(report.changed());
        assert!(report.removed_broker);
        assert!(report.removed_browser_bridge);
        assert!(report.removed_browser_worker);
        assert_eq!(report.removed_socket_paths.len(), 3);
        assert!(!broker_socket.exists());
        assert!(!bridge_socket.exists());
        assert!(!worker_socket.exists());
        assert!(!broker_endpoint_path(&temp.path).exists());
    }

    #[test]
    fn prune_stale_endpoint_preserves_live_existing_sections() {
        let temp = TempDirGuard::new("rzn-endpoint-prune-live");
        let socket = temp.path.join("run").join("worker.sock");
        let token = temp.path.join("secure").join("worker.token");
        write_marker(&socket);
        write_marker(&token);
        update_broker_endpoint_browser_worker(
            &temp.path,
            socket.to_string_lossy().to_string(),
            token.to_string_lossy().to_string(),
            Some(std::process::id()),
        )
        .expect("write worker");

        let report = prune_stale_broker_endpoint(&temp.path).expect("prune endpoint");

        assert!(!report.changed());
        let endpoint = read_broker_endpoint(&temp.path).expect("endpoint should remain");
        assert!(endpoint.browser_worker.is_some());
        assert!(socket.exists());
    }
}
