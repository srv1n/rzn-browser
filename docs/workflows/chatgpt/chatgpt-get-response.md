# ChatGPT: Get Latest Response

- JSON: `workflows/chatgpt/chatgpt_get_response_v1.json`
- Purpose: Reopen an existing chat by `chat_id`, wait for the latest assistant turn to stabilize, and return the latest response payload.
- Required params: `chat_id`
- Canonical CLI:

```sh
rzn-browser run chatgpt get-response-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef"
```

- URL-or-UUID CLI:

```sh
rzn-browser run chatgpt get-response-v1 --param chat_id="https://chatgpt.com/c/01234567-89ab-cdef-0123-456789abcdef"
```

- Response shape: `latest_assistant_response` and `latest_user_message` now include plain `text`, derived `markdown`, `links`, `code_blocks`, and latest-turn `assets` when ChatGPT exposes file/image URLs in the DOM.

- Related raw CLI:

```sh
rzn-browser run chatgpt export-full-chat-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef"
```

- Notes: Use `get-response-v1` when the newest assistant turn is enough. Use `export-full-chat-v1` when you also need the full transcript and aggregated asset metadata. Repo-local sidecar refresh remains helper-layer behavior outside the raw CLI.
