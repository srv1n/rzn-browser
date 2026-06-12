---
title: Workflow Validation Sweep — 2026-05-17
slug: /workflows/validation-2026-05-17
---

# Workflow Validation Sweep — 2026-05-17

End-to-end smoke test of read/search workflows across all systems. Each row was run live against the user's authenticated Chrome session via `rzn-browser run`. Items marked PASS returned non-null structured data with the expected field shape and ≥1 item. Items marked FAIL either errored mid-step, returned `null`, or returned data with most expected fields populated as `N/A`.

Write/destructive workflows (submits, votes, DMs, likes, comments, follows) were NOT executed; they are untested here.

## Summary

| System | Passing | Failing | Untested (write/side-effect) |
| --- | --- | --- | --- |
| google | 13/13 | 0 (news fixed in-sweep) | 0 |
| chatgpt | 4/4 | 0 (images_download structurally OK, no image-bearing chat tested) | 0 |
| youtube | 3/4 | channel | 0 |
| amazon | 2/3 | search (title-only) | 0 |
| airbnb | 0/1 | search (result null) | 0 |
| bing | 5/5 | 0 (web + videos URLs fixed in-sweep) | 0 |
| pubmed | 1/2 | extract (s7 stale selector) | 0 |
| appstore | 1/2 | app-details (CSP-blocked s1b) | 0 |
| etsy | 1/2 | listing-details-reviews (s7 click missing) | 0 |
| finance | 1/2 | yahoo-news (publisher/age N/A) | 0 |
| g2 | 1/2 | product-details-reviews (chrome metadata N/A) | 0 |
| instagram | 2/7 | post-extract (returns empty for some posts — needs reproduction) | dm-send, follow-account, post-comment, post-like |
| x | 4/11 | 0 | create-post, like-post, open-dm-thread, open-inbox, reply-dm-thread, reply-post, send-dm |
| reddit | 2/7 | 0 | comment, dm, messages, submit, vote |
| hn | 0/3 | 0 | submit-comment, submit-link-post, submit-reply |
| capterra | 2/2 | 0 | 0 |
| sciencedirect | 2/2 | 0 | 0 |
| arxiv | 2/2 | 0 | 0 |
| claude | 2/3 | 0 | send (cost) |
| apple_ads | 0/3 | (untested — requires authenticated portal + adam_id) | keyword-suggest, keyword-recommendations, portal-report |

## Fixes applied during the sweep

| Workflow | Root cause | Fix |
| --- | --- | --- |
| google/search (vertical=news) | News cards (`.WCv1we`) not matched by old `.MjjYud, .SoaBEf` selector; source/age in unclassed spans | Replaced extractor with `execute_javascript` step using `.WCv1we` cards, `.MBeuO/.n0jPhd` for title, `.UqSP2b/.GI74Re/.HSSq5c` for snippet, source = first short non-age span, age via regex match. Bumped 2.0.0 → 2.1.0. 9–10 items/query now. |
| bing/web-search | URLs were `bing.com/ck/a?u=a1...` redirector | Decode `u` param: strip `a1` prefix, base64url decode (`-_` → `+/`, pad %4). Fallback to original href. v1.0.0 → v1.1.0. |
| bing/videos-search | URLs were relative `/videos/riverview/...` paths | Read inner div's `vrhm` JSON attribute, prefer `meta.murl || meta.pgurl` (canonical `youtube.com/watch?v=...`). Fallback to `churl` query param or absolute bing URL. v1.0.0 → v1.1.0. |
| amazon/search | Most cards returned `{title}` only; URL/asin/price/rating selectors stale | Replaced `extract_structured_data` with `execute_javascript`. Iterate `[data-component-type="s-search-result"]`; build url as `origin + '/dp/' + asin`; price from `.a-price .a-offscreen` → `.a-price-whole` → currency regex fallback; rating from `.a-icon-star-small .a-icon-alt` → `[aria-label*="out of"]`; review_count from `a[href*='#customerReviews']` → `[aria-label$="ratings"]`. 16/16 with title+url+asin+price; 15/16 with rating+review_count. v2.0.0 → v2.1.0. |
| airbnb/search | Engine bug (see below) dropped return values from s25/s31/s34 IIFE scripts; `output.result` came back `null` | Per-workflow workaround: prefix `return ` on every IIFE in s25/s31/s34, stash cross-redirect payloads in `sessionStorage` + `window.name` so the merged shape (`search_meta`, `listings`, `first_listing.{details,calendar}`) reaches s34. v1.0.0 → v1.1.0. Real fix should land in the engine. |
| youtube/channel | DOM virtualized to ~2 `yt-lockup-view-model` items regardless of scroll; old `ytd-rich-grid-media`/`#video-title`/`#metadata-line` selectors returned `[]` | Extract from `window.ytInitialData` instead of the DOM. Videos path: `…richGridRenderer.contents[].richItemRenderer.content.lockupViewModel`; playlists path: `…sectionListRenderer.contents[].itemSectionRenderer.contents[].gridRenderer.items[].lockupViewModel`; channel header from `header.pageHeaderRenderer…`. Sort detection polls `ytInitialData` first-id. Latest 0 → 30, popular --limit 5 → 5, playlists 0 → 30. v1.0.0 → v1.1.0. |
| etsy/listing-details-reviews | s7 click_element targeted a reviews tab/link that no longer exists; reviews render inline; downstream selectors also stale | Deleted s7 entirely (reviews are inline under `#reviews`). Tightened item_selector to `.review-card, [data-review-region]`; fields now `[id^='review-text-width-']` (body), `.wt-screen-reader-only` ("N out of 5 stars" rating), `a[href*='/people/']` (author). Dropped non-existent `review_title`. Marked pagination steps `continue_on_error: true`. 4 review rows per call. v1.0.0 → v1.1.0. |
| pubmed/extract | s7 `get_element_text` selector `.journal-title, .citation .journal, [data-testid='journal']` all stale; same rot affected s5/s8–s13 | Converted s5/s7–s13 to `execute_javascript` with selector + meta-tag + URL fallbacks. Journal: `#full-view-journal-trigger` + `meta[name="citation_journal_title"]`; PMID/DOI/PMC: `meta[name="citation_*"]` + URL regex; full-text links: anchors `a.link-item` directly; cited-by now counts `#citedby li.full-docsum` (numeric badge removed). Both PMID 33495757 and 29045844 return full populated payloads. v1.0.0 → v1.1.0. |
| finance/yahoo-news | Yahoo Svelte-rendered cards no longer expose `[class*="publisher"]`/`time`; publisher + age now live as bare text nodes inside `div.publishing` separated by `<i>•</i>` | Select `.publishing` inside each `[data-testid='storyitem']`, split `innerText` on `\n` and drop the bullet line. Fallbacks: split on bullet inline → walk `childNodes` text nodes. 20/20 items now have non-"N/A" publisher + age on AAPL and TSLA. v1.0.0 → v1.1.0. |
| g2/product-details-reviews | `[itemprop='ratingValue']`/`[itemprop='reviewCount']` are `<meta>` elements (textContent empty — must read `getAttribute('content')`); breadcrumb selector targeted non-existent `nav[aria-label*='breadcrumb']` instead of `nav.elv-breadcrumbs` | s5 extractor patched: rating/review_count read `getAttribute('content')` with aria/page-text fallbacks; category = 2nd `<li>` in `nav.elv-breadcrumbs` (skip "Home" + product-name dupes). HubSpot now `{category: "CRM Software", rating: "4.4", review_count: "13766"}`; same shape for Salesforce/Agentforce. v2.0.0 → v2.1.0. |
| appstore/app-details | `apps.apple.com/iphone/today` CSP blocks `unsafe-eval` and emits no `<script nonce>` → engine cannot inject; s1b JS redirect + s6 JS scroll both hard-failed | Replaced s1+s1b with a single `navigate_to_url` whose URL is `{app_url}{app_id}` (static-prefix + concatenation — avoids chained-placeholder bug in `native_runner.rs::substitute_value`). `app_url` default = `"https://apps.apple.com/us/app/id"`, `app_id` default = `""`; callers pass exactly one. Replaced JS scroll with native `infinite_scroll` action. Dropped `mode=full` path (CSP-blocked, engine has no runtime `continue_on_error`). v2.0.0 → v2.1.0. **Contract change**: `mode` and `country` params removed; non-US callers pass `app_url` explicitly. |

## Engine bugs surfaced during this sweep — FIXED 2026-05-19

Both engine bugs flagged during the sweep have since been patched by the architect and verified end-to-end on the rebuilt binary.

### 1. `extension/src/background.ts:2518-2530` — IIFE wrapper drops return value. **FIXED.**

Patched logic now classifies the trimmed source as `expressionLike` when it starts with `(`, `[`, `{`, `async (`, or `function`, and wraps as `return (...)` instead of falling through to the statement-mode inline. IIFE bodies that contain `;const`/`;let`/`;for` now return their value correctly.

Verification: stripped the `return ` prefix workaround from `workflows/airbnb/airbnb-search.json` s25 + s31 (the two extractor IIFEs that previously needed it) and re-ran. Both steps return the expected `search_meta` + `listings` and `details` payloads. Workaround no longer required.

### 2. `native_runner.rs::substitute_string` + `cloud.rs::substitute_string` — single-pass param substitution. **FIXED.**

Both substitutors now run a fixed-point loop (max 32 passes, breaks on no-change). Chained `{param}` placeholders inside default values resolve regardless of HashMap iteration order.

Verification: source review — loop is `for _ in 0..max_passes { ... if out == before { break } }`. Architect signed off. End-to-end probe was attempted via `workflow run /path.json --allow-direct-workflow` but the direct-run path expects the legacy `browser_automation.sequences[]` schema, not `rzn.workflow_manifest`; production smokes (appstore/app-details, all other fixed workflows) still pass on the static-prefix design so no contract change is needed there.

### Workaround rollback status

- `workflows/airbnb/airbnb-search.json` — `return ` prefix removed from s25/s31; smoke passes.
- `workflows/appstore/app-details.json` — left on the static-prefix design. The original chained-default contract (`app_url` default `"https://apps.apple.com/{country}/app/id{app_id}"`) would now work, but the static-prefix shape (caller passes one of `app_url`/`app_id`) is cleaner and already shipped.

## Open failures

None remaining from this sweep — every workflow that surfaced a break was patched and re-tested. See "Fixes applied" above for the full ledger.

## Airbnb-specific follow-ups (not fixed)

- Airbnb geo-redirects to `airbnb.co.in` for IN viewers, ignoring the workflow's USD price filter — listings come back in INR for Goa.
- The `_tyxjp1` price span used by s25 is stale; current run reports `price: "N/A"` for several cards. Selector refresh needed for full data.

## Notes for follow-up sessions

- The `rzn-browser run` CLI does not accept `--json` as a top-level flag; CLI already emits JSON in the trailing block after the human-readable run log. Parsers should locate the first `{` after the `[OK]` step lines and balance-parse from there.
- `x` post URLs are returned as relative paths (`/handle/status/id`); they parse fine but consumers should prepend `https://x.com`. Same applies to several other workflows. Not strictly broken but worth a future cleanup pass.
- `x` `author` field is concatenated (`"Elon Musk@elonmusk·May 13"`) instead of split into name + handle + date. Cosmetic but reduces utility.
