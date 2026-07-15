---
task: ADI-T-0001
date: 2026-07-11
status: review
---

# Meta Ad Library pack — done, in review

Shipped `workflows/meta_ad_library/search.json` (one pack, mode=keyword|advertiser)
+ fixture + extended `crates/rzn_core/tests/ads_manifest_contract.rs`. All 5
acceptance rows verified (proof_status=satisfied). Live-proven: advertiser Nike
cap=25 → 25 ads 100% Nike; keyword "running shoes" US cap=25 → 25/18 advertisers;
cap=8 → 8 truncated.

Both ADI ready tasks (ADI-T-0003 Google, ADI-T-0001 Meta) are now in review and
share `schema/ads-manifest-v1.json`.

## Signal for remaining ADI backlog

- **ADI-T-0002 (Meta asset download):** the Meta pack already extracts
  `preview_url` (image) and detects video cards. Reuse this pack's card scrape;
  add per-ad video/image URL capture + download via `attachment_urls` +
  `--download-dir` (same mechanism as libgen). Video src isn't an `<img>` — grab
  it from the card's `<video>`/source or the ad detail.
- **ADI-T-0004 (smoke lane):** run each pack at a tiny cap, assert the envelope
  is non-empty and schema-valid; nonzero exit on empty/invalid. Both packs return
  `count:0` + `error`/`warning` on failure instead of throwing — the smoke lane
  can branch on `count==0`.

## Gotchas worth knowing

- Meta search box is a non-fillable React `[role=searchbox]` → used documented
  URL params (country/q/search_type/view_all_page_id) + DOM scrape.
- Advertiser page_id: resolve by name-matching a card then nearest page_id to its
  Library ID. Most-frequent-page_id is WRONG (co-advertiser with more ads wins).
- Captured in memory `project_meta_ad_library_2026_07_11`.
