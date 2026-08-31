# ChatGPT Workflows

Deterministic ChatGPT web-app workflows that reuse the authenticated Chrome session. Four workflows cover the full surface — one per purpose, no version suffixes.

```bash
rzn-browser run chatgpt <workflow> --param key="value"
```

If `rzn-browser` is not on `PATH`, use `./target/debug/rzn-browser` or `./target/release/rzn-browser`.

## Active Workflows

| Workflow | Purpose | Key Params |
| --- | --- | --- |
| `chatgpt_send.json` | **Single send path.** Opens required `entry_url` directly: saved chat URL for continuation, root or Project URL for a new chat. | required `message_text`, `entry_url`; optional `chat_id`, `project_id`, attachments, model guard params, `tool` |
| `chatgpt_read.json` | **Single read path.** Opens the stored `chat_url` directly; `mode=latest` returns just the last user→assistant exchange + a streaming flag, `mode=transcript` (default) returns user/assistant turns as clean markdown, `mode=full` returns every node with raw parts + metadata. Bundles all user-uploaded attachments into a single `.zip` download by default. | required `chat_id`, `chat_url`; optional `mode`, `include_system`, `download_attachments` |
| `chatgpt_close.json` | **Terminal cleanup.** Closes the one retained ChatGPT tab named by `--tab-ref`; it never falls back to the active tab. | required CLI `--tab-ref` |
| `chatgpt_projects.json` | **Projects discovery.** `mode=list` (default) returns every Project (`g-p-*`) with id, name, short_url, ready-to-use `project_url`, and recent conversation count; `mode=conversations` + `project_id` returns that project's chats (`chat_id`, `title`, `snippet`, timestamps). | optional `mode`, `project_id`, `limit` |
| `chatgpt_recent_chats.json` | List recent chats from local conversation-history cache + sidebar DOM. | optional `limit`, `days` |
| `chatgpt_images_download.json` | Walk a chat's mapping for `image_asset_pointer` parts and trigger browser downloads for each generated image. | `chat_id`; optional `download` |

## How They Fit Together

- **Send** anything (new chat, continued chat, with attachments, with tools): `chatgpt_send`. Set `tool=image_gen` to use the inline image generator.
- **Read** a chat: `chatgpt_read` with `mode=latest|transcript|full`. The envelope shape is identical across modes; only the contents of `messages[]` differ.
- **Discover** chats: `chatgpt_recent_chats`.
- **Projects**: `chatgpt_projects` (no params) lists every Project; `--param mode=conversations --param project_id=<g-p-...>` lists a project's chats. Reply inside a project with `chatgpt_send --param project_id=<g-p-...>` (new chat) or `--param chat_id=<id>` (continue a specific project thread — it already keeps its project). Read any returned `chat_id` with `chatgpt_read`.
- **Save generated images** locally: run `chatgpt_send --param tool=image_gen ...` then `chatgpt_images_download --param chat_id=<returned chat_id>`.

Projects are ChatGPT "gizmos" with id prefix `g-p-`. `chatgpt_projects` uses `/backend-api/gizmos/snorlax/sidebar` (list) and `/backend-api/gizmos/{id}/conversations` (per-project). `chatgpt_send`'s `project_id` accepts a bare `g-p-...` id, a `g-p-...-slug` short_url, or a full `https://chatgpt.com/g/.../project` URL; the returned payload includes `chat_id` and `project` (the project short_url) even though a project chat URL is `/g/<short>/c/<id>`.

All read workflows use `/api/auth/session` for the page session token, then `/backend-api/conversation/{chat_id}` for the JSON. `chatgpt_read` resolves each user attachment's signed `/backend-api/files/{id}/download` URL, `fetch`es the bytes in-page (`credentials: include`, so the session cookie authorizes them), packs them into one ZIP, and triggers a single browser download — no DOM scraping, no CDP, and immune to Chrome's "multiple automatic downloads" block. `chatgpt_images_download` still anchor-clicks per generated image.

## CLI Examples

```bash
# Discover
rzn-browser run chatgpt recent-chats --param limit="10" --param days="7"

# Read
rzn-browser run chatgpt read --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param chat_url="https://chatgpt.com/c/01234567-89ab-cdef-0123-456789abcdef" # transcript (default)
rzn-browser run chatgpt read --keep-tab-open --tab-ref "rzn://browser/chrome/tab/42" --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param chat_url="https://chatgpt.com/c/01234567-89ab-cdef-0123-456789abcdef" --param mode="latest" # reuse retained streaming tab
rzn-browser run chatgpt read --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param chat_url="https://chatgpt.com/c/01234567-89ab-cdef-0123-456789abcdef" --param mode="full"              # full mapping with metadata
rzn-browser run chatgpt read --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param chat_url="https://chatgpt.com/c/01234567-89ab-cdef-0123-456789abcdef" --param download_attachments=false
rzn-browser run chatgpt close --tab-ref "rzn://browser/chrome-instance/tab/42"                                        # exact retained-tab cleanup

# Send
rzn-browser run chatgpt send --param entry_url="https://chatgpt.com/" --param message_text="Summarize the last three commits"
rzn-browser run chatgpt send --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param entry_url="https://chatgpt.com/c/01234567-89ab-cdef-0123-456789abcdef" --param message_text="Now turn that into a checklist"
rzn-browser run chatgpt send --param entry_url="https://chatgpt.com/" --param message_text="Compare these" --param attachment_file_paths='["/Users/me/a.txt","/Users/me/b.txt"]'
rzn-browser run chatgpt send --param entry_url="https://chatgpt.com/" --param message_text="A watercolor skyline at dusk" --param tool="image_gen"

# Explicit model + effort (defaults are GPT-5.6 Sol / Medium)
rzn-browser run chatgpt send --param message_text="Reason about this carefully" --param model_slug="GPT-5.6 Sol" --param model_effort="Pro"
rzn-browser run chatgpt send --param message_text="Quick sanity check" --param model_slug="GPT-5.5" --param model_effort="Medium"

# Generate images then save them locally (chain)
chat_id=$(rzn-browser run chatgpt send --param message_text="A cinematic studio portrait of a fox astronaut" --param tool="image_gen" | jq -r '.chat_id')
rzn-browser run chatgpt images-download --param chat_id="$chat_id"

# Projects
rzn-browser run chatgpt projects                                                                                     # list all projects
rzn-browser run chatgpt projects --param mode="conversations" --param project_id="g-p-6a1c…" --param limit="20"       # one project's chats
rzn-browser run chatgpt send --param project_id="g-p-6a1c…" --param message_text="Kick off a new thread in this project"
```

`chat_url` is required and must be the stored full ChatGPT conversation URL; `chat_id` remains the bare conversation id used for the API and retained-tab match.

## Notes And Limits

- Active Chrome profile must already be authenticated to ChatGPT.
- `Too many requests` is a stop signal, not a transient selector failure. The
  send workflow fails with `CHATGPT_RATE_LIMITED`, and conversation API HTTP 429
  is surfaced unchanged. Do not immediately retry either result. Higher-level
  callers such as `chatgpt-handoff` must own the durable account-wide cooldown.
- To re-capture the picker markup after a future UI change, run a read-only DOM probe through rzn-browser itself (`rzn-browser run <probe>.json`) rather than working from screenshots — the Chrome bridge is the same one the workflow uses.
- **`model_slug` and `model_effort` are free-form labels.** They are matched against whatever the account actually offers, so nothing is hard-coded to one lane. Defaults are `GPT-5.6 Sol` / `Medium`. `model_version` is accepted and ignored; the version is part of the model label now.
- **Picker selection, verified live 2026-08-31.** The composer pill opens ONE flat panel, `[data-testid=composer-intelligence-picker-content]`. There is no `Advanced` entry and no submenu any more. The panel holds:
  - `div[role=menuitem][aria-label="Select model"]` — a view toggle. Its text is the current **effort**, not a model. Do not read a model off it.
  - `[data-testid=composer-model-picker-slider-simple-view]` containing `div[role=menuitem][aria-label=Power]` with a child `span[role=slider]` (`aria-valuenow` 0..4). This is the effort control.
  - `[data-testid=composer-model-picker-slider-advanced-view]` containing one `div[role=menuitemradio]` per model, each carrying `aria-checked`.
- **Effort is a slider driven by arrow keys, never a click.** `s9` focuses the `Power` row and sends `ArrowLeft`/`ArrowRight` until the tier matches. The keys commit the value on their own. **Never click that row**: a trusted click snaps the slider to the click coordinate and silently discards the selected tier. That exact bug produced `wanted ... / Pro; got ... / Extra High` for months.
- **The tier ladder is read off the page, never hardcoded.** `s9` walks the slider to its minimum and back up, reading the label the page prints in the simple view (`"<Tier>, N of 5."`). The slider exposes no `aria-valuetext`, so that caption is the only source. Observed ladder, low to high: `Instant`, `Medium`, `High`, `Extra High`, `Pro`. An account with a different set reports its own list in every error.
- **No CDP click touches the picker.** `s6` commits the model radio with an ordinary synthetic click and confirms `aria-checked` flipped. A trusted CDP click does **not** register on that radio, so the run would send on whatever model was already selected. `requires_cdp` is `false` for this workflow.
- **The model resets between page loads.** A fresh tab always comes up on the account default, so every run selects the model explicitly. Both `GPT-5.6 Sol` and `GPT-5.5` are selectable; neither is pinned.
- **While the panel is open the pill reads `Thinking effort`.** It shows the selected tier only after the panel closes. Verification therefore reads the tier from the panel caption, never from the pill.
- **Chat vs Work surface.** `s5-surface` selects the `Chat` surface when ChatGPT shows the `Chat` / `Work` switch (`button[role=radio]` with `aria-checked`, inside a `[role=radiogroup]`). A plain click flips it; the URL does not change. Without this the run can compose on the `Work` surface.
- **Hard-fail on bad commit.** Before typing the prompt, `s12` reopens the panel and reads the checked model radio and the slider tier back. A mismatch throws `model_selection_verify_failed` instead of sending under another lane; pass `require_exact_model=false` to send anyway and have the applied values reported.
- **Missing option errors name the alternatives.** A tier or model your plan no longer exposes fails as e.g. `effort_not_found: wanted Pro; available=["Instant","Medium","High"]`, so the fix is visible from the error alone. A slider that ignores the arrow keys fails as `effort_slider_unresponsive` rather than passing silently.
- `node workflows/chatgpt/chatgpt_send_picker.test.mjs` exercises the picker steps against a fake DOM that reproduces the live panel (model radios plus the effort slider), with no browser or network. It also asserts that no `click_element` step targets the model radio or the slider row — the two regressions that broke this workflow before.
- Multi-file upload uses ChatGPT's existing `#upload-files` input directly (it has `multiple=true`).
- If a tool is missing from the top-level `+` menu, `chatgpt_send` auto-expands the **More** submenu before failing.
- ChatGPT is current-tab only in the validated runtime path.
- `--keep-tab-open` releases workflow ownership without closing the dedicated tab.
  A later `--tab-ref` targets that exact tab. `chatgpt_read` skips its navigation
  step when the tab URL already contains `/c/<chat_id>`, avoiding a streaming-page
  reload. If Chrome Memory Saver discarded that matching tab, the extension reloads
  and content-readies that same tab before continuing. An exact tab that is missing
  returns `TAB_MISSING` so the handoff adapter can decide whether to open a replacement.
  Use `chatgpt close` with that same `--tab-ref` for idempotent cleanup; it has no
  active-tab fallback.
- `chatgpt_read` writes one `chatgpt-attachments-<chat_id>.zip` to the browser default Downloads folder; the result payload also carries `attachment_urls` (cookie-bound signed URLs) and `attachments_zip` (name, file_count, size_bytes, errors). Generated-image downloads land as individual files in the Downloads folder.
