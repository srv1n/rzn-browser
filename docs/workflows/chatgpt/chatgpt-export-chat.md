# ChatGPT: Export Visible Chat

- JSON: `workflows/chatgpt/chatgpt_export_chat_v1.json`
- Purpose: Reopen an existing chat by `chat_id` and extract the visible transcript into structured role/content turns.
- Required params: `chat_id`
- Canonical CLI:

```sh
rzn-browser run chatgpt export-chat-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef"
```

- Notes: Faster than full export, but limited to the currently loaded DOM.
