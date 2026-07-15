# ADI-T-0004 proof capsule — Ads packs smoke-test lane

## Checker
`ads-smoke` bin (rzn_core) + `rzn_core::ads_smoke::smoke_check`: validates a manifest
vs schema/ads-manifest-v1.json + per-source field baseline (google: media_type,first_shown;
meta: media_type,started_at; both: id, non-empty, 80% coverage). Exit 0 healthy / nonzero degraded.

## A1 (exit 0 healthy, nonzero on empty/schema-invalid)
- make ads-smoke -> both packs OK, exit 0:
    ads-smoke [OK] source=meta_ad_library ads=5
    ads-smoke [OK] source=google_ads_transparency ads=5
- invalid.manifest.json -> exit 1 ("schema: id/source required; id: missing on 1/3 ads").
- Focused tests: cargo test -p rzn_core --test ads_smoke_test -> 5 passed
  (healthy_fixtures_pass, empty_result_fails, schema_invalid_fails_and_names_source,
   drift_null_dates_fails_and_names_field, meta_drift_names_started_at).

## A2 (failure names missing/degraded fields)
- schema_invalid_fails_and_names_source: names missing `source`.
- drift_null_dates_fails_and_names_field: names `first_shown`.
- meta_drift_names_started_at: names `started_at`.

## A3 (cadence + invocation documented, one command)
- One command: `make ads-smoke`. Docs: docs/ads-smoke-lane.md (daily/pre-release cadence;
  offline CI via cargo test -p rzn_core --test ads_smoke_test --test ads_manifest_contract).

## Bonus — caught a real drift
The lane caught live Google selector drift: `.advertiser-suggestion` -> `.advertiser-suggestion-legacy`.
Fixed the Google pack to match `[class*="advertiser-suggestion"]` + wait on stable `material-select-item.item`.

## Artifacts
- crates/rzn_core/src/ads_smoke.rs, crates/rzn_core/src/bin/ads-smoke.rs
- crates/rzn_core/tests/ads_smoke_test.rs
- Makefile target `ads-smoke`; docs/ads-smoke-lane.md
