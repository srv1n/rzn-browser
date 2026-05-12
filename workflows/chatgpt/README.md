# ChatGPT Workflows

Deterministic ChatGPT web-app workflows that reuse the authenticated Chrome session. Four workflows cover the full surface — one per purpose, no version suffixes.

```bash
rzn-browser run chatgpt <workflow> --param key="value"
```

If `rzn-browser` is not on `PATH`, use `./target/debug/rzn-browser` or `./target/release/rzn-browser`.

## Active Workflows

| Workflow | Purpose | Key Params |
| --- | --- | --- |
| `chatgpt_send.json` | **Single send path.** Continues a thread when `chat_id` is set, starts a fresh chat otherwise. Supports 0..N attachments, model + thinking-effort selection, and a tool toggle (`search` / `deep_research` / `image_gen` / `canvas` / `agent`). | `message_text`; optional `chat_id`, `attachment_file_paths`, `model_slug`, `model_effort`, `tool` |
| `chatgpt_read.json` | **Single read path.** `mode=latest` returns just the last user→assistant exchange + a streaming flag, `mode=transcript` (default) returns user/assistant turns as clean markdown, `mode=full` returns every node with raw parts + metadata. Auto-downloads user-uploaded attachments by default. | `chat_id`; optional `mode`, `include_system`, `download_attachments` |
| `chatgpt_recent_chats.json` | List recent chats from local conversation-history cache + sidebar DOM. | optional `limit`, `days` |
| `chatgpt_images_download.json` | Walk a chat's mapping for `image_asset_pointer` parts and trigger browser downloads for each generated image. | `chat_id`; optional `download` |

## How They Fit Together

- **Send** anything (new chat, continued chat, with attachments, with tools): `chatgpt_send`. Set `tool=image_gen` to use the inline image generator.
- **Read** a chat: `chatgpt_read` with `mode=latest|transcript|full`. The envelope shape is identical across modes; only the contents of `messages[]` differ.
- **Discover** chats: `chatgpt_recent_chats`.
- **Save generated images** locally: run `chatgpt_send --param tool=image_gen ...` then `chatgpt_images_download --param chat_id=<returned chat_id>`.

All read workflows use `/api/auth/session` for the page session token, then `/backend-api/conversation/{chat_id}` for the JSON. Attachment / image downloads use the backend file-download API and trigger a browser anchor-click on the returned signed CloudFront URL — no DOM scraping, no CDP.

## CLI Examples

```bash
# Discover
rzn-browser run chatgpt recent-chats --param limit="10" --param days="7"

# Read
rzn-browser run chatgpt read --param chat_id="01234567-89ab-cdef-0123-456789abcdef"                                  # transcript (default)
rzn-browser run chatgpt read --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param mode="latest"            # last exchange + streaming flag
rzn-browser run chatgpt read --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param mode="full"              # full mapping with metadata
rzn-browser run chatgpt read --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param download_attachments=false

# Send
rzn-browser run chatgpt send --param message_text="Summarize the last three commits"
rzn-browser run chatgpt send --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param message_text="Now turn that into a checklist"
rzn-browser run chatgpt send --param message_text="Compare these" --param attachment_file_paths='["/Users/me/a.txt","/Users/me/b.txt"]'
rzn-browser run chatgpt send --param message_text="A watercolor skyline at dusk" --param tool="image_gen"

# Pin model + thinking effort explicitly (defaults: Pro / Extended)
rzn-browser run chatgpt send --param message_text="Reason about this carefully" --param model_slug="Thinking" --param model_effort="Heavy"
rzn-browser run chatgpt send --param message_text="Quick reply" --param model_slug="Instant"
rzn-browser run chatgpt send --param message_text="Cheaper Pro pass" --param model_slug="Pro" --param model_effort="Standard"
rzn-browser run chatgpt send --param message_text="Same as default but explicit" --param model_slug="pro-extended"

# Generate images then save them locally (chain)
chat_id=$(rzn-browser run chatgpt send --param message_text="A cinematic studio portrait of a fox astronaut" --param tool="image_gen" | jq -r '.chat_id')
rzn-browser run chatgpt images-download --param chat_id="$chat_id"
```

`--param chat_id=...` accepts either a bare ChatGPT UUID or a full `https://chatgpt.com/c/<id>` URL.

## Notes And Limits

- Active Chrome profile must already be authenticated to ChatGPT.
- `chatgpt_send` defaults to `Pro` with `Extended` effort unless `model_slug` / `model_effort` are passed.
- **Model + effort selection.** Accepted models: `Pro`, `Thinking`, `Instant`. Pro effort: `Standard` (default) or `Extended`. Thinking effort: `Light`, `Standard`, `Extended` (default), `Heavy`. Instant has no effort. Aliases work too — `pro-extended` / `extended-pro` (= Pro+Extended), `heavy` / `thinking-heavy` (= Thinking+Heavy), `fast` (= Instant). Versioned testids (`gpt-5-5-*` today) are matched by visible label so the workflow survives `5.5 → 5.6` renames.
- **Hard-fail on bad effort commit.** ChatGPT's effort radio is a controlled radix RadioGroup gated on `isTrusted=true`; synthetic DOM clicks do not commit. `chatgpt_send` uses `click_element` with `use_cdp: true` (CDP `Input.dispatchMouseEvent`) for the model and effort commits, then reopens the menu and verifies. If the selection did not stick (e.g. silent fallback Pro Extended → Pro Standard), the workflow throws `model_selection_verify_failed: wanted <model> / <effort>; got <actual> / <actual>` instead of sending under the wrong model.
- Multi-file upload uses ChatGPT's existing `#upload-files` input directly (it has `multiple=true`).
- If a tool is missing from the top-level `+` menu, `chatgpt_send` auto-expands the **More** submenu before failing.
- ChatGPT is current-tab only in the validated runtime path.
- Attachment / image downloads land in the browser default Downloads folder; redirect by post-processing the returned payload.

## Old / Archived

Earlier versions of every workflow live in `archive/workflows/chatgpt/` for reference. They are NOT discovered by the workflow runner. Each was retired because a single canonical workflow now covers its purpose:

| Archived | Replaced by |
| --- | --- |
| `chatgpt_new_chat_send_v1`, `chatgpt_new_chat_send_attachment_v1`, `chatgpt_continue_chat_v1`, `chatgpt_send_current_composer_v1` (+ `_js_v1`) | `chatgpt_send` |
| `chatgpt_export_chat`, `chatgpt_export_chat_v1` | `chatgpt_read --param mode="transcript"` |
| `chatgpt_export_full_chat`, `chatgpt_export_full_chat_v1` | `chatgpt_read --param mode="full"` |
| `chatgpt_get_response`, `chatgpt_get_response_v1` | `chatgpt_read --param mode="latest"` |
| `chatgpt_images_get_latest_v1`, `chatgpt_images_download_current_rendered_v1` | `chatgpt_images_download` |
| `chatgpt_images_new_generation_v1`, `chatgpt_images_new_generation_attachment_v1`, `chatgpt_images_generate_and_download_v1` | `chatgpt_send --param tool="image_gen"` (+ `chatgpt_images_download` to save locally) |
| `chatgpt_download_attachment_v1` | `chatgpt_read` auto-downloads user attachments. For rare button-backed artifacts not in the API payload, restore from archive on demand. |
