# Claude: Reply To Chat

- JSON: `workflows/claude/claude_reply_chat_v1.json`
- Purpose: Reopen an existing Claude chat by `thread_id`, send another prompt, and return the immediate post-send state.
- Required params: `thread_id`, `message_text`
- Canonical CLI:

```sh
rzn-browser run claude reply-chat-v1 --param thread_id="your-thread-id" --param message_text="Continue the last answer with concrete next steps."
```

- Notes: This is the canonical write path for a known Claude `thread_id`. Repo-local artifact refresh is helper-layer behavior on top of this command, not a separate workflow surface.
