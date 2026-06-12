# ChatGPT: Recent Chats

- JSON: `workflows/chatgpt/chatgpt_recent_chats_v1.json`
- Purpose: Open ChatGPT in a dedicated workflow tab and return the most recent chats with optional `limit` / `days` filtering.
- Optional params: `limit`, `days`
- Canonical CLI:

```sh
rzn-browser run chatgpt recent-chats-v1 --param limit="10" --param days="7"
```

- Notes: Returns normalized `thread_id` / `chat_id` entries. This is the discovery surface for thread-oriented read/write flows.
