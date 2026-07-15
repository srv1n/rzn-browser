# ADI-T-0001 proof capsule — Meta Ad Library pack

## Live proof runs (facebook.com/ads/library)
- A1 (advertiser search, >=20 ads w/ id, started_at, media_type, copy):
  query=Nike mode=advertiser cap=25 -> count=25, advertiser_id resolved to
  15087023444 (Nike), advertiser breakdown {Nike: 25}. All 25 have
  id + started_at + media_type + copy.
- A2 (keyword + country, recorded in manifest): query="running shoes" country=US
  cap=25 -> count=25 across 18 advertisers; manifest query={keyword:"running shoes", region:"US"}.
- A3 (pagination cap honored): mode=advertiser cap=8 -> count=8, truncated=true, clean stop.

Sample record (advertiser mode):
{ "id":"1869276447125570", "advertiser_id":"15087023444", "advertiser_name":"Nike",
  "media_type":"image", "first_shown":"2026-03-17", "started_at":"2026-03-17",
  "last_shown":null, "copy":"Celebra tu cumpleanos con Nike ...",
  "url":"https://www.facebook.com/ads/library/?id=1869276447125570", "preview_url":"https://scontent.../..." }

## A4 + A5 (schema validation + fixture contract test, no live site)
cmd: cargo test -p rzn_core --test ads_manifest_contract
result: 2 passed. Parses meta pack manifest; meta manifest validates vs
schema/ads-manifest-v1.json with source=meta_ad_library; invalid fixture rejected.

## Artifacts
- workflows/meta_ad_library/search.json  (one pack, mode=keyword|advertiser; discovered as meta-ad-library/search)
- workflows/fixtures/ads/meta_ad_library.manifest.json  (fixture)
- crates/rzn_core/tests/ads_manifest_contract.rs  (extended to cover meta pack + fixture)
- shared schema/ads-manifest-v1.json (reused; source=meta_ad_library)

## Notes
- Meta search box is a non-fillable React component; pack uses Meta's documented
  URL params + DOM scrape (justified deviation from the drive-real-UI default,
  which targets bot-fingerprint risk on sites without such an API).
- advertiser mode resolves page_id by name-matching a card then reading the
  page_id nearest that card's Library ID (most-frequent heuristic was wrong).
