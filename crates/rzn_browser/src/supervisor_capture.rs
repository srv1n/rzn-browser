//! Capture of pages the owner opens (source-network design 6.1, BFL-T-0025).
//!
//! For a site the owner switched on (`Settings::capture_sites`, default none), the extension
//! reports a page it finished loading in an ordinary tab (`capture.page`, never a workflow tab).
//! The supervisor reads that tab in place with the site's read workflow: the run binds the
//! owner's tab (`tab_ref`), keeps it open afterwards, and runs no navigation or scroll step, so
//! nothing happens on the page the owner did not do. The items it returns are queued and pushed in
//! batches through the backend push door (`POST /api/indexes/:id/sources/:key/items`) to the
//! pushed source named in `<app base>/capture.json`, which must be a source of the owner's own
//! private index: the door only takes the caller's own index, and capture sends nowhere else, so
//! a `restricted` item (a direct message, a private group) never leaves it (ruling 10).
//!
//! A capture read is a local run of the owner's device, never a fleet job, so it works in the
//! store build of the extension, which takes no jobs from us (ruling 22).
//!
//! Every page read and every batch sent is one line of `<app base>/capture/activity.jsonl`.
//!
//! The read workflow is the site's reader for an X post, a LinkedIn post or a Reddit thread
//! ([`reader_for`]), so the item has the key and shape that reader gives it in a job; another
//! page of those three sites is not read, and every other site is read with `web/page`. A
//! `truncated` item (comments the page did not load, text behind "more") is logged and not
//! queued: the push door refuses a batch that holds one.
//!
//! Stubbed this round: the push target and its bearer token are a hand-written file (no shared
//! login, design 8.4); an unsent batch lives in memory, so a supervisor restart loses at most one
//! minute of reads.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::time::Duration;
use uuid::Uuid;

use super::SupervisorState;
use crate::settings::SettingsStore;
use crate::workflow_runner::{
    execute_workflow, load_workflow_value_for_run, RunEventSink, RunOptions, SessionSpec,
    SnapshotMode,
};

/// A batch is sent when it holds this many items...
pub(super) const BATCH_MAX_ITEMS: usize = 25;
/// ...or when its first item has waited this long.
pub(super) const BATCH_MAX_AGE_MS: i64 = 60_000;
/// How often the supervisor looks for a batch that waited long enough.
const FLUSH_TICK: Duration = Duration::from_secs(5);
/// A page read again within this window (a reload) is not read twice.
const REREAD_AFTER_MS: i64 = 30_000;
/// The read workflow of a site without a reader of its own.
pub(super) const WEB_PAGE_WORKFLOW: &str = "web/page";
const ACTIVITY_FILE: &str = "activity.jsonl";
const READ_DEADLINE: Duration = Duration::from_secs(60);
/// Step kinds a read in place never runs, whatever they declare: the owner did not navigate,
/// scroll or close a popup, and the page is already loaded (no response left to capture).
const NOT_IN_PLACE: [&str; 7] = [
    "navigate_to_url",
    "open_new_tab",
    "infinite_scroll",
    "scroll_window_to",
    "dismiss_popups",
    "capture_responses",
    "wait_for_navigation",
];
/// Script steps; a read in place tells them so (`__rzn_params.in_place`) and gives them no
/// captured responses.
const SCRIPT_STEPS: [&str; 3] = [
    "execute_javascript",
    "eval_main_world",
    "eval_isolated_world",
];

/// Where batches go: a pushed source of the owner's own index.
#[derive(Debug, Clone)]
pub(super) struct CaptureTarget {
    pub server_url: String,
    pub index_id: String,
    pub source_key: String,
    /// The signed-in owner's access token.
    pub token: String,
}

/// Where the next batch goes, asked at each send; a seam for tests.
#[async_trait]
pub(super) trait Targets: Send + Sync {
    async fn target(&self) -> Result<CaptureTarget>;
}

/// The signed-in owner's target (BFL-T-0012, replacing the hand-written `capture.json`): the
/// server and access token of the RZN account this browser signed in with
/// (`fleet_cli::signin`), the index and pushed source from Settings (`capture_index`,
/// `capture_source`). A browser revoked in the app sends nothing until it signs in again.
pub(super) struct SignedIn(pub SettingsStore);

#[async_trait]
impl Targets for SignedIn {
    async fn target(&self) -> Result<CaptureTarget> {
        let settings = self.0.get();
        if settings.capture_index.is_empty() || settings.capture_source.is_empty() {
            bail!("choose the index and pushed source for captured pages in Settings");
        }
        if crate::supervisor_fleet::fleet_status_rpc()["state"] == "stopped_revoked" {
            bail!("this browser was signed out from the app; sign in again on the Account page");
        }
        let (server_url, token) = crate::fleet_cli::signin::access_token().await?;
        Ok(CaptureTarget {
            server_url,
            index_id: settings.capture_index,
            source_key: settings.capture_source,
            token,
        })
    }
}

/// Items waiting to be pushed; one key at most once (a later read of a page replaces the earlier).
#[derive(Debug, Default)]
pub(super) struct Batcher {
    items: Vec<Value>,
    first_at: Option<i64>,
}

impl Batcher {
    /// Add one item; a full batch comes back to be sent.
    pub(super) fn add(&mut self, item: Value, now: i64) -> Option<Vec<Value>> {
        let key = item.get("key").cloned();
        self.items.retain(|held| held.get("key") != key.as_ref());
        self.items.push(item);
        self.first_at.get_or_insert(now);
        (self.items.len() >= BATCH_MAX_ITEMS).then(|| self.take())
    }

    /// The batch, when its first item has waited [`BATCH_MAX_AGE_MS`].
    pub(super) fn due(&mut self, now: i64) -> Option<Vec<Value>> {
        let first = self.first_at?;
        (now - first >= BATCH_MAX_AGE_MS).then(|| self.take())
    }

    /// Everything waiting, now; `None` when empty.
    pub(super) fn take_any(&mut self) -> Option<Vec<Value>> {
        (!self.items.is_empty()).then(|| self.take())
    }

    /// Put a batch that could not be sent back in front; it is tried again one batch age after
    /// `now` (a door that is down is asked once a minute).
    fn put_back(&mut self, batch: Vec<Value>, now: i64) {
        let mut items = batch;
        let newer = std::mem::take(&mut self.items);
        items.retain(|held| !newer.iter().any(|n| n.get("key") == held.get("key")));
        items.extend(newer);
        self.items = items;
        self.first_at = Some(now);
    }

    fn take(&mut self) -> Vec<Value> {
        self.first_at = None;
        std::mem::take(&mut self.items)
    }
}

/// What the push door answered.
pub(super) enum Pushed {
    Accepted(Value),
    /// The door refused the batch for its content (a 4xx other than 401, 408, 429): resending
    /// it cannot help, so it is dropped and logged.
    Refused(String),
}

/// The push door, a seam for tests.
#[async_trait]
pub(super) trait Door: Send + Sync {
    async fn push(&self, target: &CaptureTarget, items: &[Value]) -> Result<Pushed>;
}

/// The backend's push door over HTTP.
pub(super) struct HttpDoor(reqwest::Client);

#[async_trait]
impl Door for HttpDoor {
    async fn push(&self, target: &CaptureTarget, items: &[Value]) -> Result<Pushed> {
        let url = format!(
            "{}/api/indexes/{}/sources/{}/items",
            target.server_url.trim_end_matches('/'),
            target.index_id,
            target.source_key
        );
        let response = self
            .0
            .post(&url)
            .bearer_auth(&target.token)
            .json(&json!({"type": "rzn.items/v2", "items": items}))
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .with_context(|| format!("push to {url}"))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status.is_success() {
            return Ok(Pushed::Accepted(
                serde_json::from_str(&body).unwrap_or(Value::String(body)),
            ));
        }
        let retry = matches!(status.as_u16(), 401 | 408 | 429) || status.is_server_error();
        let detail = format!("{status}: {}", body.chars().take(2000).collect::<String>());
        if retry {
            Err(anyhow!("push door answered {detail}"))
        } else {
            Ok(Pushed::Refused(detail))
        }
    }
}

/// The supervisor's capture state.
pub(super) struct Capture {
    dir: PathBuf,
    targets: Arc<dyn Targets>,
    batch: StdMutex<Batcher>,
    /// When each page was last read, to skip a reload.
    recent: StdMutex<HashMap<String, i64>>,
    door: Arc<dyn Door>,
    ticker: OnceLock<()>,
}

impl Capture {
    pub(super) fn open(app_base: &Path, settings: SettingsStore) -> Self {
        Self::with_door(
            app_base,
            Arc::new(HttpDoor(reqwest::Client::new())),
            Arc::new(SignedIn(settings)),
        )
    }

    pub(super) fn with_door(
        app_base: &Path,
        door: Arc<dyn Door>,
        targets: Arc<dyn Targets>,
    ) -> Self {
        Capture {
            dir: app_base.join("capture"),
            targets,
            batch: StdMutex::new(Batcher::default()),
            recent: StdMutex::new(HashMap::new()),
            door,
            ticker: OnceLock::new(),
        }
    }

    /// Note a read of `url`; `false` when it was read within [`REREAD_AFTER_MS`].
    fn fresh(&self, url: &str, now: i64) -> bool {
        let mut recent = lock(&self.recent);
        recent.retain(|_, at| now - *at < REREAD_AFTER_MS);
        if recent.contains_key(url) {
            return false;
        }
        recent.insert(url.to_string(), now);
        true
    }

    /// Queue the items of one read; send the batch when it is full.
    pub(super) async fn offer(&self, items: Vec<Value>, now: i64) {
        let mut full = Vec::new();
        {
            let mut batch = lock(&self.batch);
            for item in items {
                if let Some(ready) = batch.add(item, now) {
                    full.push(ready);
                }
            }
        }
        for ready in full {
            self.send(ready, now).await;
        }
    }

    /// Send the batch if it waited long enough (`force`: whatever is waiting).
    pub(super) async fn tick(&self, now: i64, force: bool) -> usize {
        let ready = {
            let mut batch = lock(&self.batch);
            if force {
                batch.take_any()
            } else {
                batch.due(now)
            }
        };
        match ready {
            Some(ready) => {
                let n = ready.len();
                self.send(ready, now).await;
                n
            }
            None => 0,
        }
    }

    async fn send(&self, batch: Vec<Value>, now: i64) {
        let keys: Vec<Value> = batch.iter().filter_map(|i| i.get("key").cloned()).collect();
        let target = match self.targets.target().await {
            Ok(target) => target,
            Err(error) => {
                self.log(json!({"event": "batch_held", "items": keys.len(), "error": format!("{error:#}")}));
                lock(&self.batch).put_back(batch, now);
                return;
            }
        };
        let place = json!({"index_id": target.index_id, "source_key": target.source_key});
        match self.door.push(&target, &batch).await {
            Ok(Pushed::Accepted(answer)) => self.log(json!({
                "event": "batch_sent", "to": place, "items": keys.len(), "keys": keys,
                "job_id": answer.get("job_id"),
            })),
            Ok(Pushed::Refused(detail)) => self.log(json!({
                "event": "batch_refused", "to": place, "items": keys.len(), "keys": keys,
                "error": detail,
            })),
            Err(error) => {
                self.log(json!({
                    "event": "batch_held", "to": place, "items": keys.len(),
                    "error": format!("{error:#}"),
                }));
                lock(&self.batch).put_back(batch, now);
            }
        }
    }

    /// One activity line: what was read or sent, when. Never the token.
    fn log(&self, mut entry: Value) {
        entry["at_ms"] = json!(now_ms());
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(&self.dir)?;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.dir.join(ACTIVITY_FILE))?;
            writeln!(file, "{entry}")
        };
        if let Err(error) = write() {
            log::warn!("capture activity log write failed: {error}");
        }
    }
}

/// `capture.page {url, tab_ref}`: read a page the owner opened, when its site is switched on.
pub(super) async fn page(state: &Arc<SupervisorState>, params: Value) -> Result<Value> {
    let url = params
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("capture.page needs url"))?
        .to_string();
    let tab_ref = params
        .get("tab_ref")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("capture.page needs tab_ref"))?
        .to_string();
    let Some(site) = site_of(&url) else {
        return Ok(json!({"read": false, "reason": "not_a_web_page"}));
    };
    if !switched_on(&state.settings.get().capture_sites, &site) {
        return Ok(json!({"read": false, "reason": "site_off"}));
    }
    if state.control.paused() {
        return Ok(json!({"read": false, "reason": "automation_paused"}));
    }
    let Some((workflow_id, param)) = reader_for(&url) else {
        return Ok(json!({"read": false, "reason": "not_an_item_page"}));
    };
    if !state.capture.fresh(&url, now_ms()) {
        return Ok(json!({"read": false, "reason": "read_recently"}));
    }
    ensure_ticker(state);
    let owned = state.clone();
    tokio::spawn(async move {
        let started = now_ms();
        let entry = match read_in_place(&owned, workflow_id, param, &url, &tab_ref).await {
            Ok(items) => {
                let (whole, held) = whole_items(items);
                let keys: Vec<Value> = whole.iter().filter_map(|i| i.get("key").cloned()).collect();
                let mut entry = json!({"event": "page_read", "url": url, "workflow_id": workflow_id,
                    "items": keys.len(), "keys": keys, "ms": now_ms() - started});
                if !held.is_empty() {
                    entry["truncated"] = Value::Array(held);
                }
                owned.capture.log(entry);
                owned.capture.offer(whole, now_ms()).await;
                return;
            }
            Err(error) => json!({"event": "page_read", "url": url, "workflow_id": workflow_id,
                "items": 0, "error": format!("{error:#}"), "ms": now_ms() - started}),
        };
        owned.capture.log(entry);
    });
    Ok(json!({"read": true, "workflow_id": workflow_id, "site": site}))
}

/// `capture.flush`: send whatever is waiting now.
pub(super) async fn flush(state: &Arc<SupervisorState>) -> Result<Value> {
    let sent = state.capture.tick(now_ms(), true).await;
    Ok(json!({"items": sent}))
}

fn ensure_ticker(state: &Arc<SupervisorState>) {
    if state.capture.ticker.set(()).is_err() {
        return;
    }
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(FLUSH_TICK);
        loop {
            ticker.tick().await;
            let Some(state) = weak.upgrade() else { return };
            state.capture.tick(now_ms(), false).await;
        }
    });
}

/// The read workflow of a page and the parameter its URL goes in: the site's reader for an X
/// post, a LinkedIn post or a Reddit thread; `None` for another page of those sites (a feed,
/// a profile, a search: no reader returns it as one item); `web/page` for every other site.
pub(super) fn reader_for(url: &str) -> Option<(&'static str, &'static str)> {
    let parsed = url::Url::parse(url).ok()?;
    let site = site_of(url)?;
    let segments: Vec<&str> = parsed
        .path_segments()
        .map(|s| s.filter(|seg| !seg.is_empty()).collect())
        .unwrap_or_default();
    let on = |domain: &str| site == domain || site.ends_with(&format!(".{domain}"));
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if on("x.com") || on("twitter.com") {
        // /<handle>/status/<id>, /<handle>/article/<id>, /i/web/status/<id>
        let post = segments
            .windows(2)
            .any(|w| matches!(w[0], "status" | "article") && digits(w[1]));
        return post.then_some(("x/open", "url"));
    }
    if on("linkedin.com") {
        let path = parsed.path();
        let post = ["urn:li:activity:", "urn:li:share:", "urn:li:ugcPost:"]
            .iter()
            .any(|urn| path.contains(urn))
            || (segments.first() == Some(&"posts") && path.contains("-activity-"));
        return post.then_some(("linkedin/open", "url"));
    }
    if on("reddit.com") {
        let thread = segments
            .windows(2)
            .any(|w| w[0] == "comments" && w[1].bytes().all(|b| b.is_ascii_alphanumeric()));
        return thread.then_some(("reddit/thread", "post"));
    }
    Some((WEB_PAGE_WORKFLOW, "url"))
}

/// The items to queue, and the keys and reasons of those held back as `truncated`.
pub(super) fn whole_items(items: Vec<Value>) -> (Vec<Value>, Vec<Value>) {
    let mut held = Vec::new();
    let whole = items
        .into_iter()
        .filter(
            |item| match item.get("truncated").filter(|t| !t.is_null()) {
                Some(truncated) => {
                    held.push(json!({"key": item.get("key"), "reason": truncated.get("reason")}));
                    false
                }
                None => true,
            },
        )
        .collect();
    (whole, held)
}

/// Run `workflow_id` on the owner's tab with no navigation or scrolling, the page's URL as
/// `param`; its items.
async fn read_in_place(
    state: &Arc<SupervisorState>,
    workflow_id: &str,
    param: &str,
    url: &str,
    tab_ref: &str,
) -> Result<Vec<Value>> {
    let path = state.resolve_local_workflow(workflow_id)?;
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path)?)
        .with_context(|| format!("read workflow {}", path.display()))?;
    in_place(&mut manifest)?;
    let params = HashMap::from([(param.to_string(), url.to_string())]);
    let workflow = load_workflow_value_for_run(manifest, &params)?;
    let transport = crate::supervisor_fleet::InProcessTransport {
        state: state.clone(),
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let opts = RunOptions {
        run_id: format!("capture-{}", Uuid::new_v4()),
        workflow_hash: None,
        params,
        deadline: Some(READ_DEADLINE),
        session: SessionSpec {
            tab_ref: Some(tab_ref.to_string()),
            retain_tab_on_close: true,
            origin: Some("capture".to_string()),
            ..SessionSpec::default()
        },
        snapshot_mode: SnapshotMode::None,
        workflow_path: path.to_string_lossy().into_owned(),
        artifacts_dir: None,
    };
    let result = execute_workflow(&transport, &QuietSink, workflow, opts).await;
    if result.status != rzn_contracts::workflow::RunStatus::Succeeded {
        let error = result
            .error
            .map(|e| format!("{}: {}", e.code, e.message))
            .unwrap_or_else(|| format!("{:?}", result.status));
        return Err(anyhow!("read failed: {error}"));
    }
    items_of(result.output)
}

struct QuietSink;
impl RunEventSink for QuietSink {}

/// Keep only the steps a read in place runs: those that declare nothing but `read_only` and
/// are not of a kind in [`NOT_IN_PLACE`]. Script steps are told they run in place and get no
/// captured responses. A workflow that writes to the site is refused.
pub(super) fn in_place(manifest: &mut Value) -> Result<()> {
    let writes = manifest
        .get("side_effects")
        .and_then(Value::as_array)
        .is_some_and(|effects| {
            effects
                .iter()
                .any(|e| e.get("class").and_then(Value::as_str) == Some("external_write"))
        });
    if writes {
        return Err(anyhow!("a capture read never runs a workflow that writes"));
    }
    let steps = manifest
        .get_mut("steps")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("workflow has no steps"))?;
    steps.retain(|step| {
        let kind = step
            .pointer("/action/kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let read_only = step
            .pointer("/action/side_effects")
            .and_then(Value::as_array)
            .is_some_and(|effects| {
                !effects.is_empty() && effects.iter().all(|e| e.as_str() == Some("read_only"))
            });
        read_only && !NOT_IN_PLACE.contains(&kind) && !kind.starts_with("scroll")
    });
    if steps.is_empty() {
        return Err(anyhow!("nothing of the workflow runs in place"));
    }
    for step in steps.iter_mut() {
        let kind = step.pointer("/action/kind").and_then(Value::as_str);
        if !kind.is_some_and(|k| SCRIPT_STEPS.contains(&k)) {
            continue;
        }
        if let Some(inputs) = step
            .pointer_mut("/action/inputs")
            .and_then(Value::as_object_mut)
        {
            inputs.remove("captured_responses");
            let params = inputs.entry("params").or_insert_with(|| json!({}));
            if let Some(params) = params.as_object_mut() {
                params.insert("in_place".into(), Value::Bool(true));
            }
        }
    }
    Ok(())
}

/// The items a read returned: one `rzn.item/v2`, or a page of them.
pub(super) fn items_of(output: Option<Value>) -> Result<Vec<Value>> {
    let output = output.ok_or_else(|| anyhow!("the read returned nothing"))?;
    match output.get("type").and_then(Value::as_str) {
        Some("rzn.item/v2") => Ok(vec![output]),
        Some("rzn.items/v2") => Ok(output
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()),
        other => Err(anyhow!("the read returned {other:?}, not rzn.item/v2")),
    }
}

/// The site of a web page: its host, lowercase, without `www.`; `None` for anything else.
pub(super) fn site_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    Some(host.strip_prefix("www.").unwrap_or(&host).to_string())
}

/// A site is on when it, or a domain it belongs to, is in the owner's list.
pub(super) fn switched_on(sites: &[String], site: &str) -> bool {
    sites.iter().any(|on| {
        let on = on.trim().to_ascii_lowercase();
        let on = on.strip_prefix("www.").unwrap_or(&on);
        !on.is_empty() && (site == on || site.ends_with(&format!(".{on}")))
    })
}

fn lock<T>(m: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
#[path = "supervisor_capture_tests.rs"]
mod tests;
