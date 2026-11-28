//! Capture of opened pages (BFL-T-0025): the switch, the in-place read and the batches.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// A door that counts its calls and keeps every batch; `fail` makes the next call fail.
#[derive(Default)]
struct CountingDoor {
    calls: AtomicUsize,
    batches: StdMutex<Vec<Vec<Value>>>,
    fail: AtomicBool,
}

#[async_trait]
impl Door for CountingDoor {
    async fn push(&self, _target: &CaptureTarget, items: &[Value]) -> Result<Pushed> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(anyhow!("door down"));
        }
        lock(&self.batches).push(items.to_vec());
        Ok(Pushed::Accepted(json!({"job_id": "j1"})))
    }
}

/// A fixed target; `None` is a browser not signed in or with no index chosen.
struct Fixed(Option<CaptureTarget>);

#[async_trait]
impl Targets for Fixed {
    async fn target(&self) -> Result<CaptureTarget> {
        self.0
            .clone()
            .ok_or_else(|| anyhow!("not signed in to RZN"))
    }
}

fn signed_in() -> Arc<dyn Targets> {
    Arc::new(Fixed(Some(CaptureTarget {
        server_url: "http://127.0.0.1:1".into(),
        index_id: "i".into(),
        source_key: "main".into(),
        token: "t".into(),
    })))
}

fn base() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rzn-capture-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn page_item(n: usize) -> Value {
    json!({"type": "rzn.item/v2", "key": format!("https://example.com/p{n}"), "kind": "page"})
}

fn activity(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join("capture").join(ACTIVITY_FILE))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("activity line"))
        .collect()
}

#[test]
fn a1_a_site_is_read_only_when_switched_on() {
    let site = site_of("https://www.VoiceBrainDump.com/blog/post?x=1").expect("site");
    assert_eq!(site, "voicebraindump.com");
    assert!(!switched_on(&[], &site), "default: every site off");
    assert!(!switched_on(&["lambda.ai".into()], &site));
    assert!(switched_on(&["voicebraindump.com".into()], &site));
    assert!(switched_on(&["WWW.voicebraindump.com".into()], &site));
    assert!(switched_on(
        &["voicebraindump.com".into()],
        "blog.voicebraindump.com"
    ));
    assert!(
        !switched_on(&["dump.com".into()], &site),
        "a suffix is not a domain"
    );
    assert_eq!(site_of("chrome://extensions"), None);
    assert_eq!(site_of("file:///tmp/a.html"), None);
}

#[tokio::test]
async fn a2_thirty_pages_make_at_most_two_push_calls() {
    let dir = base();
    let door = Arc::new(CountingDoor::default());
    let capture = Capture::with_door(&dir, door.clone(), signed_in());
    let start = 1_700_000_000_000;
    // The owner opens 30 pages, one every two seconds, the timer ticking every 5 s.
    for n in 0..30 {
        let now = start + n as i64 * 2_000;
        capture.offer(vec![page_item(n)], now).await;
        if n % 3 == 0 {
            capture.tick(now, false).await;
        }
    }
    // The timer runs on until the last batch is due.
    let mut now = start + 60_000;
    while now <= start + 240_000 {
        capture.tick(now, false).await;
        now += 5_000;
    }
    let calls = door.calls.load(Ordering::SeqCst);
    assert!(calls <= 2, "{calls} push calls for 30 pages");
    let sent: usize = lock(&door.batches).iter().map(Vec::len).sum();
    assert_eq!(sent, 30, "every page sent once");
    let batches: Vec<_> = activity(&dir)
        .into_iter()
        .filter(|e| e["event"] == "batch_sent")
        .collect();
    assert_eq!(batches.len(), calls, "one activity line per batch sent");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_failed_push_is_held_and_sent_with_the_next_batch() {
    let dir = base();
    let door = Arc::new(CountingDoor::default());
    let capture = Capture::with_door(&dir, door.clone(), signed_in());
    door.fail.store(true, Ordering::SeqCst);
    capture.offer(vec![page_item(1)], 0).await;
    assert_eq!(capture.tick(BATCH_MAX_AGE_MS, false).await, 1);
    assert!(
        lock(&door.batches).is_empty(),
        "the failed batch was not taken"
    );
    // The same page read again replaces its held copy: one key, once.
    capture
        .offer(vec![page_item(1), page_item(2)], BATCH_MAX_AGE_MS + 1)
        .await;
    assert_eq!(
        capture.tick(BATCH_MAX_AGE_MS + 5_000, false).await,
        0,
        "retried a minute later"
    );
    assert_eq!(capture.tick(2 * BATCH_MAX_AGE_MS, false).await, 2);
    let batches = lock(&door.batches).clone();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].len(), 2);
    assert!(activity(&dir).iter().any(|e| e["event"] == "batch_held"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn no_target_holds_the_batch() {
    let dir = base();
    let door = Arc::new(CountingDoor::default());
    let capture = Capture::with_door(&dir, door.clone(), Arc::new(Fixed(None)));
    capture.offer(vec![page_item(1)], 0).await;
    capture.tick(0, true).await;
    assert_eq!(door.calls.load(Ordering::SeqCst), 0);
    assert_eq!(lock(&capture.batch).items.len(), 1, "still waiting");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_reload_is_not_read_twice() {
    let capture = Capture::with_door(&base(), Arc::new(CountingDoor::default()), signed_in());
    assert!(capture.fresh("https://example.com/a", 0));
    assert!(!capture.fresh("https://example.com/a", REREAD_AFTER_MS - 1));
    assert!(capture.fresh("https://example.com/a", REREAD_AFTER_MS));
}

#[test]
fn in_place_runs_only_read_only_steps_and_never_a_write() {
    let ro = json!(["read_only"]);
    let mut manifest = json!({"side_effects": [{"class": "read_only"}], "steps": [
        {"id": "a", "action": {"kind": "navigate_to_url", "side_effects": ["browser_state"]}},
        {"id": "b", "action": {"kind": "scroll_element_into_view", "side_effects": ro}},
        {"id": "c", "action": {"kind": "infinite_scroll", "side_effects": ro}},
        {"id": "d", "action": {"kind": "dismiss_popups", "side_effects": ro}},
        {"id": "e", "action": {"kind": "execute_javascript", "side_effects": ["browser_state"],
            "inputs": {"script": "location.assign('/x')"}}},
        {"id": "f", "action": {"kind": "wait_for_navigation", "side_effects": ro}},
        {"id": "g", "action": {"kind": "wait_for_element", "side_effects": ro}},
        {"id": "h", "action": {"kind": "execute_javascript", "side_effects": ro,
            "inputs": {"script": "return 1", "captured_responses": true}}},
        {"id": "i", "action": {"kind": "execute_javascript"}},
    ]});
    in_place(&mut manifest).expect("in place");
    let kept: Vec<_> = manifest["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .map(|s| s["id"].clone())
        .collect();
    assert_eq!(kept, vec![json!("g"), json!("h")]);
    let script = &manifest["steps"][1]["action"]["inputs"];
    assert_eq!(script["params"]["in_place"], true, "the script knows");
    assert!(script.get("captured_responses").is_none(), "{script}");
    let mut writes = json!({"side_effects": [{"class": "external_write"}], "steps": [{"action": {"kind": "click"}}]});
    assert!(in_place(&mut writes).is_err());
    let mut only_navigation = json!({"steps": [{"action": {"kind": "navigate_to_url"}}]});
    assert!(in_place(&mut only_navigation).is_err());
}

#[test]
fn a_post_or_thread_is_read_by_its_site_reader() {
    for (url, reader) in [
        ("https://x.com/jack/status/20", Some(("x/open", "url"))),
        (
            "https://twitter.com/jack/status/20?s=20",
            Some(("x/open", "url")),
        ),
        ("https://x.com/i/web/status/20", Some(("x/open", "url"))),
        ("https://x.com/a/article/1890", Some(("x/open", "url"))),
        ("https://x.com/home", None),
        ("https://x.com/jack", None),
        (
            "https://www.linkedin.com/feed/update/urn:li:activity:7400000000000000000/",
            Some(("linkedin/open", "url")),
        ),
        (
            "https://www.linkedin.com/posts/ada_notes-activity-7400000000000000000-AbCd",
            Some(("linkedin/open", "url")),
        ),
        ("https://www.linkedin.com/feed/", None),
        ("https://www.linkedin.com/in/ada/", None),
        (
            "https://www.reddit.com/r/rust/comments/1y2ifey/a_title/",
            Some(("reddit/thread", "post")),
        ),
        (
            "https://old.reddit.com/r/rust/comments/1y2ifey/",
            Some(("reddit/thread", "post")),
        ),
        (
            "https://www.reddit.com/comments/1y2ifey",
            Some(("reddit/thread", "post")),
        ),
        ("https://www.reddit.com/r/rust/", None),
        (
            "https://voicebraindump.com/offline",
            Some((WEB_PAGE_WORKFLOW, "url")),
        ),
        (
            "https://notx.com/a/status/1",
            Some((WEB_PAGE_WORKFLOW, "url")),
        ),
    ] {
        assert_eq!(reader_for(url), reader, "{url}");
    }
}

#[test]
fn a_truncated_item_is_held_back() {
    let whole = json!({"type": "rzn.item/v2", "key": "reddit:thread:a"});
    let cut = json!({"type": "rzn.item/v2", "key": "reddit:thread:b",
        "truncated": {"reason": "more_comments_not_fetched", "returned_blocks": 3}});
    let (queued, held) = whole_items(vec![whole.clone(), cut]);
    assert_eq!(queued, vec![whole]);
    assert_eq!(
        held,
        vec![json!({"key": "reddit:thread:b", "reason": "more_comments_not_fetched"})]
    );
}

#[test]
fn the_site_readers_are_valid_in_place() {
    for (file, param, url, left) in [
        (
            "x/x_open.json",
            "url",
            "https://x.com/jack/status/20",
            vec!["s3", "s4", "s5"],
        ),
        (
            "linkedin/linkedin_open.json",
            "url",
            "https://www.linkedin.com/feed/update/urn:li:activity:7400000000000000000/",
            vec!["s3", "s4"],
        ),
        (
            "reddit/reddit-thread.json",
            "post",
            "https://www.reddit.com/r/rust/comments/1y2ifey/",
            vec!["s4", "s5"],
        ),
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../workflows")
            .join(file);
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(&path).expect("manifest")).expect("json");
        in_place(&mut manifest).expect("in place");
        let kept: Vec<_> = manifest["steps"]
            .as_array()
            .expect("steps")
            .iter()
            .filter_map(|s| s["id"].as_str())
            .collect();
        assert_eq!(kept, left, "{file}");
        let params = HashMap::from([(param.to_string(), url.to_string())]);
        load_workflow_value_for_run(manifest, &params).expect("valid manifest");
    }
}

#[test]
fn a_read_returns_one_item_or_a_page() {
    assert_eq!(items_of(Some(page_item(1))).expect("item").len(), 1);
    let page = json!({"type": "rzn.items/v2", "items": [page_item(1), page_item(2)]});
    assert_eq!(items_of(Some(page)).expect("page").len(), 2);
    assert!(items_of(Some(json!({"text": "x"}))).is_err());
    assert!(items_of(None).is_err());
}

#[test]
fn the_web_page_workflow_is_valid_and_runs_whole_in_place() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../workflows/web/web_page.json");
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&path).expect("web/page manifest")).expect("json");
    assert_eq!(manifest["id"], WEB_PAGE_WORKFLOW);
    let steps = manifest["steps"].as_array().map(Vec::len);
    in_place(&mut manifest).expect("in place");
    assert_eq!(
        manifest["steps"].as_array().map(Vec::len),
        steps,
        "nothing dropped"
    );
    let params = HashMap::from([("url".to_string(), "https://example.com/a".to_string())]);
    load_workflow_value_for_run(manifest, &params).expect("valid manifest");
}
