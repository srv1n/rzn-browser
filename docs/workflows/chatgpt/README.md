---
title: ChatGPT Workflows
slug: /workflows/chatgpt/
sidebar:
  label: ChatGPT
---

# ChatGPT Workflows

The canonical surface is the workflow CLI:

```sh
rzn-browser run chatgpt <workflow> --param key="value"
```

If you are running from the repo without an installed binary, use `./target/debug/rzn-browser` instead of `rzn-browser`.

For a URL-or-UUID thread ref, pass the full URL directly in `chat_id`:

```sh
rzn-browser run chatgpt get-response-v1 --param chat_id="https://chatgpt.com/c/01234567-89ab-cdef-0123-456789abcdef"
```

## Thread Operations Today

| Capability | Workflow | Status | Notes |
| --- | --- | --- | --- |
| List recent chats | `chatgpt_recent_chats_v1.json` | Validated | Dedicated workflow-tab path in the authenticated Chrome profile |
| Fetch one thread by `chat_id` | `chatgpt_export_full_chat_v1.json` | Validated | Canonical read path for a known id |
| Export thread asset metadata by `chat_id` | `chatgpt_export_full_chat_v1.json` | Implemented | Full export now includes aggregated file/image metadata |
| Click a latest-assistant attachment by exact label | `chatgpt_download_attachment_v1.json` | Validated | Correct path for button-backed ChatGPT artifacts like `Markdown source` or `Zip package...` |
| Fetch visible thread only | `chatgpt_export_chat_v1.json` | Validated | Faster, visible DOM only |
| Read latest assistant response | `chatgpt_get_response_v1.json` | Implemented | Good for polling |
| Post into an existing thread by `chat_id` | `chatgpt_continue_chat_v1.json` | Implemented | Canonical write path |
| Accept bare UUID or full ChatGPT URL in `chat_id` | `rzn-browser run ... --param chat_id=...` | Validated | The binary normalizes `https://chatgpt.com/c/<id>` into the actual `chat_id` before workflow execution |
| Write local JSON/Markdown for a thread | `rzn-browser run chatgpt export-full-chat-v1 --param chat_id=...` | Implemented | Returns one structured payload (transcript + asset URLs); callers persist as JSON/Markdown |
| Select the newest assistant/user message | `rzn-browser run chatgpt export-full-chat-v1 --param chat_id=...` then take the last matching-role entry, or `rzn-browser run chatgpt get-response-v1 --param chat_id=...` for the latest assistant turn | Implemented | No dedicated `latest` subcommand — derive from the export payload |
| Materialize thread files/images locally | `rzn-browser run chatgpt download-attachment-v1 --param chat_id=... --param attachment_label=...` for button-backed artifacts; for image generations use `rzn-browser run chatgpt images-generate-and-download-v1` | Implemented | Browser handles the download; file lands in the active Chrome profile's Downloads folder (or the workflow's `download_folder` for image flows) |

## Notes

- The pack is session-aware and expects an authenticated ChatGPT session in the Chrome profile. It does not bind catalog workflows to the browser active tab.
- `chatgpt_recent_chats_v1.json` is the dedicated-tab discovery workflow used for inbox-style sync/export.
- `chatgpt_export_full_chat_v1.json` is the canonical single-thread read path.
- `chatgpt_export_full_chat_v1.json` now also returns aggregated thread assets under `assets.files` and `assets.images`.
- Some ChatGPT artifacts are button-backed rather than link-backed. `chatgpt_download_attachment_v1.json` clicks the exact attachment button inside the latest assistant turn instead of scanning page-level buttons.
- Full-thread and latest-message exports now carry both plain `text` and derived `markdown` per turn. The markdown is reconstructed from the rendered DOM, so it is structured and useful, but not guaranteed to be the model's exact original source.
- `chatgpt_continue_chat_v1.json` is the canonical single-thread write path for a known `chat_id`.
- Recent-thread sync, per-thread artifact persistence, last-message selection, and reply-refresh state are caller responsibilities. Compose them from `rzn-browser run chatgpt recent-chats-v1`, `export-full-chat-v1`, `get-response-v1`, and `continue-chat-v1` against the binary's stdout. There is no shared Python helper.
- Asset downloads are browser-driven: `chatgpt_download_attachment_v1.json` clicks button-backed artifacts and they land in the active Chrome profile's Downloads folder; `chatgpt_images_generate_and_download_v1.json` accepts a `download_folder` parameter.
- Transcript export is usable, but not perfectly deduplicated yet.

## Attachment Verification

Validated on April 16, 2026 against a real ChatGPT thread whose latest assistant turn exposed three artifact buttons.

| Attachment label | Browser download observed | Status |
| --- | --- | --- |
| `Self-contained HTML manual` | `game_design_operating_manual.html` | Validated |
| `Markdown source` | `game_design_operating_manual.md` | Validated |
| `Zip package with Markdown + extracted figure assets` | `game_design_sop_package.zip` | Validated |

Important details:

- These are three separate attachment actions.
- The zip is only the output of the third button.
- The zip overlaps with the first two because it bundles the HTML, the Markdown, and extracted assets together.
- Repeated clicks create uniquified downloads like `(1)`, `(2)`, and so on.
- The validated path is exact-label click inside the latest assistant turn. It does not rely on share or copy-link controls.
