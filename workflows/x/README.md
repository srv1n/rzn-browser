# X Workflows

Canonical pack for automating `x.com` via the authenticated Chrome session. Every workflow opens a **fresh tab** (`use_current_tab: false`) so multiple flows can run in parallel against the same logged-in profile.

## Pack

| Workflow | Purpose | Required params |
| --- | --- | --- |
| `x_home_timeline_digest` | Scroll the home timeline and return a bounded digest. | — |
| `x_open_post` | Open one post URL and return a DOM + GraphQL snapshot. | `post_url` |
| `x_open_article` | Open one `/article/<id>` URL and return body + assets. | `article_url` |
| `x_open_inbox` | Open the DM inbox read-only. | — |
| `x_open_dm_thread` | Open one DM thread URL read-only. | `thread_url` |
| `x_like_post` | Like one post after a review gate. | `post_url` |
| `x_reply_post` | Reply to one post after a review gate. | `post_url`, `reply_text` |
| `x_create_post` | Publish one top-level post after a review gate. | `post_text` |
| `x_send_dm` | Start a new DM with a handle after a review gate. | `recipient_handle`, `message_text` |
| `x_reply_dm_thread` | Reply inside an existing DM thread after a review gate. | `thread_url`, `message_text` |
| `x_search_posts` | Search posts from a handle, optionally within a date window (top/latest/live). | `handle` (+ optional `since_date`, `until_date`, `timeline_mode`) |
| `x_profile_posts` | Return up to 20 recent posts from one profile. | `handle` |
| `x_thread` | Expand one same-author thread via conversation search. | `post_url`, `handle` |

All mutating flows (`x_like_post`, `x_reply_post`, `x_create_post`, `x_send_dm`, `x_reply_dm_thread`) pause at an in-page review gate (`approval_mode: "ask_user"`, `continue_on_timeout: false`) and only send on explicit continue.

## Auth

The workflows reuse the Chrome browser's logged-in `x.com` session. No cookie export, no separate credential store. The native host + extension need to be connected before a run.

## Running

```bash
rzn-browser run x home-timeline-digest
rzn-browser run x open-post --param post_url="https://x.com/elonmusk/status/2046981493197586714"
rzn-browser run x search-posts --param handle="felixrieseberg" --param since_date="2026-03-10" --param until_date="2026-03-18" --param timeline_mode="live"
rzn-browser run x thread --param post_url="https://x.com/felixrieseberg/status/123" --param handle="felixrieseberg"
rzn-browser run x create-post --param post_text="Today's a great day"
rzn-browser run x reply-post --param post_url="https://x.com/elonmusk/status/2046981493197586714" --param reply_text="I love your content, cheers"
rzn-browser run x like-post --param post_url="https://x.com/elonmusk/status/2046981493197586714"
rzn-browser run x send-dm --param recipient_handle="jack" --param message_text="Today's a great day"
rzn-browser run x reply-dm-thread --param thread_url="https://x.com/messages/123-456" --param message_text="Today's a great day"
```

## Design rules

- **One workflow per distinct capability.** `x_search_posts` absorbs the old `search-user-window` + `search-top-from-user`; `x_thread` absorbs `thread-from-post-url` + `thread-from-current-tab`. Tab policy is no longer a reason to split.
- **No `_v1` suffix on filenames.** `id` and `version` live inside the JSON.
- **Fresh tab by default.** `browser_automation.use_current_tab: false` everywhere so parallel runs don't collide.
- **JavaScript first, CDP only when forced.** Composer activation and final send clicks use `click_element { use_cdp: true }` because X gesture-gates those surfaces. Non-composer actions, such as opening the DM composer or clicking Like, should start with the standard non-CDP `click_element` path and rely on follow-up assertions to catch ignored clicks.
- **Review gates are mandatory for mutating flows.** `request_user_intervention` with `approval_mode: "ask_user"` and `continue_on_timeout: false` — timing out stops the workflow rather than falling through to send.

## Notes and limits

- x.com is a fast-moving SPA; selectors can drift. Probe live DOM via the Claude-in-Chrome extension before editing.
- `x_search_posts` and `x_thread` build their URL and navigate via `setTimeout(() => location.assign(url), 0)` because awaiting `location.assign` inside an eval kills the execution context.
- Landing-page consent banners can intercept clicks on the search box but do not block `location.assign` — direct URL navigation is used everywhere.
- X Chat may redirect `/messages` → `/i/chat` and gate the inbox behind passcode onboarding. `x_send_dm` and `x_reply_dm_thread` treat that onboarding surface as a first-class state with its own review gate.
- Operators can override approval behavior at runtime with `RZN_APPROVAL_MODE` / `RZN_INTERVENTION_POLICY` and `RZN_CONTINUE_ON_TIMEOUT` / `RZN_APPROVAL_CONTINUE_ON_TIMEOUT`.

## Catalog

- `resources/cards/social/x_v1.json` — canonical card catalog.
- `resources/cards/social/x_browser_profile_v1.json` — browser-connector operation registry (points at the catalog).
