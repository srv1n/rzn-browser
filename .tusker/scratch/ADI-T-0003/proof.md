# ADI-T-0003 proof capsule — Google Ads Transparency Center pack

## Live proof runs (adstransparency.google.com, advertiser "Nike", region US)
- A1 (>=20 ads with id, format, date): cap=30 -> count=30; all 30 ads have
  id + media_type + first_shown + last_shown. media_type breakdown: 29 image, 1 text.
- A3 (pagination cap honored): cap=10 -> count=10, truncated=true, stopped cleanly.
- Advertiser resolved via the real UI typeahead -> AR16735076323512287233
  ("Nike, Inc."), total_available ~10000.

Sample record (cap=10, first ad):
{ "id":"CR00378902083572596737", "advertiser_id":"AR16735076323512287233",
  "advertiser_name":"Nike, Inc.", "media_type":"image", "format_code":1,
  "first_shown":"2023-05-01", "last_shown":"2026-07-11", "copy":null,
  "url":".../creative/CR00378902083572596737?region=US",
  "preview_url":"https://tpc.googlesyndication.com/archive/simgad/6486945144496172146" }

## A2 + A4 (schema validation + fixture contract test, no live site)
cmd: cargo test -p rzn_core --test ads_manifest_contract
result: 2 passed (ads_packs_parse_as_manifest, ads_manifest_fixtures_validate_against_shared_schema)
- Manifest validates against schema/ads-manifest-v1.json with source="google_ads_transparency".
- Invalid fixture (missing source/id) correctly rejected.

## Artifacts
- workflows/google_ads_transparency/search.json  (pack; auto-discovered as google-ads-transparency/search)
- schema/ads-manifest-v1.json                     (shared ads manifest schema, source-discriminated)
- workflows/fixtures/ads/google_ads_transparency.manifest.json, invalid.manifest.json
- crates/rzn_core/tests/ads_manifest_contract.rs  (contract test, no live site)
