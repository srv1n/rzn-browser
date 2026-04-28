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
}
