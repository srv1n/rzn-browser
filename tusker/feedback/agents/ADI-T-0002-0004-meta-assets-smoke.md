---
task: ADI-T-0002, ADI-T-0004
date: 2026-07-11
status: review
---

# Meta asset capture (0002) + ads smoke lane (0004) — done, in review

All four ADI tasks now in review.

## ADI-T-0004 (smoke lane) — and it earned its keep
`ads-smoke` bin + `rzn_core::ads_smoke` validate a manifest vs schema + per-source
field baseline. `make ads-smoke` (live) + offline tests (`ads_smoke_test`).
**It caught a real drift on its first live run**: Google renamed
`.advertiser-suggestion` -> `.advertiser-suggestion-legacy`. Fixed the Google pack
(match `[class*="advertiser-suggestion"]`, wait on stable `material-select-item.item`).
Lesson: run `make ads-smoke` before trusting a pack after any gap.

## ADI-T-0002 (Meta asset capture)
Pack emits `attachment_urls`; engine downloader extended in a testable lib module
`crates/rzn_browser/src/asset_download.rs` (stream + sha256 + 100MiB cap + skip-existing).
- fbcdn image + video URLs are cookieless-fetchable by reqwest (needed reqwest "stream" feature).
- Video `<video>` mounts lazily -> gradual-scroll + image->mp4 upgrade; top ads often stay poster jpg.
- **Engine change is only in target/debug/rzn-browser** — run `make install` to ship it to the
  installed CLI. The search packs run via the installed binary; capture with hashing/skip needs the rebuild.

## For whoever reviews
- Both packs are live-scrapers of third-party sites; run `make ads-smoke` as part of review.
- Consider a scheduled `make ads-smoke` (daily) — drift is a when-not-if.
