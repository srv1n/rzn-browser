# Claude: Export Full Chat

- JSON: `workflows/claude/claude_export_full_chat_v1.json`
- Purpose: Reopen an existing Claude chat by `thread_id`, scroll toward the top, and export the structured transcript.
- Required params: `thread_id`
- Canonical CLI:

```sh
rzn-browser run claude export-full-chat-v1 --param thread_id="your-thread-id"
```

- Notes: This is the canonical read path for a known Claude `thread_id`. Repo-local `json|md|jsonl` sidecars and last-message selection remain helper-layer behavior on top of the raw CLI.
