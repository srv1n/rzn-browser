# ChatGPT: Continue Chat

- JSON: `workflows/chatgpt/chatgpt_continue_chat_v1.json`
- Purpose: Reopen an existing chat, enforce GPT-5.6 Sol with Pro intelligence, send another prompt, and return immediate post-send state.
- Required params: `chat_id`, `message_text`
- Optional params: `model_slug`, `model_effort`
- Canonical CLI:

```sh
rzn-browser run chatgpt continue-chat-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param message_text="Now turn that into a checklist"
```

- Notes: This is the canonical write path for a known `chat_id`.
