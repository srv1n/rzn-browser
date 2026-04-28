---
title: Claude Workflows
slug: /workflows/claude/
sidebar:
  label: Claude
---

# Claude Workflows

The canonical surface is the workflow CLI:

```sh
rzn-browser run claude <workflow> --param key="value"
```

## Runtime Choice

- Current documented choice: dedicated-tab.
- Read flows were live-validated there.
- Reply is implemented there, but final live reply validation is still pending.

## Thread Operations Today

| Capability | Workflow | Status | Notes |
| --- | --- | --- | --- |
| List recent chats | `recent-chats-v1` | Validated | Canonical discovery path |
| Fetch one thread by `thread_id` | `export-full-chat-v1` | Validated | Canonical read path |
| Post into an existing thread by `thread_id` | `reply-chat-v1` | Implemented | Canonical write path, but live reply validation is still pending |
| Write local JSON/Markdown for a thread | `rzn-browser run claude export-chat --param thread_id=...` | Implemented | Returns one structured payload; callers persist it however they prefer |
| Select the newest assistant/user message | `rzn-browser run claude export-chat --param thread_id=...` then take the last entry of the requested role from the returned transcript | Implemented | No dedicated `latest` subcommand — derive from the export payload |

## Notes
- The pack is session-aware and expects an authenticated Claude session in the active Chrome profile.
- `recent-chats-v1` is the canonical discovery path.
- `export-full-chat-v1` is the canonical read path.
- `reply-chat-v1` is the canonical write path.
- Dedicated-tab is the documented runtime choice today.
- Recent-thread sync, local artifact writing, last-message selection, and reply-refresh state are caller responsibilities. Drive them by composing `rzn-browser run claude recent-chats`, `export-chat`, and `send` calls against the binary's stdout — no shared Python helper is shipped.
