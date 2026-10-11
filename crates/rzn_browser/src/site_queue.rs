//! One person, one site, one tab.
//!
//! Agents fan out CLI runs in parallel. Without a gate, eight `capterra search`
//! calls become eight tabs opening, loading and closing on capterra.com within
//! a second, which is exactly the burst a bot filter blocks. Every manifest run
//! goes through a [`SiteGate`] keyed by the workflow's `system`:
//!
//! * runs for the same site hold an exclusive file lock, so they queue and
//!   execute one at a time across processes; the next run starts as soon as the
//!   previous one ends (human pacing comes from the steps themselves);
//! * the tab of the previous run stays open and the next run navigates inside
//!   it, unless the workflow binds the user's active tab or manages tabs itself;
//! * a run that fails with `site_challenge` (the workflow saw a captcha or
//!   verification wall) pauses the site for [`CHALLENGE_COOLDOWN_MS`], so queued
//!   runs fail fast instead of loading the challenge page again and again.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CHALLENGE_COOLDOWN_MS: u64 = 15 * 60 * 1000;

/// Marker a workflow puts in its error message when the page is a bot challenge.
pub const CHALLENGE_MARKER: &str = "site_challenge";

/// Step kinds that open, switch or close tabs; such workflows own their tabs.
const TAB_STEPS: [&str; 3] = ["open_new_tab", "switch_to_tab", "close_current_tab"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SitePolicy {
    pub site: String,
    pub park_tab: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SiteState {
    #[serde(default)]
    tab_ref: Option<String>,
    /// Browser target the parked tab was opened under; reuse needs a match.
    #[serde(default)]
    browser_target: Option<Value>,
    #[serde(default)]
    blocked_until_ms: u64,
}

pub struct SiteGate {
    // Held for the whole run; the OS releases it if the process dies.
    _lock: File,
    state_path: PathBuf,
    state: SiteState,
    policy: SitePolicy,
}

/// Site policy for a workflow manifest; `None` for files without a usable `system`.
pub fn policy_for_workflow(path: &str) -> Option<SitePolicy> {
    let manifest: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    policy_for_manifest(&manifest)
}

fn policy_for_manifest(manifest: &Value) -> Option<SitePolicy> {
    let site = manifest
        .get("system")?
        .as_str()?
        .trim()
        .to_ascii_lowercase();
    let valid = !site.is_empty()
        && !site.starts_with('.')
        && site
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if !valid {
        return None;
    }
    let uses_active_tab = manifest
        .pointer("/runtime/requires_existing_session")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let manages_tabs = manifest
        .get("steps")
        .and_then(Value::as_array)
        .is_some_and(|steps| {
            steps.iter().any(|step| {
                step.pointer("/action/kind")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| TAB_STEPS.contains(&kind))
            })
        });
    Some(SitePolicy {
        site,
        park_tab: !uses_active_tab && !manages_tabs,
    })
}

impl SiteGate {
    pub async fn acquire(app_base: &Path, policy: SitePolicy) -> Result<Self> {
        let dir = app_base.join("site-queue");
        fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        let mut lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(format!("{}.lock", policy.site)))?;
        if lock.try_lock().is_err() {
            eprintln!(
                "[queue] another {} run is active; this run waits its turn",
                policy.site
            );
            lock = tokio::task::spawn_blocking(move || lock.lock().map(|_| lock)).await??;
        }
        let state_path = dir.join(format!("{}.json", policy.site));
        let state: SiteState = fs::read(&state_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();

        let now = now_ms();
        if state.blocked_until_ms > now {
            bail!(
                "{site} showed a bot challenge; runs are paused for {mins} more min. \
                 Solve the challenge in the {site} tab by hand, then wait or delete {path}",
                site = policy.site,
                mins = (state.blocked_until_ms - now).div_ceil(60_000),
                path = state_path.display(),
            );
        }
        Ok(Self {
            _lock: lock,
            state_path,
            state,
            policy,
        })
    }

    pub fn park_tab(&self) -> bool {
        self.policy.park_tab
    }

    /// The parked tab, if it was opened under the same browser target.
    pub fn parked_tab(&self, browser_target: Option<&Value>) -> Option<String> {
        let same_target = self.state.browser_target.as_ref() == browser_target;
        self.policy
            .park_tab
            .then(|| self.state.tab_ref.clone().filter(|_| same_target))
            .flatten()
    }

    pub fn forget_parked_tab(&mut self) {
        self.state.tab_ref = None;
    }

    /// Records the tab to reuse next time (and any challenge), then releases the lock.
    pub fn finish(
        mut self,
        tab_ref: Option<String>,
        browser_target: Option<Value>,
        challenged: bool,
    ) -> Result<()> {
        if self.policy.park_tab && tab_ref.is_some() {
            self.state.tab_ref = tab_ref;
            self.state.browser_target = browser_target;
        }
        if challenged {
            self.state.blocked_until_ms = now_ms() + CHALLENGE_COOLDOWN_MS;
            eprintln!(
                "[queue] {site} challenge detected; pausing {site} runs for {mins} min",
                site = self.policy.site,
                mins = CHALLENGE_COOLDOWN_MS / 60_000
            );
        }
        // Write then rename, so a killed process cannot leave a truncated file
        // that would silently drop the challenge pause.
        let tmp = self.state_path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec(&self.state)?)?;
        fs::rename(&tmp, &self.state_path)?;
        Ok(())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn policy_comes_from_the_manifest() {
        let plain = json!({"system": "g2", "steps": [{"action": {"kind": "navigate_to_url"}}]});
        assert_eq!(
            policy_for_manifest(&plain),
            Some(SitePolicy {
                site: "g2".into(),
                park_tab: true
            })
        );
        let active = json!({"system": "x", "runtime": {"requires_existing_session": true}});
        assert!(!policy_for_manifest(&active).unwrap().park_tab);
        let tabs = json!({"system": "airbnb", "steps": [{"action": {"kind": "open_new_tab"}}]});
        assert!(!policy_for_manifest(&tabs).unwrap().park_tab);
        assert_eq!(policy_for_manifest(&json!({"system": "../etc"})), None);
        assert_eq!(policy_for_manifest(&json!({"steps": []})), None);
    }

    #[tokio::test]
    async fn gate_parks_tab_and_blocks_after_challenge() {
        let dir = std::env::temp_dir().join(format!("rzn-site-queue-{}", std::process::id()));
        let policy = SitePolicy {
            site: "example".into(),
            park_tab: true,
        };

        let gate = SiteGate::acquire(&dir, policy.clone()).await.unwrap();
        assert_eq!(gate.parked_tab(None), None);
        let chrome = json!({"browser": "chrome"});
        gate.finish(
            Some("rzn://browser/b/tab/7".into()),
            Some(chrome.clone()),
            false,
        )
        .unwrap();

        let gate = SiteGate::acquire(&dir, policy.clone()).await.unwrap();
        assert_eq!(
            gate.parked_tab(Some(&chrome)).as_deref(),
            Some("rzn://browser/b/tab/7")
        );
        assert_eq!(gate.parked_tab(Some(&json!({"browser": "edge"}))), None);
        assert_eq!(gate.parked_tab(None), None);
        gate.finish(None, None, true).unwrap();

        let blocked = SiteGate::acquire(&dir, policy).await;
        assert!(blocked.err().unwrap().to_string().contains("bot challenge"));
        let _ = fs::remove_dir_all(dir);
    }
}
