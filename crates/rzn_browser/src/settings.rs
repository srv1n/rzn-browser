use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub run_retention_count: usize,
    pub run_retention_days: i64,
    pub notifications_enabled: bool,
    pub notify_on: String,
    pub fleet_keep_window_on_failure: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            run_retention_count: 500,
            run_retention_days: 30,
            notifications_enabled: true,
            notify_on: "all".into(),
            fleet_keep_window_on_failure: false,
        }
    }
}
#[derive(Clone)]
pub struct SettingsStore {
    path: PathBuf,
    value: Arc<Mutex<Settings>>,
}
impl SettingsStore {
    pub fn open(base: PathBuf) -> Self {
        let path = base.join("settings.json");
        let value = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %err,
                        "settings parse failed; using defaults"
                    );
                    Settings::default()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "settings read failed; using defaults"
                );
                Settings::default()
            }
        };
        Self {
            path,
            value: Arc::new(Mutex::new(value)),
        }
    }
    pub fn get(&self) -> Settings {
        self.value.lock().unwrap().clone()
    }
    pub fn patch(&self, patch: Value) -> Result<Settings> {
        let o = patch
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("patch must be an object"))?;
        for k in o.keys() {
            if !matches!(
                k.as_str(),
                "run_retention_count"
                    | "run_retention_days"
                    | "notifications_enabled"
                    | "notify_on"
                    | "fleet_keep_window_on_failure"
            ) {
                bail!("unknown setting: {k}")
            }
        }
        let mut next = serde_json::to_value(self.get())?;
        for (k, v) in o {
            next[k] = v.clone()
        }
        let next: Settings = serde_json::from_value(next)?;
        if !(10..=10000).contains(&next.run_retention_count) {
            bail!("run_retention_count must be 10..10000")
        }
        if !(1..=365).contains(&next.run_retention_days) {
            bail!("run_retention_days must be 1..365")
        }
        if !matches!(next.notify_on.as_str(), "all" | "failures_only") {
            bail!("notify_on must be all or failures_only")
        }
        if let Some(p) = self.path.parent() {
            fs::create_dir_all(p)?
        }
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp =
            self.path
                .with_file_name(format!(".settings.json.{}.{}.tmp", std::process::id(), seq));
        fs::write(&tmp, serde_json::to_vec_pretty(&next)?)?;
        if let Err(err) = fs::rename(&tmp, &self.path) {
            let _ = fs::remove_file(&tmp);
            return Err(err.into());
        }
        *self.value.lock().unwrap() = next.clone();
        Ok(next)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_and_persists() {
        let d = std::env::temp_dir().join(format!("rzn-settings-{}", std::process::id()));
        let s = SettingsStore::open(d.clone());
        assert!(s
            .patch(serde_json::json!({"run_retention_count":9}))
            .is_err());
        s.patch(serde_json::json!({"notifications_enabled":false}))
            .unwrap();
        assert!(!SettingsStore::open(d).get().notifications_enabled);
    }
    #[test]
    fn fleet_run_notice_settings_gate() {
        let mut settings = Settings::default();
        assert!(crate::supervisor::fleet_notice_enabled(
            &settings, "started"
        ));
        settings.notify_on = "failures_only".into();
        assert!(!crate::supervisor::fleet_notice_enabled(
            &settings,
            "succeeded"
        ));
        assert!(crate::supervisor::fleet_notice_enabled(&settings, "failed"));
        settings.notifications_enabled = false;
        assert!(!crate::supervisor::fleet_notice_enabled(
            &settings, "failed"
        ));
        assert_eq!(
            crate::supervisor::fleet_run_notice_payload("j1", "wf", "failed", Some("timeout")),
            serde_json::json!({"job_id":"j1","workflow_id":"wf","phase":"failed","error_class":"timeout"})
        );
    }
}
