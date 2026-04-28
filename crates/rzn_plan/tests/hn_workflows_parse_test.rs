use std::path::PathBuf;

use rzn_core::dsl::Workflow;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

#[test]
fn hn_workflows_parse_as_v1_workflow() {
    let workflows = [
        "workflows/hn/hn-submit-link-post.json",
        "workflows/hn/hn-submit-comment.json",
        "workflows/hn/hn-submit-reply.json",
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
