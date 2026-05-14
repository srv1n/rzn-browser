# ChatGPT Workflows — Automation Fixes

## Overview
- Goal: Make the `workflows/chatgpt/*` pack viable for unattended agent automation. Today it works for an attended human at the keyboard but breaks on long generations, the new ChatGPT-Images DOM, focused-tab assumptions, and missing `chat_id` recovery handles. This feature tracks four blocker issues and two stretch additions surfaced by an external automated pipeline (Reddit-posting agents that spawn ChatGPT image jobs and download results).
- Constraints: Keep ChatGPT-specific logic inside the workflow pack (per `chatgpt_web_app_workflows`). Do not introduce site-specific selectors into shared executor code. ChatGPT-Images UI changes frequently, so prefer network-level signals + accessibility/role probes over brittle `<img>` lookups. Engine changes for dedicated-tab spawn belong in the runtime; the chatgpt pack should consume them, not reimplement.

## Reporter Context
External user running rzn-browser 0.2.5 on macOS Darwin 25.3.0 + Chrome stable as of 2026-05-01, ChatGPT Plus/Pro signed in. Building a fully automated Reddit-posting pipeline: background agents spawn ChatGPT image generations, download results, post via the working `reddit-*` workflows. The `chatgpt-*` workflows are the blocker. Repro chat: `69f49357-94a8-83a6-b9e4-8871f8e24aa7` (3 messages, images visible in browser, `images-get-latest-v1` reports empty).

## Flow Diagrams
- Current (broken for automation):
```text
agent run -> workflow used the active tab
          -> injects into whatever tab has focus (often wrong)
          -> s4 send prompt -> chat_id captured locally but NOT returned on failure
          -> s6 wait_for_element timeout_ms: 600000 (10 min)
          -> 7-image generation often takes 12-20 min
          -> native_host_disconnected -> chat_id lost -> unrecoverable
```

- Target:
```text
agent run -> runtime opens dedicated tab
          -> images-submit-only-v1 -> returns chat_id, exits
          -> caller persists chat_id, polls externally
          -> images-by-chat-id-v1 (chat_id) -> shadow-pierced DOM probe
                                            -> Network.responseReceived fallback
                                            -> downloads via chrome.downloads
```

## Decision Record
- Fix Issue 4 first (always return `chat_id`) — smallest patch, biggest unblock. Without this every other failure mode is unrecoverable, so all other fixes are gated on having a stable handle.
- Prefer fixing the engine over splitting workflows, per AGENTS.md rule 4 ("If the engine is the blocker, patch the engine — don't ship the split."). But Issue 1 has a real reason to split: a fundamentally different output contract (`{chat_id}` vs `{images, files}`) and a side-effect class (pure submit vs. download with disk writes) that callers want to reason about separately. So `images-submit-only-v1` + `images-by-chat-id-v1` is acceptable — and once they exist, `images-generate-and-download-v1` should be re-evaluated (probably a thin wrapper, possibly deprecated).
- Network-level fallback (CDP `Network.responseReceived`) is the right defense for Issue 2. The DOM strategy will keep breaking; the network surface is much more stable. Keep DOM-pierce as primary (cheaper, no CDP wiring per call) and network log as fallback.
- Dedicated-tab behavior for Issue 3 must be a real engine feature (spawn dedicated tab → run → close on success / leave on error), not a workaround. Confirm whether the runtime already supports this; if not, this becomes an engine task.
- Do not add a separate "headless mode" workflow variant — same workflow JSON should run correctly whether attended or unattended. The attended-only assumption is the bug.

## Architecture
- Affected workflows:
  - `workflows/chatgpt/chatgpt_images_generate_and_download_v1.json` — timeout, missing chat_id on failure, active-tab dependence.
  - `workflows/chatgpt/chatgpt_images_get_latest_v1.json` — DOM selector misses current ChatGPT-Images surface.
  - `workflows/chatgpt/chatgpt_images_download_current_rendered_v1.json` — same DOM issue + active-tab dependence.
  - `workflows/chatgpt/chatgpt_new_chat_send_v1.json` — verify `chat_id` returned (Issue 4).
  - All other `workflows/chatgpt/*.json` that touch a conversation — audit for `chat_id` in success envelope.
- New workflows (stretch, after blockers fixed):
  - `workflows/chatgpt/chatgpt_images_submit_only_v1.json` — submit prompt, return `chat_id`, exit. No wait, no download.
  - `workflows/chatgpt/chatgpt_images_by_chat_id_v1.json` — take `chat_id`, probe shadow DOM + network log, download images. Replaces `images-get-latest-v1` and `images-download-current-rendered-v1` if the consolidated path proves robust.
- Engine touchpoints (likely):
  - Runtime tab-spawn semantics for dedicated workflow tabs.
  - CDP `Network` domain wiring inside an `execute_javascript` step or as a new workflow step type (`capture_network_log`).
  - Workflow error envelope: include partial outputs (e.g., `chat_id`) when later steps fail.

## Implementation Notes
### Issue 1 — 10-min watcher cap is insufficient and silently destroys recoverable state
- **Repro:** `rzn-browser run chatgpt images-generate-and-download-v1 --param "message_text=<long prompt asking for 7 variations>" --param "download_folder=test_$(date +%s)" --param "variation_count=7"` — 7-variation generations regularly take 12–20 min; step `s6` (`wait_for_element`) has `timeout_ms: 600000` (10 min); when it elapses the workflow fails with `native_host_disconnected` and the `chat_id` is lost even though images are still rendering in the browser.
- **Observed log:**
  ```
  [STEP] 7/9 s6 (wait_for_element)
  [ERR] s6 (wait_for_element) Native host timeout after 605000ms
  Workflow failed: chatgpt/chatgpt_images_generate_and_download_v1
  Failed at: s6
  Reason: native_host_disconnected
  ```
- **Fixes (in priority order):**
  1. Capture `chat_id` immediately after `s4` (send prompt) and surface it in workflow output even on later failure. `s4` already returns `{url, title, submitted_prompt, variation_count}`, but only after the URL transitions to `/c/<chat_id>`. Add an explicit poll for the URL change inside `s4` (or a new `s4.5`) so `chat_id` is always returned. Surface it in the error envelope when subsequent steps fail.
  2. Bump `s6` timeout to `1800000ms` (30 min) as a stopgap for realistic upper-end runs.
  3. Split into submit-only + download-by-chat-id workflows (see stretch items). The current monolith conflates "fire the prompt" and "wait for the result," which is wrong for any generation that may exceed one browser session.

### Issue 2 — DOM selectors miss the current ChatGPT-Images UI
- **Repro:**
  1. Open `https://chatgpt.com/c/<chat_id>` in Chrome where 7 generated images are visually present.
  2. `rzn-browser run chatgpt images-get-latest-v1 --param "chat_id=<chat_id>"`
- **Observed:**
  ```json
  {
    "assistant_turn_count": 0,
    "chat_id": "69f49357-94a8-83a6-b9e4-8871f8e24aa7",
    "image_count": 0,
    "images": [],
    "latest_assistant_response": null,
    "message_count": 3,
    "status": "missing",
    "title": "Carousel Design Brief"
  }
  ```
  Workflow correctly identifies the chat and reads `latest_user_message`, but reports zero images and `latest_assistant_response: null`. Page snapshot via `images-download-current-rendered-v1` confirms zero `<img>` tags and zero image URLs. Current selector strategy looks for `<img>` with `naturalWidth*naturalHeight > 60000`; ChatGPT-Images now appears to render results inside a closed shadow root or via canvas/preview elements that aren't plain `<img>`.
- **Fixes:**
  1. Re-probe the live ChatGPT-Images DOM. Image turns appear inside `[data-message-author-role="assistant"]` containers but the actual image nodes are not plain `<img>`. Check `<picture>`, `<canvas>`, `background-image` CSS on figures, and shadow-pierced selectors via `qsDeep` (the same approach used for new-reddit's file input).
  2. Add a network-level fallback. Hook the Chrome network log via CDP (`Network.responseReceived`) and capture URLs matching `https://*.openai.com/*.png|.webp` (or whatever the current image hosting pattern is). Image URLs flow through the network regardless of DOM mutations.
  3. Probe URL pattern. Verify `chatgpt.com/c/<id>` is even the right surface — ChatGPT-Images may now use `chatgpt.com/g/<id>`, `chatgpt.com/images/<id>`, or a Library view. `images-generate-and-download-v1` starts from `https://chatgpt.com/images/`; confirm whether the post-submit URL transitions to `/c/<id>` or stays in an `/images/` namespace.

### Issue 3 — ChatGPT workflows must not depend on the active tab
- **Repro:** Open Chrome, focus any non-`chatgpt.com` tab (e.g., `chrome://extensions/`). Run any chatgpt workflow.
- **Observed:** `[ERR] s1 (wait_for_element) Cannot inject on non-http(s) URL: chrome://extensions/` → `extension_disconnected`. Or, when the active tab is on a different `http(s)` page, the workflow injects into the wrong page entirely.
- **Impact:** Any non-chatgpt tab focus breaks the workflow. Multiple agents running in parallel collide on the same tab. A human clicking another tab mid-workflow breaks the run. `s0` (`navigate_to_url` to `chatgpt.com/images/`) navigates the active tab away from whatever the human was looking at.
- **Fixes:**
  1. Use dedicated workflow tabs in every `workflows/chatgpt/*.json`. Have the runtime open a fresh tab, navigate it, do the work, close it on success / leave on error.
  2. Verify whether dedicated tabs are already runtime-supported. If yes: document the behavior and patch the chatgpt workflows. If no: implement runtime support first; chatgpt patches block on it.
  3. Confirm whether `reddit-*` workflows tolerate active-tab reuse because they spawn dedicated tabs already, or for some other reason (e.g., URL match guards, single-session usage). Either way, the chatgpt pack should match whatever pattern unblocks unattended runs.

### Issue 4 — Workflow output should always include `chat_id` for any chatgpt workflow that creates or touches a conversation
- **Audit:**
  - `images-generate-and-download-v1` — does NOT surface `chat_id` (only `submitted_prompt` and `url` from the pre-navigation point). **Bug.**
  - `recent-chats-v1` — returns `chat_ids`. OK.
  - `images-get-latest-v1` — echoes the input `chat_id`. OK.
  - `new-chat-send-v1` — needs verification.
  - All other conversation-creating workflows — needs audit.
- **Fix:** Add a final `s_capture_chat_id` step (or fold into existing JS) that polls `window.location.href` for `/c/<uuid>` and returns the parsed UUID. Add `chat_id` to the workflow result schema across all conversation-creating workflows. Surface in error envelopes when later steps fail.

## Tasks & Status

Priority order (smallest fix → largest unblock first):

- [ ] **P1 — Issue 4: Always return `chat_id`** in `images-generate-and-download-v1` and any other conversation-creating workflow that's missing it. Audit `new-chat-send-v1`, `new-chat-send-attachment-v1`, `continue-chat-v1`, `images-new-generation-v1`, `images-new-generation-attachment-v1`. Surface `chat_id` in error envelopes.
- [ ] **P2 — Issue 2: Re-probe ChatGPT-Images DOM** against the live authenticated UI. Update selectors in `images-get-latest-v1` and `images-download-current-rendered-v1`. Add `qsDeep` shadow-pierced fallback. Verify URL surface (`/c/<id>` vs `/images/<id>` vs Library).
- [ ] **P2 — Issue 2: Add network-log fallback** for image URL extraction via CDP `Network.responseReceived`. Filter on current OpenAI image-host pattern.
- [ ] **P3 — Issue 1: Bump `s6` timeout** in `images-generate-and-download-v1` from `600000` → `1800000` ms as a stopgap.
- [ ] **P3 — Issue 1: Split** `images-generate-and-download-v1` into `images-submit-only-v1` (returns `chat_id`, exits) + `images-by-chat-id-v1` (polls + downloads). Re-evaluate whether `images-generate-and-download-v1` becomes a thin wrapper or is deprecated.
- [x] **P4 — Issue 3: Use dedicated tabs** across all `workflows/chatgpt/*.json`. Runtime supports dedicated workflow tabs; session continuation should use the workflow session handle.
- [ ] **Stretch — `images-by-chat-id-v1`** as the consolidated robust path (shadow-DOM pierce + network-log fallback + downloads). May replace `images-get-latest-v1` and `images-download-current-rendered-v1` if it proves equivalent or better.
- [ ] **Stretch — `images-submit-only-v1`** as the fire-and-forget submit path that pairs with `images-by-chat-id-v1`.
- [ ] **Validation:** Run end-to-end against repro chat `69f49357-94a8-83a6-b9e4-8871f8e24aa7` and against a fresh 7-variation generation. Per AGENTS.md rule 16, structural validation is necessary but not sufficient.

## What Works (Do Not Change)
- The `reddit-*` workflows work correctly under the same automation pipeline. Whatever pattern they use for tab handling is the pattern the chatgpt pack should match.
- `chat_id` extraction logic from `location.pathname` matching `/c/<uuid>` works when reached. Keep that — just surface it earlier and on failure paths.
- Generic `upload_file` step + ChatGPT-specific DOM prep (per `chatgpt_web_app_workflows`) is the right attachment pattern. Don't reimplement.
- Operator-supplied model/effort overrides (`model_slug`, `model_effort`) and the `Pro -> Extended` default. Don't touch.

## Tried & Didn't Work
- Relying on plain `<img>` tags + `naturalWidth*naturalHeight` heuristic on ChatGPT-Images turns: misses the current UI (zero matches even when 7 images are visually present).
- 10-min `wait_for_element` cap on multi-variation generations: too short for realistic 7-image runs (12–20 min observed). Caused unrecoverable state loss because `chat_id` was never surfaced.
- Active-tab dependence for unattended automation: any non-chatgpt focused tab breaks the run; concurrent agents collide. Fixed for shipped ChatGPT workflows by using dedicated workflow tabs.
- Asking the human "what's the chat URL?" as a recovery step: this is exactly the antipattern the chatgpt pack was meant to eliminate.
