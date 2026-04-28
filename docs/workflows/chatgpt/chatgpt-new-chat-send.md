# ChatGPT: New Chat And Send

- JSON: `workflows/chatgpt/chatgpt_new_chat_send_v1.json`
- Purpose: Open ChatGPT, normalize to a fresh chat, default to `Pro` with `Extended` effort unless overridden, send the initial prompt, and return the resolved `chat_id`.
- Required params: `message_text`
- Optional params: `model_slug`, `model_effort`
