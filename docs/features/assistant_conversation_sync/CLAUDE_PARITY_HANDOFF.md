# Claude Parity Handoff

> **Historical document.** This handoff describes work done against the now-removed `scripts/assistant_conversation_sync.py` Python helper. The orchestration surface it references no longer exists — the binary `rzn-browser run claude recent-chats | export-chat | send` commands replace it. Kept for context only; see `README.md` in this directory for the current architecture.

## Goal

Bring Claude thread operations to the same practical level as ChatGPT:

- list recent conversations
- fetch one thread by `thread_id`
- post a new message into one thread by `thread_id`
- keep local JSON / Markdown / `index.json` state in sync

The important product rule: the workflow CLI is the first-class orchestration surface. The Python helper is only a local artifact/index layer on top.

## What ChatGPT Has Today

ChatGPT is the reference implementation.

- Canonical read path for a known id:
  - `workflows/chatgpt/chatgpt_export_full_chat_v1.json`
- Canonical write path for a known id:
  - `workflows/chatgpt/chatgpt_continue_chat_v1.json`
- Canonical discovery path:
  - `workflows/chatgpt/chatgpt_recent_chats_v1.json`
- Helper-layer local state:
  - `scripts/assistant_conversation_sync.py sync --system chatgpt`
  - `scripts/assistant_conversation_sync.py fetch --system chatgpt --chat-id ...`
  - `scripts/assistant_conversation_sync.py reply --system chatgpt --chat-id ... --message ...`
- Docs already updated to be CLI-first:
  - `workflows/chatgpt/README.md`
  - `docs/workflows/chatgpt/README.md`

## Current Claude State

Claude already has the basic workflow files:

- `workflows/claude/claude_recent_chats_v1.json`
- `workflows/claude/claude_export_full_chat_v1.json`
- `workflows/claude/claude_reply_chat_v1.json`

The shared helper already supports Claude:

- `scripts/assistant_conversation_sync.py sync --system claude`
- `scripts/assistant_conversation_sync.py fetch --system claude --thread-id ...`
- `scripts/assistant_conversation_sync.py reply --system claude --thread-id ... --message ...`

But Claude is not at ChatGPT parity yet because:

- it has not been live-validated to the same standard
- its docs are still wrapper-first / older style
- Claude should stay on the dedicated-tab policy unless live evidence proves a workflow-engine bug; do not move shipped workflows to active-tab binding as a workaround
- transcript quality and reply stability still need real session evidence

## Required Outcome

Another agent should land Claude parity with this exact bar:

| Area | Required Outcome |
| --- | --- |
| Discovery | `claude_recent_chats_v1.json` works live and returns usable `thread_id` entries |
| Read by id | `claude_export_full_chat_v1.json` works live for one known `thread_id` |
| Write by id | `claude_reply_chat_v1.json` works live for one known `thread_id` with a controlled test message |
| Helper fetch | `assistant_conversation_sync.py fetch --system claude --thread-id ...` works live |
| Helper reply | `assistant_conversation_sync.py reply --system claude --thread-id ... --message ...` works live |
| Docs | Claude docs mirror ChatGPT’s CLI-first structure and clearly separate workflow CLI vs helper-layer behavior |
| Runtime choice | Validate Claude on dedicated workflow tabs and document any engine bug separately |

## Non-Negotiable Constraints

- Do not make Python the “official” orchestration surface in docs.
- The first-class contract must stay workflow CLI plus explicit variables.
- Do not add site-specific engine code. Fix Claude drift in Claude workflow JSONs, not in shared Rust/extension logic.
- Keep browser noise low. Do not spam retries or parallel runs on a live authenticated session.
- If dedicated-tab Claude is flaky, file the engine/workflow blocker; do not switch shipped workflows to active-tab binding.

## Suggested Execution Order

1. Live-validate `claude_recent_chats_v1.json`.
2. Pick one real Claude `thread_id` from the result.
3. Live-validate `claude_export_full_chat_v1.json` on that thread.
4. Live-validate `claude_reply_chat_v1.json` on that thread with a safe test message.
5. Run helper-layer `fetch --system claude --thread-id ...`.
6. Run helper-layer `reply --system claude --thread-id ... --message ...`.
7. Update Claude docs to match the ChatGPT documentation shape.
8. Update `docs/features/assistant_conversation_sync/README.md` with the real Claude runtime decision and status.

## Files To Touch

- Workflows:
  - `workflows/claude/claude_recent_chats_v1.json`
  - `workflows/claude/claude_export_full_chat_v1.json`
  - `workflows/claude/claude_reply_chat_v1.json`
- Docs:
  - `workflows/claude/README.md`
  - `docs/workflows/claude/README.md`
  - `docs/workflows/claude/claude-recent-chats.md`
  - `docs/workflows/claude/claude-export-full-chat.md`
  - `docs/workflows/claude/claude-reply-chat.md`
  - `docs/features/assistant_conversation_sync/README.md`
- Helper, only if truly needed:
  - `scripts/assistant_conversation_sync.py`

## Acceptance Criteria

- A real Claude session can:
  - list recent chats
  - fetch one thread by `thread_id`
  - post a reply into one thread by `thread_id`
- The helper can:
  - fetch one Claude thread into local artifacts
  - reply and refresh local artifacts
- Claude docs clearly state:
  - canonical read path
  - canonical write path
  - canonical discovery path
  - dedicated-tab validation status and any remaining runtime blocker
  - that the helper is not the primary control plane

## Copy-Paste Prompt

```text
Bring Claude thread operations to parity with the validated ChatGPT thread flows.

What “parity” means:
- list recent Claude chats
- fetch one Claude thread by thread_id
- post a new message into one Claude thread by thread_id
- keep local JSON / Markdown / index.json state in sync

Important constraints:
- The workflow CLI is the first-class orchestration surface. Do not make the Python helper the official interface in docs.
- Keep browser noise low. No spammy retries or parallel runs on the live authenticated Claude session.
- Do not add site-specific engine code. Fix Claude behavior in Claude workflow JSONs and docs.
- If dedicated-tab Claude is flaky, document the blocker and keep shipped workflow JSON on the manifest standard with dedicated workflow tabs.

Files to use:
- workflows/claude/claude_recent_chats_v1.json
- workflows/claude/claude_export_full_chat_v1.json
- workflows/claude/claude_reply_chat_v1.json
- workflows/claude/README.md
- docs/workflows/claude/README.md
- docs/workflows/claude/claude-recent-chats.md
- docs/workflows/claude/claude-export-full-chat.md
- docs/workflows/claude/claude-reply-chat.md
- docs/features/assistant_conversation_sync/README.md
- scripts/assistant_conversation_sync.py

Required validation order:
1. Live-validate claude_recent_chats_v1.json
2. Pick one real thread_id
3. Live-validate claude_export_full_chat_v1.json on that thread
4. Live-validate claude_reply_chat_v1.json on that thread with a controlled test message
5. Live-validate helper fetch: scripts/assistant_conversation_sync.py fetch --system claude --thread-id ...
6. Live-validate helper reply: scripts/assistant_conversation_sync.py reply --system claude --thread-id ... --message ...
7. Rewrite Claude docs to mirror the ChatGPT docs structure, with CLI-first examples and explicit “what works today” status

Acceptance criteria:
- Claude recent chats works live
- Claude fetch by thread_id works live
- Claude reply by thread_id works live
- Helper fetch/reply for Claude work live
- Docs clearly state the canonical read path, write path, discovery path, and runtime choice
```
