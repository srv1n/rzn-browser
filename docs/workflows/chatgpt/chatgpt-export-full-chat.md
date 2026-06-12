# ChatGPT: Export Full Chat

- JSON: `workflows/chatgpt/chatgpt_export_full_chat_v1.json`
- Purpose: Reopen an existing chat by `chat_id`, scroll toward the top to load older turns, and export the structured transcript plus aggregated file/image metadata.
- Required params: `chat_id`
- Canonical CLI:

```sh
rzn-browser run chatgpt export-full-chat-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef"
```

- URL-or-UUID CLI:

```sh
rzn-browser run chatgpt export-full-chat-v1 --param chat_id="https://chatgpt.com/c/01234567-89ab-cdef-0123-456789abcdef"
```

- Notes: This is the canonical read path for a known `chat_id`.
- Asset shape: successful exports now include top-level `assets.files` and `assets.images`, plus per-turn `assets` for file/image references discovered in the exported DOM.
- Content shape: each exported turn now carries plain `text` plus derived `markdown`. The markdown is reconstructed from the rendered DOM, so it is useful and structured, but not guaranteed to be byte-for-byte identical to the model's original source markdown.
- Raw CLI returns the transcript plus asset URLs directly.
- Repo-local `threads/<id>.json|.md|.jsonl` sidecars and local asset materialization remain helper-layer behavior on top of this command.
