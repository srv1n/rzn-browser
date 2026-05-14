use std::path::PathBuf;

use rzn_core::dsl::Workflow;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

#[test]
fn x_workflows_parse_as_v1_workflow() {
    let workflows = [
        "workflows/x/x_home_timeline_digest.json",
        "workflows/x/x_open.json",
        "workflows/x/x_like_post.json",
        "workflows/x/x_reply_post.json",
        "workflows/x/x_create_post.json",
        "workflows/x/x_open_inbox.json",
        "workflows/x/x_open_dm_thread.json",
        "workflows/x/x_send_dm.json",
        "workflows/x/x_reply_dm_thread.json",
        "workflows/x/x_search_posts.json",
        "workflows/x/x_profile_posts.json",
    ];

    for rel in workflows {
        let path = repo_root().join(rel);
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
        let wf: Workflow = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse {} as Workflow: {}", path.display(), e));

        assert!(
            !wf.browser_automation.sequences.is_empty(),
            "{} has no sequences",
            rel
        );
        for seq in &wf.browser_automation.sequences {
            assert!(
                !seq.steps.is_empty(),
                "{} sequence '{}' has no steps",
                rel,
                seq.name
            );
        }
    }
}
