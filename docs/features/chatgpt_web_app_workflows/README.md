# ChatGPT Web App Workflows

## Overview
- Goal: Add a deterministic `workflows/chatgpt` pack for the ChatGPT web app that covers the operator actions we actually need: start a fresh chat and send a prompt on GPT-5.6 Sol with Pro intelligence, upload a file/image into a fresh chat, reopen a chat and wait for the latest assistant response, continue an existing chat, export the visible transcript, export full-thread transcript plus file/image asset metadata, and run an Images-specific flow that can generate two variations, poll for completion, and hand local downloads off to a thin wrapper.
- Constraints: The ChatGPT web UI is a fast-moving SPA, the useful path depends on an already-authenticated Chrome session, model-picker and attachment markup are unstable, image URLs may only be knowable after the latest assistant turn stabilizes, and the feature must stay workflow-level rather than introducing `chatgpt.com` rules into shared executor code.

## Flow Diagrams
- End-to-end flow
```text
CLI run
  -> supervisor
  -> native host
  -> extension background
  -> content script / main-world JS
  -> chatgpt.com SPA
  <- extracted state / transcript / chat_id
```

- Workflow surface
```text
chatgpt_new_chat_send_v1
  -> open ChatGPT home
  -> normalize to a fresh composer
  -> default/select model and effort
  -> type + send
  -> return chat_id from /c/<id>

chatgpt_new_chat_send_attachment_v1
  -> open ChatGPT home
  -> normalize to a fresh composer
  -> default/select model and effort
  -> reveal file input + upload local file
  -> type + send
  -> return chat_id from /c/<id>

chatgpt_get_response_v1
  -> open /c/<id>
  -> wait for latest assistant turn to stabilize
  -> return latest assistant response + thread metadata

chatgpt_continue_chat_v1
  -> open /c/<id>
  -> optionally choose model
  -> type + send
  -> return post-send state

chatgpt_export_chat_v1
  -> open /c/<id>
  -> extract visible transcript turns
  -> return structured role/content payload

chatgpt_images_new_generation_v1
  -> open /images
  -> ensure composer is ready
  -> send prompt with a default "exactly 2 variations" suffix when needed
  -> return chat_id

chatgpt_images_new_generation_attachment_v1
  -> open /images
  -> prepare upload input
  -> upload one file/image
  -> send prompt with a default "exactly 2 variations" suffix when needed
  -> return chat_id

chatgpt_images_get_latest_v1
  -> open /c/<id>
  -> inspect latest assistant turn
  -> extract image URLs + readiness

chatgpt_images_generate_and_download_v1
  -> rzn-browser run chatgpt images-generate-and-download-v1 --param message_text=... [--param attachment_file_path=...] [--param variation_count=...] [--param download_folder=...]
  -> waits for rendered image URLs to stabilize, then downloads them through chrome.downloads
```

- Internal state machine
```text
load route
  -> auth/session present?
    -> no: fail with surface-not-ready signal
    -> yes: find composer or thread
      -> enforce GPT-5.6 Sol in the model submenu
      -> enforce Pro in the Intelligence menu
      -> requested attachment?
        -> no: continue
        -> yes: reveal file input -> upload via generic upload_file step
      -> send path or read path
        -> send: set composer text -> trigger send -> wait for /c/<id> or response surface
        -> read: collect turns -> poll until latest assistant turn is stable
```

## Decision Record
- Keep the implementation in workflow JSON. ChatGPT selectors are site-specific and unstable, so the right place for that logic is `execute_javascript` inside the pack, not shared Rust or extension heuristics.
- Use explicit dedicated workflow tabs. These flows reuse the operator's logged-in Chrome profile without stealing the browser's active tab.
- Lock send flows to GPT-5.6 Sol with Pro intelligence. Conflicting overrides fail closed.
- Keep model and effort selection in the workflow. The picker is site-specific and changes often, so the right place for that traversal is the ChatGPT pack rather than generic engine code.
- Use the generic `upload_file` step for attachments after a ChatGPT-specific prep script stamps the live file input with a stable temporary selector.
- Return `chat_id` as the stable handle. That gives downstream callers a concrete key for polling, continuation, and transcript export.
- Keep ChatGPT Images downloads outside the browser workflow. Chrome's downloads API writes into browser-managed locations, so caller-controlled output dirs and explicit file names belong in a local wrapper, not in shared browser runtime code.

## Architecture
- Modules:
  - `workflows/chatgpt/chatgpt_new_chat_send_v1.json`: Fresh-chat entry point that selects a model/effort policy and sends the initial prompt.
  - `workflows/chatgpt/chatgpt_new_chat_send_attachment_v1.json`: Fresh-chat entry point that uploads one local file or image before sending the initial prompt.
  - `workflows/chatgpt/chatgpt_get_response_v1.json`: Thread reader that waits for the latest assistant turn to settle.
  - `workflows/chatgpt/chatgpt_continue_chat_v1.json`: Existing-thread send path.
  - `workflows/chatgpt/chatgpt_export_chat_v1.json`: Visible transcript exporter.
  - `workflows/chatgpt/chatgpt_export_full_chat_v1.json`: Canonical full-thread exporter that also aggregates file/image asset metadata for the thread.
  - `workflows/chatgpt/chatgpt_images_new_generation_v1.json`: Images entry point that sends a prompt on `chatgpt.com/images` and returns the resulting `chat_id`.
  - `workflows/chatgpt/chatgpt_images_new_generation_attachment_v1.json`: Images entry point that uploads one local file/image before sending the prompt.
  - `workflows/chatgpt/chatgpt_images_get_latest_v1.json`: Poll/read path that extracts the latest assistant-turn image URLs and readiness state.
  - `workflows/chatgpt/chatgpt_images_generate_and_download_v1.json`: Single-shot binary workflow that starts generation, waits until the rendered image set stabilizes, and downloads the final URLs into a caller-specified `download_folder`.
  - `workflows/chatgpt/chatgpt_images_download_current_rendered_v1.json`: Session-tab helper that downloads an already visible rendered image from a workflow-owned tab.
  - `workflows/chatgpt/README.md`: Operator-facing pack overview and run examples.
  - `docs/workflows/chatgpt/*.md`: Docs-site workflow pages.
- Data contracts:
  - `chat_id`: extracted from `location.pathname` when the route matches `/c/<id>`.
  - `model_slug`: optional exact guard; only `GPT-5.6 Sol` is accepted and it is the default.
  - `model_version`: optional exact guard; only `5.6` is accepted and it is the default.
  - `model_effort`: optional intelligence guard; only `Pro` is accepted and it is the default.
  - `require_exact_model`: optional guard that defaults to true; false is rejected.
  - `message_text`: prompt body sent into the composer.
  - `attachment_file_path`: absolute local path to a single file/image for the attachment flow.
  - `variation_count`: optional desired image count for the Images flows. The wrapper defaults this to `2`.
  - `expected_image_count`: optional read-side count used to decide whether the latest assistant turn is ready.
  - `output_dir`: wrapper-only local destination that defaults to the current working directory.
  - `file_names`: wrapper-only optional explicit output names that must align with the requested image count.
  - Read flows return compact JSON objects with `url`, `title`, `chat_id`, `latest_assistant_response`, `message_count`, and transcript arrays where applicable.

## Implementation Notes
- Entry points:
  - CLI/native runner parameter substitution injects variables into workflow JSON and exposes script params via `window.__rzn_params`.
  - Main-world `execute_javascript` handles the ChatGPT-specific DOM traversal and extraction.
- Key calls and event flow:
  - The send flow normalizes the page into a composer-ready state, selects GPT-5.6 Sol in the nested model submenu, selects Pro in the Intelligence menu, verifies both, populates the prompt box via native setters, and triggers send from the nearest enabled submit control.
- **Model + intelligence commits run through CDP `Input.dispatchMouseEvent`, not synthetic clicks.** Both controlled radio groups gate state on trusted input. `chatgpt_send.json` stamps the GPT-5.6 Sol and Pro radios, commits each with `click_element` and `inputs.use_cdp:true`, then reopens both menus and fails on any mismatch.
- **Visible labels are the contract.** The workflow matches `GPT-5.6 Sol` in the model submenu and `Pro` in Intelligence rather than depending on versioned testids.
  - The attachment flow uses a prep script to find or reveal ChatGPT's file input, stamps it with `#rzn-chatgpt-upload-input`, then uses the generic `upload_file` step to set the local file path before sending.
- The read flows derive turn containers from `data-message-author-role` first, then fall back to broader conversation-turn/article selectors.
- The full-thread exporter also records per-turn asset references plus top-level aggregated `assets.files` and `assets.images`, which gives downstream helpers a deterministic surface for local downloads.
  - The latest-response reader polls until the newest assistant turn is present, the text has stabilized across multiple passes, and no visible stop-generation control remains.
  - The Images send flows stay separate from the model-picker flows and target `https://chatgpt.com/images/`, adding a default "Generate exactly 2 distinct image variations" suffix only when the operator prompt does not already specify a variation count.
  - The Images poll flow inspects the latest assistant turn, filters out avatar/icon-like images, and reports `ready`, `status`, and extracted image URLs without attempting local file writes inside the browser runtime.
  - The local wrapper waits for two consecutive ready polls with the same URL fingerprint before downloading, which reduces the chance of grabbing a mid-stream asset URL set.
- Error handling and retries:
  - Missing authenticated session surfaces fail as "composer/thread surface not found" instead of silently succeeding.
  - Model selection throws only when a model override was explicitly requested.
  - Transcript export is intentionally limited to the visible DOM; long virtualized threads may require future scroll pagination if the current surface truncates older turns.
  - Direct local download can still fail if ChatGPT changes its asset URL contract; keep that responsibility in the wrapper so fallback strategies stay local and do not leak into the shared browser engine.

## Tasks & Status
- [x] Define the four-operation ChatGPT workflow surface
- [x] Add `workflows/chatgpt` JSON workflows
- [x] Add pack docs and a feature scratchpad
- [x] Lock ChatGPT send flows to GPT-5.6 Sol / Pro
- [x] Add a fresh-chat attachment workflow for local files/images
- [x] Add ChatGPT Images start, attachment-start, and poll workflows
- [x] Add a local ChatGPT Images wrapper for polling + downloads + caller-controlled file names
- [ ] Validate against a real logged-in ChatGPT session in Chrome
- [ ] Tune model-picker and attachment heuristics against the live authenticated DOM as the UI drifts
- [ ] Validate the Images turn and asset extraction heuristics against the live authenticated `chatgpt.com/images` DOM
- [ ] Add a browser-click fallback for full-thread asset downloads when ChatGPT hides a document behind a button instead of a direct URL
- [ ] Land the automation-blocker fixes tracked in `docs/features/chatgpt_workflows_automation_fixes/README.md` (always-return `chat_id`, ChatGPT-Images DOM/network fallbacks, dedicated-tab spawn, optional submit-only + by-chat-id split)

## What Works (Do Not Change)
- Keep ChatGPT-specific DOM logic inside the workflow pack.
- Keep the primary handle as `chat_id`, not a transient DOM selector.
- Keep session-owned workflow tabs so the pack reuses the operator's authenticated Chrome profile without stealing the active tab.
- Keep file-upload transport generic. Only the DOM prep for ChatGPT belongs in this pack.
- Keep local download naming and cwd-relative output logic in the wrapper script, not in the extension runtime.

## Tried & Didn't Work
- External Playwright probing: wrong validation loop for this repository and not representative of the intended extension/native-host path.
- Single-selector model targeting: too brittle for a UI that frequently renames or restructures picker controls.
- Browser-managed downloads for final ChatGPT Images assets: wrong ownership boundary for caller-controlled output directories and optional explicit file names.
