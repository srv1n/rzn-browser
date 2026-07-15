//! Smoke check / selector-drift detection for the ads-intelligence workflow packs
//! (ADI epic). Given a produced ads manifest, it (1) validates against the shared
//! schema `schema/ads-manifest-v1.json` and (2) checks that the fields most likely
//! to break when a site changes its markup are still populated across the ads.
//!
//! A healthy manifest returns `ok = true`; an empty, schema-invalid, or degraded
//! manifest returns `ok = false` with human-readable issues naming the offending
//! fields, so a drifted pack fails loudly instead of silently returning junk.

use std::sync::OnceLock;

use jsonschema::JSONSchema;
use serde_json::Value;

/// The shared ads-manifest schema, embedded at compile time so the checker needs
/// no filesystem access at runtime.
const ADS_MANIFEST_SCHEMA: &str = include_str!("../../../schema/ads-manifest-v1.json");

/// Minimum fraction of ads that must carry a baseline field before it is
/// considered healthy (below this reads as selector drift).
const COVERAGE_THRESHOLD: f64 = 0.8;

fn compiled_schema() -> &'static JSONSchema {
    static SCHEMA: OnceLock<JSONSchema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let schema: Value = serde_json::from_str(ADS_MANIFEST_SCHEMA)
            .expect("embedded ads-manifest schema is valid JSON");
        JSONSchema::compile(&schema).expect("embedded ads-manifest schema compiles")
    })
}

/// Result of a smoke check over one ads manifest.
#[derive(Debug, Clone)]
pub struct SmokeReport {
    /// True when the manifest is schema-valid, non-empty, and every baseline
    /// field meets the coverage threshold.
    pub ok: bool,
    /// The `source` discriminator, when present.
    pub source: Option<String>,
    /// Number of ads in the manifest.
    pub count: usize,
    /// Human-readable problems, each naming the offending field or condition.
    pub issues: Vec<String>,
}

impl SmokeReport {
    /// Multi-line report suitable for a CLI, ending without a trailing newline.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let status = if self.ok { "OK" } else { "FAIL" };
        let src = self.source.as_deref().unwrap_or("<no source>");
        out.push_str(&format!(
            "ads-smoke [{status}] source={src} ads={}",
            self.count
        ));
        for issue in &self.issues {
            out.push_str(&format!("\n  - {issue}"));
        }
        out
    }
}

/// The baseline fields (beyond `id`, which the schema already requires on every
/// ad) whose population is checked per source. These are the fields most likely
/// to go null when a site changes its markup.
fn baseline_fields(source: Option<&str>) -> &'static [&'static str] {
    match source {
        Some("google_ads_transparency") => &["media_type", "first_shown"],
        Some("meta_ad_library") => &["media_type", "started_at"],
        _ => &["media_type"],
    }
}

fn is_populated(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(_) => true,
    }
}

/// Run the smoke check over a produced ads manifest.
pub fn smoke_check(manifest: &Value) -> SmokeReport {
    let source = manifest
        .get("source")
        .and_then(Value::as_str)
        .map(str::to_string);

    let ads = manifest.get("ads").and_then(Value::as_array);
    let count = ads.map(|a| a.len()).unwrap_or(0);
    let mut issues = Vec::new();

    // 1) Schema validity — a structural break fails hard.
    if let Err(errors) = compiled_schema().validate(manifest) {
        for e in errors {
            issues.push(format!("schema: {e}"));
        }
    }

    // 2) Non-empty — an empty result is drift or a dead query.
    if count == 0 {
        issues.push("empty result: 0 ads returned".to_string());
        return SmokeReport {
            ok: false,
            source,
            count,
            issues,
        };
    }
    let ads = ads.expect("ads array present when count > 0");

    // 3) Every ad must carry an id.
    let missing_id = ads.iter().filter(|a| !is_populated(a.get("id"))).count();
    if missing_id > 0 {
        issues.push(format!("id: missing on {missing_id}/{count} ads"));
    }

    // 4) Baseline fields must clear the coverage threshold.
    for field in baseline_fields(source.as_deref()) {
        let have = ads.iter().filter(|a| is_populated(a.get(*field))).count();
        let coverage = have as f64 / count as f64;
        if coverage < COVERAGE_THRESHOLD {
            issues.push(format!(
                "{field}: populated on {have}/{count} ads ({:.0}% < {:.0}% baseline)",
                coverage * 100.0,
                COVERAGE_THRESHOLD * 100.0
            ));
        }
    }

    SmokeReport {
        ok: issues.is_empty(),
        source,
        count,
        issues,
    }
}
