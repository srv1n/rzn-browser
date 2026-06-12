# Claude Workflows

Deterministic `claude.ai` workflows. Three workflows — one per distinct capability. Everything that differs only in options (new vs reply, with vs without attachment, full transcript vs latest reply, which model, thinking on/off) is a parameter on an existing workflow, not a new file.

All flows run through the native CLI:

```bash
rzn-browser run claude <workflow> --param key="value"
```

If `rzn-browser` is not on `PATH`, use `./target/release/rzn-browser`.

## Workflows

| Workflow | Purpose | Side effect |
| --- | --- | --- |
| `recent-chats` | List threads from the sidebar. | Read |
| `export-chat` | Read one thread. `mode` picks full transcript, latest assistant turn, or latest user turn. `download_artifacts="true"` also clicks every artifact Download button (files land in the browser's default Downloads folder). | Read (+ side-effect downloads when `download_artifacts="true"`) |
| `send` | Send a message. Optional `thread_id` (reply vs new chat), `attachment_file_paths`, `model_slug`, `adaptive_thinking`. | Write |

## `send` parameter reference

| Param | Required | Shape | Behavior |
| --- | --- | --- | --- |
| `message_text` | yes | text | Prompt to send. |
| `thread_id` | no | string id | If set, sends into `/chat/<thread_id>`. If not, starts a new chat. |
| `attachment_file_paths` | no | path, comma-separated paths, or JSON list `[\"/a\",\"/b\"]` | Files to upload before sending. Empty = no upload (no-op). |
| `model_slug` | no | string | Selects from the model picker by visible label (`"Opus 4.7"`, `"Sonnet 4.6"`, etc). Opens the "More models" submenu if the label isn't in the main menu. |
| `adaptive_thinking` | no | `on` \| `off` | Toggles Adaptive thinking. Leave unset to keep current state. |

## Why these three

- `export-chat` replaces any "get latest response" surface via `mode=latest-assistant|latest-user`.
- `send` replaces new-chat, reply-chat, and attachment variants. Thread routing, attachments, and model choice all ride on the same workflow.
- Workflow IDs don't carry a version suffix — the `version` field inside each JSON is the source of truth for semver.

## Examples

```bash
# List the last 10 threads
rzn-browser run claude recent-chats --param limit="10"

# Read one thread end-to-end
rzn-browser run claude export-chat --param thread_id="..."

# Get just the latest Claude reply
rzn-browser run claude export-chat --param thread_id="..." --param mode="latest-assistant"

# Start a new chat
rzn-browser run claude send --param message_text="Summarize the last three commits."

# Reply in an existing thread
rzn-browser run claude send --param thread_id="..." --param message_text="Turn that into a checklist."

# New chat with an attachment
rzn-browser run claude send --param message_text="Describe this image." --param attachment_file_paths="/abs/path/to/image.png"

# Multiple attachments, explicit model, thinking off
rzn-browser run claude send \
  --param message_text="Diff these two diagrams." \
  --param attachment_file_paths="/a.png,/b.png" \
  --param model_slug="Sonnet 4.6" \
  --param adaptive_thinking="off"
```

## Notes

- Every flow assumes the active Chrome profile is already authenticated to Claude.
- `send` requires the rzn-browser extension patched for empty/placeholder-safe `upload_file`. Rebuild with `cd extension && bun run build` and reload the extension in `chrome://extensions` if attachments silently fail.
- Turn detection in `export-chat` uses stable Claude selectors: `[data-testid='user-message']` for user turns and the `.group` wrapper containing `[class*='font-claude-response']` or `[data-testid='action-bar-retry']` for assistant turns. `sr-only`, buttons, and `<style>` blocks are stripped.
- Model picker uses `button[data-testid='model-selector-dropdown']` and reads current state from its `aria-label` (format: `"Model: Opus 4.7 Adaptive"`).
- Pair any `send` with `export-chat --param mode="latest-assistant"` to fetch Claude's reply after streaming finishes.
- `export-chat --param download_artifacts="true"` clicks each `button[aria-label^='Download ']` in the thread. Each click runs in its own `execute_javascript` step so Chrome treats them as separate user gestures (avoids the "Download multiple files" permission prompt). Up to 8 artifacts per thread; files land in Chrome's default Downloads folder. Caller is responsible for moving files to a destination based on the returned `artifacts[]` slug list.
