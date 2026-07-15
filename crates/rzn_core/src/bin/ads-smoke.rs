//! Smoke-test lane for the ads-intelligence workflow packs (ADI-T-0004).
//!
//! Reads a produced ads manifest (from a file argument, or stdin) and validates
//! it against the shared schema + a per-source field baseline. Exits 0 when the
//! manifest is healthy, non-zero when it is empty, schema-invalid, or degraded —
//! naming the offending fields so selector drift is caught in CI.
//!
//! Usage:
//!   ads-smoke <manifest.json>
//!   rzn-browser run google_ads_transparency search --param advertiser=Nike --param cap=5 | ads-smoke -

use std::io::Read;
use std::process::ExitCode;

use rzn_core::ads_smoke::smoke_check;
use serde_json::Value;

fn read_input(arg: Option<&str>) -> Result<String, String> {
    match arg {
        Some("-") | None => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| format!("read stdin: {e}"))?;
            Ok(buf)
        }
        Some(path) => std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}")),
    }
}

/// The input may be a bare manifest, the CLI run envelope
/// (`{ "output": { "result": { ...manifest... } } }`), or that JSON preceded by
/// the `rzn-browser run` banner text. Locate the first JSON object, read one
/// value from it, and unwrap to the manifest.
fn extract_manifest(raw: &str) -> Result<Value, String> {
    let start = raw
        .find('{')
        .ok_or_else(|| "no JSON object found in input".to_string())?;
    let v: Value = serde_json::Deserializer::from_str(&raw[start..])
        .into_iter::<Value>()
        .next()
        .ok_or_else(|| "no JSON value in input".to_string())?
        .map_err(|e| format!("parse JSON: {e}"))?;
    if v.get("source").is_some() && v.get("ads").is_some() {
        return Ok(v);
    }
    if let Some(inner) = v.pointer("/output/result") {
        return Ok(inner.clone());
    }
    if let Some(inner) = v.get("result") {
        return Ok(inner.clone());
    }
    Ok(v)
}

fn main() -> ExitCode {
    let arg = std::env::args().nth(1);
    let raw = match read_input(arg.as_deref()) {
        Ok(raw) => raw,
        Err(e) => {
            eprintln!("ads-smoke [FAIL] {e}");
            return ExitCode::from(2);
        }
    };
    let manifest = match extract_manifest(&raw) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("ads-smoke [FAIL] {e}");
            return ExitCode::from(2);
        }
    };

    let report = smoke_check(&manifest);
    println!("{}", report.render());
    if report.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
