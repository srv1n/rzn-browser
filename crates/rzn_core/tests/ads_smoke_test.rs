//! ADI-T-0004: focused tests for the ads smoke-test lane. No live site.

use std::path::PathBuf;

use rzn_core::ads_smoke::smoke_check;
use serde_json::{json, Value};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn load(rel: &str) -> Value {
    let path = repo_root().join(rel);
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {} as JSON: {}", path.display(), e))
}

/// A1: a healthy pack manifest passes (would exit 0).
#[test]
fn healthy_fixtures_pass() {
    for rel in [
        "workflows/fixtures/ads/google_ads_transparency.manifest.json",
        "workflows/fixtures/ads/meta_ad_library.manifest.json",
    ] {
        let report = smoke_check(&load(rel));
        assert!(
            report.ok,
            "{rel} should be healthy, issues: {:?}",
            report.issues
        );
        assert!(report.count > 0);
    }
}

/// A1: an empty result fails (would exit non-zero).
#[test]
fn empty_result_fails() {
    let manifest = json!({
        "source": "google_ads_transparency",
        "query": { "advertiser": "Nobody", "region": "US" },
        "count": 0,
        "ads": []
    });
    let report = smoke_check(&manifest);
    assert!(!report.ok, "empty manifest must fail");
    assert!(
        report.issues.iter().any(|i| i.contains("empty")),
        "must name the empty condition: {:?}",
        report.issues
    );
}

/// A1: a schema-invalid envelope (missing the source discriminator) fails.
#[test]
fn schema_invalid_fails_and_names_source() {
    let manifest = json!({
        "query": { "region": "US" },
        "count": 1,
        "ads": [ { "id": "CR123" } ]
    });
    let report = smoke_check(&manifest);
    assert!(!report.ok, "missing source must fail");
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.contains("schema") && i.contains("source")),
        "must name the missing source field: {:?}",
        report.issues
    );
}

/// A2: selector drift (a baseline field gone null across ads) fails and names the field.
#[test]
fn drift_null_dates_fails_and_names_field() {
    let manifest = json!({
        "source": "google_ads_transparency",
        "query": { "advertiser": "Nike", "region": "US" },
        "count": 3,
        "ads": [
            { "id": "CR1", "media_type": "image", "first_shown": null },
            { "id": "CR2", "media_type": "image", "first_shown": null },
            { "id": "CR3", "media_type": "image", "first_shown": null }
        ]
    });
    let report = smoke_check(&manifest);
    assert!(!report.ok, "all-null dates must fail");
    assert!(
        report.issues.iter().any(|i| i.starts_with("first_shown")),
        "drift report must name first_shown: {:?}",
        report.issues
    );
}

/// A2: the Meta baseline names started_at when it drifts.
#[test]
fn meta_drift_names_started_at() {
    let ads: Vec<Value> = (0..5)
        .map(|i| json!({ "id": format!("{i}00000"), "media_type": "image", "started_at": null }))
        .collect();
    let manifest = json!({
        "source": "meta_ad_library",
        "query": { "keyword": "shoes", "region": "US" },
        "count": ads.len(),
        "ads": ads
    });
    let report = smoke_check(&manifest);
    assert!(!report.ok);
    assert!(
        report.issues.iter().any(|i| i.starts_with("started_at")),
        "must name started_at: {:?}",
        report.issues
    );
}
