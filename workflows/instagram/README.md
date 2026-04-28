# Instagram Workflows

Deterministic workflows covering the common Instagram surfaces an agent needs: discovery, per-post extraction, and a small set of write actions (search, follow, like, comment, DM). All run via the `rzn-browser` CLI through the extension + native host; no Python wrappers.

## Pack Overview

| Workflow | Side effect | Purpose | Key Params |
| --- | --- | --- | --- |
| `instagram-search.json` | read | Search from the nav rail and return the result list (accounts, hashtags, or places). | `query`, `mode` (`accounts` \| `hashtags` \| `places` \| `top`) |
| `instagram-profile-recent-posts.json` | read | Open a profile and progressively scroll the grid until enough `/p/` and `/reel/` URLs are rendered. | `handle`, `target_count`, `max_scrolls`, `max_idle_scrolls` |
| `instagram-post-extract.json` | read | Open one post/reel/tv URL and return identity, author, timestamp, stats, media assets, and comments. `mode=preflight` skips the carousel walk and DOM fallback. | `post_url`, `mode` (`preflight` \| `full`) |
| `instagram-follow-account.json` | write | Follow one account via the home search rail. Idempotent. | `handle` |
| `instagram-post-like.json` | write | Like or unlike one post. Idempotent. | `post_url`, `action` (`like` \| `unlike`) |
| `instagram-post-comment.json` | write | Post one comment on a post or reel. | `post_url`, `comment_text` |
| `instagram-dm-send.json` | write | Send one direct message to a handle. | `recipient_handle`, `message_text` |

## Running

Search for an account:

```bash
rzn-browser run instagram search --param query="timbersfc" --param mode="accounts"
```

Discover recent posts from a handle:

```bash
rzn-browser run instagram profile-recent-posts --param handle="timbersfc" --param target_count="36"
```

Preflight one post cheaply (no carousel walk):

```bash
rzn-browser run instagram post-extract --param post_url="https://www.instagram.com/p/DXexrK0De7B/" --param mode="preflight"
```

Full extraction of one post:

```bash
rzn-browser run instagram post-extract --param post_url="https://www.instagram.com/p/DXexrK0De7B/"
```

Follow an account (idempotent):

```bash
rzn-browser run instagram follow-account --param handle="timbersfc"
```

Like one post (idempotent):

```bash
rzn-browser run instagram post-like --param post_url="https://www.instagram.com/p/DXexrK0De7B/"
```

Comment on one post:

```bash
rzn-browser run instagram post-comment --param post_url="https://www.instagram.com/p/DXexrK0De7B/" --param comment_text="Great shot!"
```

Send a direct message:

```bash
rzn-browser run instagram dm-send --param recipient_handle="timbersfc" --param message_text="Hi from RZN"
```

## Notes And Limits

- Every workflow opens a dedicated workflow tab (`use_current_tab=false`), so multiple instagram runs are safe to invoke in parallel. They share the Chrome profile's cookies, so they ride the same logged-in session.
- Write workflows require a signed-in Instagram session in the active Chrome profile and return `status=login_required` when the profile is signed out.
- Write workflows are idempotent where idempotency makes sense: `follow-account` / `post-like` no-op if already in the target state; `post-comment` and `dm-send` always create a new public or private artifact, respectively, so the caller is responsible for deduping.
- Rate limiting is an Instagram-side concern: don't loop faster than a real user would. Space batched writes with a real delay between calls.
- Selectors prefer stable signals (ARIA labels, input roles, `/handle/` path matching) over Tailwind class fingerprints. If Instagram ships a DOM change that breaks extraction, update the selector logic inside the workflow's `execute_javascript` step — not by hardcoding new classnames.
- Each write workflow returns a structured status (e.g. `followed | already_following | requested | not_found | login_required | blocked | click_failed`) so callers can branch without re-reading the DOM.
