//! Contract tests for the ads-intelligence workflow packs (ADI epic).
//!
//! These run with NO live site: they parse the pack manifests and validate
//! sample output manifests against the shared schema (`schema/ads-manifest-v1.json`).

use std::path::PathBuf;

use jsonschema::JSONSchema;
use serde_json::Value;

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

/// Every ads pack must parse as a v1 workflow manifest with real steps and a
/// result selector. Add new packs (e.g. the Meta Ad Library pack) to this list.
#[test]
fn ads_packs_parse_as_manifest() {
    let packs = [
        "workflows/google_ads_transparency/search.json",
        "workflows/meta_ad_library/search.json",
    ];
    for rel in packs {
        let v = load(rel);
        assert_eq!(
            v.get("schema_version").and_then(Value::as_str),
            Some("rzn.workflow_manifest"),
            "{rel} must be a rzn.workflow_manifest"
        );
        let steps = v
            .get("steps")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("{rel} has no steps array"));
        assert!(!steps.is_empty(), "{rel} has no steps");
        assert!(
            v.get("capability").and_then(Value::as_str).is_some(),
            "{rel} has no capability"
        );
        assert!(
            v.pointer("/result/output_selector/step_id").is_some(),
            "{rel} has no result.output_selector.step_id"
        );
    }
}

/// A produced manifest must validate against the shared ads schema, carry a
/// recognized `source` discriminator, and a malformed one must be rejected.
#[test]
fn ads_manifest_fixtures_validate_against_shared_schema() {
    let schema = load("schema/ads-manifest-v1.json");
    let compiled = JSONSchema::compile(&schema).expect("compile ads-manifest-v1 schema");

    // Each ads pack's produced manifest validates and carries its own source discriminator.
    let valid_fixtures = [
        (
            "workflows/fixtures/ads/google_ads_transparency.manifest.json",
            "google_ads_transparency",
        ),
        (
            "workflows/fixtures/ads/meta_ad_library.manifest.json",
            "meta_ad_library",
        ),
    ];
    for (rel, source) in valid_fixtures {
        let manifest = load(rel);
        if let Err(errors) = compiled.validate(&manifest) {
            let msgs: Vec<String> = errors.map(|e| e.to_string()).collect();
            panic!("valid manifest {rel} should pass schema, got: {msgs:?}");
        }
        assert_eq!(
            manifest.get("source").and_then(Value::as_str),
            Some(source),
            "{rel} must carry source discriminator {source}"
        );
    }

    let invalid = load("workflows/fixtures/ads/invalid.manifest.json");
    assert!(
        !compiled.is_valid(&invalid),
        "invalid ads manifest (missing source/id) must fail schema validation"
    );
}
