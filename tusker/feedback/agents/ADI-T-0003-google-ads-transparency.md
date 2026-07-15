---
task: ADI-T-0003
date: 2026-07-11
status: review
---

# Google Ads Transparency pack — done, in review

Shipped `workflows/google_ads_transparency/search.json` + shared
`schema/ads-manifest-v1.json` + contract test `crates/rzn_core/tests/ads_manifest_contract.rs`
+ fixtures in `workflows/fixtures/ads/`. All 4 acceptance rows verified
(proof_status=satisfied). Live-proven on advertiser "Nike" (US): cap=30→30 ads,
cap=10→10 with truncated=true.

## Signal for the Meta pack (ADI-T-0001, next)

- **Reuse the harness.** The shared `source`-discriminated schema + contract test
  already exist. The Meta pack should emit the same manifest shape with
  `source: "meta_ad_library"` and add its path to the `ads_packs` list in
  `ads_manifest_contract.rs`. Don't fork the schema.
- **Same DOM-vs-feed lesson likely applies.** For Google the grid DOM lacked
  format/dates; the win was replaying the site's own JSON RPC. Check whether
  Meta's Ad Library GraphQL feed is similarly reachable via same-origin fetch
  before committing to DOM scraping.
- **Meta needs the throwaway FB login** (decided with the human) + heavier
  anti-bot handling than Google. Budget for consent walls / login nudges;
  sprinkle `dismiss_popups`.

## Ops note

- rzn-browser's MV3 extension link drops between runs (`extension_disconnected`);
  `rzn-browser heal` reconnects. Prepend it to every scripted run. Captured in
  memory `reference_rzn_browser_dev_loop_gotchas`.

## Open follow-ups (not blockers for ADI-T-0003)

- Region beyond US: only US region codes (2840/2356) wired; other regions return
  an explicit `error`. Add codes per region as needed.
- Format code enum: only code `1`=image confirmed live; media_type is derived
  from content as the reliable signal. Confirm video/text codes against a
  video-heavy advertiser when convenient.
