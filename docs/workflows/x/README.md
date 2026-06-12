---
title: X Workflows
slug: /workflows/x/
sidebar:
  label: X
---

# X Workflows

Canonical pack for automating `x.com` via the authenticated Chrome session. All 11 workflows open a fresh tab so they can run in parallel against the same logged-in profile. Full overview (including a pack table, run examples, and design rules) lives next to the JSON at `workflows/x/README.md`.

## JSON files

- `workflows/x/x_home_timeline_digest.json`
- `workflows/x/x_open.json` — unified post / article / thread reader (auto-detect, returns markdown + assets)
- `workflows/x/x_open_inbox.json`
- `workflows/x/x_open_dm_thread.json`
- `workflows/x/x_like_post.json`
- `workflows/x/x_reply_post.json`
- `workflows/x/x_create_post.json`
- `workflows/x/x_send_dm.json`
- `workflows/x/x_reply_dm_thread.json`
- `workflows/x/x_search_posts.json`
- `workflows/x/x_profile_posts.json`

## Supporting files

- `resources/cards/social/x_v1.json` — canonical card catalog
- `resources/cards/social/x_browser_profile_v1.json` — browser-connector operation registry

## Notes

- Session-aware: expects a logged-in Chrome profile.
- Mutating flows (`like_post`, `reply_post`, `create_post`, `send_dm`, `reply_dm_thread`) pause at an in-page review gate with `approval_mode: "ask_user"` and never fall through on timeout.
- Dedicated workflow tabs by default so parallel runs are safe.
