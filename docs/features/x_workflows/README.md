# X Workflows

## Overview
- Goal: expand the `x.com` pack from simple post extraction into a spec-aligned social workflow family: time-window discovery, full-thread reading, a canonical one-workflow-per-operation parity pack, a portable social-card catalog, and a browser-side social profile contract.
- Constraints: `x.com` is a client-rendered SPA with login walls and shifting markup; the workflow DSL cannot natively loop over extracted URLs and aggregate nested outputs; auth should stay tied to the logged-in browser session; mutating send/post actions need an explicit approval policy because users may want either hard review gates or fast YOLO continuation.

## Flow Diagrams
- End-to-end export path
```text
rzn-browser run x search-posts --param handle=... --param since_date=... --param until_date=...
  -> candidate post URLs
  -> rzn-browser run x thread --param post_url=... --param handle=... (repeat per URL)
  -> caller assembles JSON / Markdown and downloads assets from the returned URL lists
```

- Review-gated interaction path
```text
current tab
  -> navigate to target post/profile/messages route
  -> activate textbox/composer with trusted click when needed
  -> type_text
  -> assert_selector_state(send/reply enabled)
  -> request_user_intervention
  -> click final send/like control only after Continue
```

- Session model
```text
logged-in Chrome profile
  -> browser worker / extension
  -> current-tab or new-tab workflow
  -> x.com requests use browser-managed cookies
  -> workflow reads hydrated DOM
```

## Decision Record
- Chosen: keep the data plane DOM-first and workflow-level. X-specific selectors and thread extraction logic live in workflow JSON and wrapper scripts, not shared engine code.
- Chosen: implement full-thread export as `search workflow + thread workflow + wrapper script`. This is the cleanest fit because the workflow DSL does not support `for each extracted status URL, run another sequence and merge the results`.
- Chosen: for explicit source URLs, pivot thread discovery through X conversation search using `from:<handle> conversation_id:<status_id>`. That is more reliable than inferring thread/comment boundaries from the conversation page’s DOM layout.
- Chosen: add a spec-aligned catalog file at `resources/cards/social/x_v1.json` plus a browser connector profile at `resources/cards/social/x_browser_profile_v1.json`. The catalog maps card ids to workflows; the profile encodes the browser auth model, required operations, and approval capability surface.
- Chosen: turn `request_user_intervention` into an approval-policy step instead of a styled timeout. That allows the same workflow to run in safe-review, notify-and-stop, auto-continue, or noop-stop modes.
- Rejected: private X API / GraphQL calls via `same_origin_request`. Too brittle and unnecessarily tied to undocumented request headers and response formats.
- Rejected: raw-cookie-centric automation. Browser-managed session reuse is simpler and safer.

## Architecture

### Post-consolidation state (2026-04-24)

The pack was consolidated from 37 files down to 13 canonical workflows. `_v1` filename suffixes were removed (id + version live inside the JSON). Every surviving workflow flips `browser_automation.use_current_tab: false` so parallel runs can execute against the same authenticated Chrome session.

- Modules — canonical pack (read-only)
  - `workflows/x/x_home_timeline_digest.json`: home timeline digest in a fresh tab.
  - `workflows/x/x_open_post.json`: single-post DOM + GraphQL snapshot.
  - `workflows/x/x_open_article.json`: longform `/article/<id>` reader.
  - `workflows/x/x_open_inbox.json`: DM inbox opener (handles passcode onboarding state).
  - `workflows/x/x_open_dm_thread.json`: DM thread opener.
  - `workflows/x/x_search_posts.json`: search a handle optionally within a date window (merges the legacy `search-user-window` + `search-top-from-user`).
  - `workflows/x/x_profile_posts.json`: profile timeline extractor.
  - `workflows/x/x_thread.json`: same-author thread expansion via conversation search (merges the legacy `thread-from-post-url` + `thread-from-current-tab`).
- Modules — canonical pack (mutating, review-gated)
  - `workflows/x/x_like_post.json`
  - `workflows/x/x_reply_post.json`
  - `workflows/x/x_create_post.json`
  - `workflows/x/x_send_dm.json`
  - `workflows/x/x_reply_dm_thread.json`
- Catalog
  - `resources/cards/social/x_v1.json`: canonical social-card catalog.
  - `resources/cards/social/x_browser_profile_v1.json`: browser social connector profile for X.
- Deleted in the consolidation
  - All 11 `x-debug-*` workflows (speculative tools — Anthropic rule 5).
  - All 4 `x-draft-*` workflows (legacy draft-only siblings — replaced by the review-gated mutating flows, which pause before send).
  - Tab-policy duplicates (`-current-tab` siblings) — fresh-tab-by-default absorbs them.
  - `x-session-cookies-debug.json` — `document.cookie`-only reporter, not part of the real session model.
  - Legacy catalog `resources/cards/social/x.json` and the former `scripts/x_export_threads.py` / `scripts/x_compose_draft.py` Python wrappers have been removed. All orchestration runs directly through the `rzn-browser` binary.
- Data contracts
  - Search workflow returns an array of candidate post objects with `post_url`, `posted_at`, `text`, and counters.
  - Thread workflow returns a single object with `root_post_url`, `posts[]`, `assets.image_urls[]`, `assets.video_urls[]`, and `links[]`.
- Export wrapper writes `<handle>_<since>_<until>_<mode>.json` and `.md` under `output/x/`.
- Exported thread objects may also include `articles[]` entries with `source_url`, `resolved_url`, `markdown`, `assets.image_urls[]`, optional downloaded asset mappings, and optional local `source.html` snapshots when asset download is enabled.

## Spec Crosswalk

| Social-card spec concept | X implementation in this repo |
| --- | --- |
| Catalog document | `resources/cards/social/x_v1.json` |
| Browser connector profile | `resources/cards/social/x_browser_profile_v1.json` |
| Browse / read / engage modes | Timeline digest, post/inbox/thread openers, and review-gated mutating engage flows |
| Safety defaults | Mutating flows use `request_user_intervention` with `approval_mode: ask_user` and `continue_on_timeout: false` |
| Artifacts | `result.json` for workflows, plus `thread.md` / export JSON and Markdown via wrapper |
| DM pattern | `x_open_inbox_v1`, `x_open_dm_thread_v1`, `x_send_dm_v1`, `x_reply_dm_thread_v1` |

## Implementation Notes
- Entry points: raw workflow calls use `rzn-browser run x <workflow> --param ...`; `rzn-browser run ...` remains the repo-local harness; the export helper is still the repo-local path when you explicitly want JSON/Markdown fan-out and asset downloads.
- Search windows: the wrapper computes relative windows using the local date and emits explicit `since_date` / `until_date` values.
- Thread extraction: `x-thread-from-post-url.json` uses `execute_javascript` in the page context to combine same-author posts, image URLs, video URLs, and hrefs into one final payload.
- X article extraction: `x_open_article_v1.json` navigates to `/article/<id>`, reads the hydrated article DOM in the live browser session, and returns article HTML plus localizable asset URLs for export.
- Current-tab extraction: `x-thread-from-current-tab.json` navigates to an explicit post URL, redirects into conversation search for the same author + conversation id, then accumulates matching post cards across scroll passes.
- Canonical parity pack: the new `*_v1.json` workflows are current-tab-first so they reuse the live authenticated browser session and avoid the known `open_new_tab` fragility on X.
- Review-gated mutating flows: `x_like_post_v1`, `x_reply_post_v1`, `x_create_post_v1`, `x_send_dm_v1`, and `x_reply_dm_thread_v1` all assert that the target control is actionable before the final click, then pause on `request_user_intervention` with `approval_mode: "ask_user"` and `continue_on_timeout: false`.
- Approval overrides: the native runner can override those gates globally with `RZN_APPROVAL_MODE` / `RZN_INTERVENTION_POLICY` and `RZN_CONTINUE_ON_TIMEOUT` / `RZN_APPROVAL_CONTINUE_ON_TIMEOUT`.
- X Chat routing: for this authenticated account, inbox access currently lands on `/i/chat` and may be blocked behind passcode onboarding. The canonical inbox and DM workflows now treat `button[data-testid='pin-onboarding-setup-now']` as a real state instead of assuming the composer is immediately available.
- Legacy draft reply / DM flows: these still stop before send and remain useful as a safer debugging path.
- Compose path: review-gated mutating flows (`rzn-browser run x create-post`, `reply-post`, `send-dm`, `reply-dm-thread`) handle their own tab and approval lifecycle; no external compose wrapper is needed.
- Draft composer correctness: the generic `fill_input_field` contenteditable path must not dispatch a second synthetic `input` event after `document.execCommand('insertText')` succeeds. Rich editors like X can interpret that extra event as a duplicate insert, which shows up as doubled reply text.
- Direct typing correctness: on a post page the inline reply textbox is already present. The workflow must first activate that textbox with a trusted `click_element(use_cdp=true)` on the textbox itself. Clicking the reply trigger can route X onto `/compose/post`, which is the wrong surface for the draft-only inline flow.
- Runtime guardrail: the direct `type_text` path should not re-click or reset selection ranges on contenteditable hosts once a trusted activation has already established the caret.
- CDP typing correctness: the trusted `type_text` primitive must not emit duplicate printable-character insertions. For rich editors like X, doubled trusted inserts show visible text while `Reply` stays disabled.
- Cookie debug: still limited to `document.cookie` visibility and intentionally not part of the main session model.
- Output handling: the binary returns one structured JSON payload per workflow invocation. Multi-thread aggregation, Markdown rendering, and asset downloads are caller responsibilities — drive them from your own script or pipeline using the binary's stdout.
- Asset export hygiene: the exporter now writes downloads into an export-scoped `<basename>_assets/` folder and preserves URL-to-local-file mappings in the Markdown/JSON output.
- Explicit-link robustness: the exporter unions a top-of-search snapshot with the scrolled conversation-search collector before reopening each discovered post URL for a richer per-post payload.
- Explicit thread default: for `--post-url` exports, stay current-tab-first and use the scrolled conversation-search collector output directly. Reopening each discovered post is now opt-in via `--reopen-posts` because it is slower, noisier, and worse for stealth.
- Explicit post classifier: probe the root post once with `x_open_post_v1`. If it looks like an X-article launcher rather than a real thread opener, export the linked X article directly and skip the conversation.
- Linked-article capture: after thread extraction, the exporter resolves external links, fetches article HTML, uses a lightweight BeautifulSoup-based content heuristic to emit markdown, and when `--download-assets` is set it stores per-article `source.html` plus local image/video assets under `thread_##/articles/article_##_*`.
- Explicit X-article export: for `/article/<id>` URLs, run `rzn-browser run x open-article --param article_url=https://x.com/<handle>/article/<id>` directly. It returns the article body plus image/video asset URLs in one payload.

## Tasks & Status
- [x] Add windowed user search workflow
- [x] Add single-thread expansion workflow
- [x] Add draft-only reply workflow
- [x] Add draft-only DM workflow
- [x] Add current-tab draft reply / DM workflows
- [x] Add read-only inbox/thread DM helpers
- [x] Add social-card catalog for X
- [x] Add one-workflow-per-operation parity pack for X (`x_daily_scroll`, `x_open_post`, `x_like_post`, `x_reply_post`, `x_create_post`, `x_open_inbox`, `x_open_dm_thread`, `x_send_dm`, `x_reply_dm`)
- [x] Add canonical social catalog at `resources/cards/social/x_v1.json`
- [x] Add browser social connector profile at `resources/cards/social/x_browser_profile_v1.json`
- [x] Formalize approval modes for `request_user_intervention`
- [x] Add runner-level approval override support
- [x] Add export wrapper for JSON + Markdown + optional asset download
- [x] Inline linked-article bodies into X thread exports and download their local assets / HTML snapshots
- [x] Add explicit X longform article export support for `/article/<id>` URLs
- [x] Preserve the exact source post in explicit-thread exports and pivot thread discovery through conversation search
- [x] Write export-scoped asset folders and local file mappings in Markdown/JSON
- [x] Add Chrome-tab compose wrapper for current-tab draft flows
- [x] Live-validate current-tab thread extraction against an authenticated X session in the user's real Chrome profile
- [x] Expand truncated thread posts via `tweet-text-show-more-link`
- [x] Fix duplicate characters in X draft composers by avoiding duplicate input dispatch on successful contenteditable `insertText`
- [x] Fix wrapper deadlock caused by inherited stdout pipes from spawned browser-worker processes
- [x] Fix wrapper JSON parsing for pretty-printed nested workflow payloads
- [x] Align draft reply flows with trusted inline-textbox activation plus explicit `Reply`-enabled assertion
- [x] Add non-pausing reply acceptance debug workflows for current-tab and new-tab validation
- [ ] Live-validate the mutating like/create-post/DM-send/DM-reply paths end to end on the user's authenticated session
- [ ] Resolve the separate `open_new_tab` native-run `about:blank` injection issue so the new-tab acceptance probe works without the current-tab fallback

## What Works (Do Not Change)
- Keep X-specific selectors in workflow data, not in generic engine heuristics.
- Keep the canonical parity pack current-tab-first until the `open_new_tab` path is reliable again on live X flows.
- Keep the primary auth model anchored to the logged-in Chrome session.
- Keep the bounded `Show more` expansion in workflow data so long-post recovery stays site-specific and does not leak into generic engine code.

## Tried & Didn’t Work
- Logged-out inspection of public `x.com` pages: often incomplete and not representative of authenticated flows.
- Static `curl` inspection: server HTML is mostly a shell and does not expose the hydrated thread/composer structure.
- Using JavaScript template literals like `` `/${handle}/status/` `` inside workflow `execute_javascript`: the workflow variable interpolation layer also treats `{handle}` as a placeholder inside those strings, which corrupted the emitted script. Plain string concatenation is safer in workflow-embedded JavaScript.
- `subprocess.run(..., capture_output=True)` inside the export wrapper: this hung because spawned browser-worker descendants inherited the stdout pipe and prevented EOF. Redirecting to temp files avoids that failure mode.
- Relying on programmatic focus for the inline reply composer: X can render text but still reject the draft as unsendable. A trusted activation step is required before direct typing.
- Clicking the visible reply trigger on the post page: this can route the session into `/compose/post` instead of the already-present inline reply textbox, so it is the wrong surface for the target draft flow.
- Treating `request_user_intervention` as a true approval primitive before runtime support existed: it used to auto-continue on timeout, which made it unsafe as a hard review gate.
