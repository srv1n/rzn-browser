# Claude: Recent Chats

- JSON: `workflows/claude/claude_recent_chats_v1.json`
- Purpose: Open Claude in a dedicated tab and extract the visible recent chat list with optional `limit` / `days` filtering.
- Optional params: `limit`, `days`
- Canonical CLI:

```sh
rzn-browser run claude recent-chats-v1 --param limit="10" --param days="7"
```

- Notes: This is the canonical discovery path for Claude thread ids.
