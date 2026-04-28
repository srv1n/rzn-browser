# ChatGPT: New Chat Send With Attachment

- JSON: `workflows/chatgpt/chatgpt_new_chat_send_attachment_v1.json`
- Purpose: Open ChatGPT, normalize to a fresh chat, default to `Pro` with `Extended` effort unless overridden, upload a local file or image, send the initial prompt, and return the resolved `chat_id`.
- Required params: `message_text`, `attachment_file_path`
- Optional params: `model_slug`, `model_effort`
