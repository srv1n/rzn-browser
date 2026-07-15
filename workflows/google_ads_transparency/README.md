# Google Ads Transparency Center Workflows

Browser workflow pack over the public **Google Ads Transparency Center**
(`adstransparency.google.com`). No Google account is required — the center is
fully public.

| Ref | What it does | Key output |
|---|---|---|
| `google_ads_transparency/search` | Drives the real homepage search box → advertiser typeahead → selects the top advertiser, then enumerates that advertiser's ads. Pure read. | `{ source, query, cap, count, truncated, total_available, advertiser_name, advertiser_id, ads:[{ id, advertiser_id, advertiser_name, media_type, format_code, first_shown, last_shown, copy, url, preview_url }] }` |

## Typical flow

```bash
# List up to 30 Nike ads shown in the US
rzn-browser run google_ads_transparency search --param advertiser="Nike"

# Small page to verify the cap
rzn-browser run google_ads_transparency search --param advertiser="Booking.com" --param cap=10
```

## Parameters

- `advertiser` (required) — advertiser name to search. Typed into the real
  search box; the top **advertiser** suggestion (not a website suggestion) is
  selected.
- `region` (default `US`) — region to scope ads to. **v1 supports `US`** and
  returns a manifest with `error: "region_not_supported:<X> (supported: US)"`
  for anything else. The requested region is always recorded in `query`.
- `cap` (default `30`, min `1`, max `300`) — maximum ads to collect. Pagination
  follows the RPC cursor and stops cleanly at the cap; `truncated: true` means
  more ads were available.

## How enumeration actually happens

The pack **drives the real UI** to resolve the advertiser (homepage → type →
click the top suggestion), then reads that advertiser's creatives from the
site's own data feed: a **same-origin `fetch`** of
`SearchService.SearchCreatives` (the exact RPC the page uses), authenticated
with the page's `xsrfToken` (sent as the `x-framework-xsrf-token` header) and
the session cookies. Pagination follows the response cursor (`"2"`) back into
the next request. This yields clean `id` / `format` / date fields that the grid
DOM does not expose, in one request per page.

The result is a manifest that validates against
[`schema/ads-manifest-v1.json`](../../schema/ads-manifest-v1.json) with
`source: "google_ads_transparency"` — the shared shape used by every ads pack
(the Meta Ad Library pack emits the same shape with `source: "meta_ad_library"`).

## Field notes

- `id` — creative id (`CR…`); `advertiser_id` — advertiser id (`AR…`).
- `media_type` — `image` / `video` / `text`, derived from the creative content;
  `format_code` is the raw numeric code from the source.
- `first_shown` / `last_shown` — ISO dates (`yyyy-mm-dd`) converted from the
  RPC's unix timestamps. `last_shown` is today for still-running ads.
- `copy` — ad text for text creatives; `null` for image/video creatives.
- `preview_url` — the creative's image/thumbnail URL.
- `url` — link to the creative's detail page in the Transparency Center.

## Failure behavior

The pack returns a manifest with `count: 0` and an `error` string instead of
throwing, so a caller can branch:

| error | meaning |
|---|---|
| `advertiser_not_resolved` | The typeahead didn't land on an advertiser page (no advertiser matched, or the UI changed). |
| `xsrf_token_missing` | The page's `xsrfToken` wasn't found — the page shell changed. |
| `region_not_supported:<X> …` | A region other than the wired `US` was requested. |

## Implementation notes

- No CDP; all JS runs in the page main world.
- The extractor reads parameters from `window.__rzn_params`.
- Region codes for the `SearchCreatives` query are currently wired for `US`
  only; additional regions can be added to the `REGION_CODES` map in the
  extractor (capture the page's own request body for that region to read its
  codes).
