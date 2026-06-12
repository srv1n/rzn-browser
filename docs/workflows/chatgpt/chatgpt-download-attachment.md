# ChatGPT: Download Attachment

- JSON: `workflows/chatgpt/chatgpt_download_attachment_v1.json`
- Purpose: Reopen an existing chat by `chat_id`, scope to the latest assistant turn, find an attachment-like button by exact visible label, and click it.
- Required params: `chat_id`, `attachment_label`
- Canonical CLI:
  - If `rzn-browser` is not on `PATH`, use `./target/debug/rzn-browser` from the repo root.

```sh
rzn-browser run chatgpt download-attachment-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param attachment_label="Zip package with Markdown + extracted figure assets"
```

- Notes:
  - This workflow exists because some ChatGPT artifacts are buttons, not anchors.
  - It does not scrape share/copy-link controls; it scopes to the latest assistant turn only.
  - It accepts a bare ChatGPT UUID or a full `https://chatgpt.com/c/<id>` URL in `chat_id`.
  - Browser download verification is still a separate concern from the click itself.

## Validated Labels

Validated on April 16, 2026 against a real ChatGPT assistant turn with these exact labels:

| Attachment label | Download observed |
| --- | --- |
| `Self-contained HTML manual` | `game_design_operating_manual.html` |
| `Markdown source` | `game_design_operating_manual.md` |
| `Zip package with Markdown + extracted figure assets` | `game_design_sop_package.zip` |

Important detail:

- The zip is not the result of all three buttons collapsing into one artifact.
- The first button downloads HTML.
- The second button downloads Markdown.
- The third button downloads a zip bundle that contains HTML + Markdown + extracted assets.

## Verification Pattern

Use the workflow once per label, then diff the browser Downloads folder:

```sh
rzn-browser run chatgpt download-attachment-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param attachment_label="Self-contained HTML manual"
ls -lt ~/Downloads | rg "game_design_operating_manual|game_design_sop_package"
```
