# Meta Ad Library Workflows

Browser workflow pack over the public **Meta Ad Library**
(`facebook.com/ads/library`) — the first-party archive of ads running across
Facebook and Instagram. The library is viewable without login; a logged-in
Facebook session in the automation profile improves reliability.

| Ref | What it does | Key output |
|---|---|---|
| `meta_ad_library/search` | Searches by keyword (`mode=keyword`) or by advertiser (`mode=advertiser`) and enumerates matching ads. Pure read. | `{ source, query, mode, cap, count, truncated, advertiser_name, advertiser_id, ads:[{ id, advertiser_id, advertiser_name, media_type, first_shown, started_at, last_shown, copy, url, preview_url }] }` |

## Typical flow

```bash
# Keyword + country search
rzn-browser run meta_ad_library search --param query="running shoes" --param country=US

# All ads for one advertiser (resolves the advertiser's Page id automatically)
rzn-browser run meta_ad_library search --param query="Nike" --param mode=advertiser

# Precise advertiser scope by Page id, larger cap
rzn-browser run meta_ad_library search --param query="Nike" --param mode=advertiser --param page_id=15087023444 --param cap=50
```

## Parameters

- `query` (required) — a keyword (`mode=keyword`) or an advertiser name
  (`mode=advertiser`). Recorded in the manifest `query`.
- `mode` (default `keyword`) — `keyword` for a keyword+country search;
  `advertiser` to scope to a single advertiser's ads.
- `country` (default `US`) — ISO country code the Ad Library requires. Recorded
  as `query.region`.
- `page_id` (optional) — advertiser Facebook Page id for precise advertiser
  scoping; skips name resolution.
- `cap` (default `30`, min `1`, max `300`) — maximum ads to collect. Scroll
  accumulates and stops cleanly at the cap; `truncated: true` means more were
  available.

## How it works

Meta's Ad Library search box is a non-fillable React component, so the pack uses
**Meta's documented Ad Library URL parameters** (`country`, `q`, `search_type`,
`view_all_page_id`) — the officially supported access path for this public tool.
It then scrapes ad cards straight from the rendered DOM, parsed by stable text
anchors (`Library ID`, `Started running on`, `Sponsored`), scroll-accumulating
by ad id up to the cap.

**Advertiser mode** first runs a keyword search for the name, finds the ad card
whose advertiser matches the query, reads the `page_id` sitting nearest that
card's Library ID in the page data, then re-navigates to
`view_all_page_id=<id>` so every returned ad belongs to that advertiser. Pass an
explicit `page_id` to skip resolution.

The result validates against
[`schema/ads-manifest-v1.json`](../../schema/ads-manifest-v1.json) with
`source: "meta_ad_library"` — the shared shape the Google Ads Transparency pack
also emits.

## Creative asset capture (`--download-dir`)

The pack emits `attachment_urls` (one per ad — the video file for video ads, the
creative image for image ads). Pass `--download-dir` to fetch them:

```bash
rzn-browser run meta_ad_library search --param query="Nike" --param mode=advertiser \
  --param cap=30 --download-dir ~/ads/nike
# -> ~/ads/nike/attachments/<nnn>_<ad_id>.{jpg,mp4}  (+ manifest.json)
```

The CLI downloader (`--download-dir`) streams each asset with a **size cap**
(100 MiB), records **path, byte size, and sha256** per asset in
`manifest.json`, and **skips assets already on disk** on a re-run (idempotent).
A failing asset (unreachable, oversized) is recorded as a per-asset `error` and
does not abort the run. `manifest.json` shape:
`{ downloaded, skipped, items: [{ kind, url, path, bytes, sha256, skipped } | { kind, url, error }] }`.

Video capture is opportunistic: a video ad's `<video>` mounts lazily, so the pack
captures the video file when the inline player has loaded and otherwise falls back
to the creative's poster image — every ad still yields a local asset.

## Field notes

- `id` — the ad's **Library ID**; `advertiser_id` — the advertiser's Page id
  (set in advertiser mode, or when a scoped view is reached).
- `media_type` — `image` / `video` / `text` from the card content.
- `first_shown` / `started_at` — ISO date parsed from the card's "Started
  running on" text (both keys carry the same value). `last_shown` is `null` —
  Meta cards show only the start date.
- `copy` — the ad's primary text (may include the CTA link description).
- `preview_url` — the creative image (`scontent`/`fbcdn`); `null` for
  video-only cards.
- `url` — `facebook.com/ads/library/?id=<Library ID>`.

## Notes / limits

- `mode=advertiser` name resolution relies on a matching card appearing in the
  keyword result; pass `page_id` for guaranteed scoping.
- No CDP; all JS runs in the page main world. Parameters are read from
  `window.__rzn_params`.
