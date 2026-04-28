# ChatGPT Workflows

This pack provides deterministic ChatGPT web-app workflows that reuse the authenticated Chrome session.

The first-class surface is the workflow CLI:

- `rzn-browser run chatgpt <workflow> --param key="value"`
- If `rzn-browser` is not on `PATH`, use `./target/debug/rzn-browser` or `./target/release/rzn-browser`.
- Helper scripts still exist for repo-local artifact and index management, but the workflow CLI is the canonical operator interface.

## What Works Today

| Capability | Primary Surface | Canonical Workflow / Helper | Live Status | Notes |
| --- | --- | --- | --- | --- |
| List recent chats | CLI workflow | `chatgpt_recent_chats_v1.json` | Validated | Current-tab only in the validated Chrome session. Returns normalized `thread_id` / `chat_id` entries. |
| Fetch one thread by `chat_id` | CLI workflow | `chatgpt_export_full_chat_v1.json` | Validated | Canonical read path for a known thread id. |
| Export thread asset metadata by `chat_id` | CLI workflow | `chatgpt_export_full_chat_v1.json` | Implemented | Full export now includes aggregated file/image metadata for the thread. |
| Click a latest-assistant attachment by exact label | CLI workflow | `chatgpt_download_attachment_v1.json` | Validated | Correct path for button-backed ChatGPT artifacts like `Markdown source` or `Zip package...`. |
| Fetch visible thread content by `chat_id` | CLI workflow | `chatgpt_export_chat_v1.json` | Validated | Faster, but limited to the currently loaded DOM. |
| Read the latest assistant response by `chat_id` | CLI workflow | `chatgpt_get_response_v1.json` | Implemented | Good polling surface when only the newest assistant output matters. |
| Post a new message into a known thread by `chat_id` | CLI workflow | `chatgpt_continue_chat_v1.json` | Implemented | Canonical write path. I did not auto-post into a real user thread during validation. |
| Sync recent chats into local JSON/Markdown/JSONL/index state | Helper layer | `assistant sync helper (sync)` | Implemented | ChatGPT is hard-limited to sequential sync today. |
| Fetch one thread into local JSON/Markdown/JSONL/index state | Helper layer | `assistant sync helper (fetch)` | Implemented | Wraps `chatgpt_export_full_chat_v1.json`. |
| Materialize exported thread files/images locally | Helper layer | `assistant sync helper (fetch + download-assets)` | Implemented | Best-effort direct download of exported file/image URLs into `output/assistants/chatgpt/thread_assets/<chat_id>/`. |
| Select the newest assistant/user/any message and refresh local state | Helper layer | `assistant sync helper (latest)` | Implemented | Uses the canonical full-thread export, then selects the newest matching turn. |
| Reply and refresh local JSON/Markdown/index state | Helper layer | `assistant sync helper (reply)` | Implemented | Wraps `chatgpt_continue_chat_v1.json` then re-exports. |
| Parallel multi-thread ChatGPT sync | Not supported | N/A | Rejected | Dedicated/background-tab ChatGPT runs were unstable in this session. |
| Perfect transcript deduplication | Not yet | N/A | Partial | Export works, but some transient assistant wrapper turns still duplicate. |

## Thread Operations

| Workflow | Purpose | Key Params |
| --- | --- | --- |
| `chatgpt_recent_chats_v1.json` | Open ChatGPT in the current tab and return the recent chat list. | `limit` (optional), `days` (optional) |
| `chatgpt_export_full_chat_v1.json` | Open an existing chat in the current tab, scroll toward the top, and export the transcript. | `chat_id` |
| `chatgpt_download_attachment_v1.json` | Open an existing chat, scope to the latest assistant turn, and click an attachment-like button by exact label. | `chat_id`, `attachment_label` |
| `chatgpt_export_chat_v1.json` | Open an existing chat and extract only the visible transcript. | `chat_id` |
| `chatgpt_get_response_v1.json` | Open an existing chat by `chat_id`, wait for the latest assistant turn to stabilize, and return the latest response payload. | `chat_id` |
| `chatgpt_continue_chat_v1.json` | Open an existing chat, send another prompt, and return post-send thread state. | `chat_id`, `message_text`, `model_slug` (optional), `model_effort` (optional) |

Export/read shape:

- `chatgpt_export_full_chat_v1.json` returns `transcript[].text`, `transcript[].markdown`, `transcript[].links`, `transcript[].assets`, and aggregated top-level `assets`.
- `chatgpt_get_response_v1.json` returns the same richer turn shape for `latest_assistant_response` and `latest_user_message` when the DOM exposes it.
- Attachment capture is DOM-driven today. Some ChatGPT artifacts are real links/images; others are button-backed file actions in the latest assistant turn. `chatgpt_download_attachment_v1.json` handles the button-backed lane.

## Attachment Verification

Validated on April 16, 2026 against a real ChatGPT thread with three attachment buttons in the latest assistant turn:

| Attachment label | Resulting browser download | Status |
| --- | --- | --- |
| `Self-contained HTML manual` | `game_design_operating_manual.html` | Validated |
| `Markdown source` | `game_design_operating_manual.md` | Validated |
| `Zip package with Markdown + extracted figure assets` | `game_design_sop_package.zip` | Validated |

Important detail:

- Those are three separate attachment actions, not three labels that all collapse into one zip.
- The zip overlaps with the first two artifacts because it bundles the HTML, the Markdown, and extracted assets together.
- Repeated clicks create uniquified browser downloads like `game_design_sop_package (1).zip`, `game_design_operating_manual (1).md`, and `game_design_operating_manual (1).html`.
- The validated click contract is "latest assistant turn + exact visible label". It does not rely on share menus, copy-link controls, or page-wide button fishing.

## Other ChatGPT Workflows

| Workflow | Purpose | Key Params |
| --- | --- | --- |
| `chatgpt_new_chat_send_v1.json` | Open ChatGPT, normalize to a fresh chat, default to `Pro` with `Extended` effort unless overridden, send the first prompt, and return the new `chat_id`. | `message_text`, `model_slug` (optional), `model_effort` (optional) |
| `chatgpt_new_chat_send_attachment_v1.json` | Open ChatGPT, normalize to a fresh chat, upload one local file or image, send the first prompt, and return the new `chat_id`. | `message_text`, `attachment_file_path`, `model_slug` (optional), `model_effort` (optional) |
| `chatgpt_send_current_composer_v1.json` | Reuse the current ChatGPT tab without navigating, optionally verify an already attached file/image, send the prompt from the visible composer, and return the resolved `chat_id`. | `message_text`, `attachment_file_path` (optional) |
| `chatgpt_images_new_generation_v1.json` | Open `chatgpt.com/images`, send an image-generation prompt, default to 2 variations unless already specified, and return the new `chat_id`. | `message_text`, `variation_count` (optional) |
| `chatgpt_images_new_generation_attachment_v1.json` | Open `chatgpt.com/images`, upload one local file/image, send an image-generation prompt, default to 2 variations unless already specified, and return the new `chat_id`. | `message_text`, `attachment_file_path`, `variation_count` (optional) |
| `chatgpt_images_get_latest_v1.json` | Open an existing image-generation chat, inspect the latest assistant turn for image URLs, and report whether the set is ready. | `chat_id`, `expected_image_count` (optional) |
| `chatgpt_images_generate_and_download_v1.json` | Open `chatgpt.com/images`, send an image-generation prompt, wait for the rendered result, and trigger browser downloads into a named Downloads subfolder. | `message_text`, `download_folder`, `variation_count` (optional) |

## CLI-First Usage

Start from an authenticated ChatGPT browser session in Chrome.

List recent chats:

```bash
rzn-browser run chatgpt recent-chats-v1 --param limit="10" --param days="7"
```

Fetch a full thread by `chat_id`:

```bash
rzn-browser run chatgpt export-full-chat-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef"
```

That export already includes transcript turns plus aggregated `assets.files` and `assets.images`.

Fetch the visible portion of a thread by `chat_id`:

```bash
rzn-browser run chatgpt export-chat-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef"
```

Read only the latest assistant response:

```bash
rzn-browser run chatgpt get-response-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef"
```

Read the latest assistant response by pasting a full ChatGPT thread URL:

```bash
rzn-browser run chatgpt get-response-v1 --param chat_id="https://chatgpt.com/c/01234567-89ab-cdef-0123-456789abcdef"
```

Click a known latest-assistant attachment by exact label:

```bash
rzn-browser run chatgpt download-attachment-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param attachment_label="Markdown source"
```

Post a new message into a known thread:

```bash
rzn-browser run chatgpt continue-chat-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param message_text="Now turn that into a checklist"
```

Export a full thread by passing either a UUID or a full URL:

```bash
rzn-browser run chatgpt export-full-chat-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef"
rzn-browser run chatgpt export-full-chat-v1 --param chat_id="https://chatgpt.com/c/01234567-89ab-cdef-0123-456789abcdef"
```

Create a new chat and send the first prompt:

```bash
rzn-browser run chatgpt new-chat-send-v1 --param message_text="Summarize the last three commits"
```

Reuse the current visible composer after you have already prepared it:

```bash
rzn-browser run chatgpt send-current-composer-v1 --param message_text="Continue from the current draft"
```

Generate two ChatGPT Images variations:

```bash
rzn-browser run chatgpt images-new-generation-v1 --param message_text="A watercolor skyline at dusk" --param variation_count="2"
```

Generate two ChatGPT Images variations and trigger browser downloads:

```bash
rzn-browser run chatgpt images-generate-and-download-v1 --param message_text="A cinematic studio portrait of a fox astronaut" --param download_folder="chatgpt_images_fox" --param variation_count="2"
```

## Helper Layer

Helper scripts still own repo-local side effects such as `index.json`, per-thread `json|md|jsonl` sidecars, and caller-chosen download paths. The raw CLI surfaces underneath are `recent-chats-v1`, `export-full-chat-v1`, `get-response-v1`, `continue-chat-v1`, `download-attachment-v1`, `images-new-generation-v1`, and `images-generate-and-download-v1`.

## Notes And Limits

- These workflows assume the active Chrome profile is already authenticated to ChatGPT.
- The send flows default to `Pro` with `Extended` effort unless you override `model_slug` and/or `model_effort`.
- For a known `chat_id`, the canonical read path is `chatgpt_export_full_chat_v1.json`, and the canonical write path is `chatgpt_continue_chat_v1.json`.
- `rzn-browser run ... --param chat_id=...` now accepts either a bare ChatGPT UUID or a full `https://chatgpt.com/c/<id>` URL. The binary normalizes the URL before workflow execution.
- `chatgpt_recent_chats_v1.json` uses the local conversation-history cache first and merges sidebar DOM data when available.
- `chatgpt_export_full_chat_v1.json` is the right export surface for thread reads because it attempts upward scrolling before extracting the transcript.
- `chatgpt_export_full_chat_v1.json` now also returns top-level `assets.files` and `assets.images` for the thread, aggregated across exported turns.
- `chatgpt_download_attachment_v1.json` was live-validated against all three attachment labels in one real ChatGPT thread: HTML -> `.html`, Markdown -> `.md`, Zip bundle -> `.zip`.
- Recent-chat sync, per-thread export, and continue-chat reply are all first-class binary commands: `rzn-browser run chatgpt recent-chats-v1`, `rzn-browser run chatgpt export-full-chat-v1 --param chat_id=...`, and `rzn-browser run chatgpt continue-chat-v1 --param chat_id=... --param message_text=...`. There is no separate Python helper layer.
- For "just give me the last useful message", run `rzn-browser run chatgpt export-full-chat-v1 --param chat_id=...` and take the final entry of the matching role from the returned transcript.
- Output paths default to the browser Downloads folder for asset workflows; callers redirect by post-processing the returned payload.
- Download completion is still verified externally today via browser download side effects. The workflow itself proves the click path, not a first-class downloads manifest yet.
- ChatGPT is current-tab only in the validated runtime path for this session. Dedicated/background-tab ChatGPT runs were unstable.
- Transcript export is good enough to use, not perfect. Some transient assistant wrapper turns can still duplicate.
- The model picker is still treated as workflow-level UI traversal. If ChatGPT renames or moves those controls, fix the workflow pack rather than the shared engine.
- The pack intentionally avoids engine changes. Any ChatGPT DOM drift should be fixed here first.
