# Reddit Profile History Browser Workflow

## Goal

Provide a read-only fallback/cross-check path for Reddit profile history when
the API or anonymous JSON endpoints are blocked. The workflow must use the
normal `rzn-browser -> supervisor -> native host -> browser extension` path and
the connected browser's existing cookies. Account authentication improves
history depth but is not required for public profile extraction.

## User Outcome

`reddit/profile-history` returns rendered posts and comments with their text,
subreddit, exact timestamp, title/context, stable Reddit id, and permalink. It
does not need OAuth or API keys.

## Observed DOM Contract

Live Chrome probing on 2026-08-01 found:

- comments: `shreddit-profile-comment`, with `comment-id`, `href`, nested
  `time[datetime]`, `[data-testid="location-anchor"]`, and
  `[id$="-post-rtjson-content"]`;
- posts: `shreddit-post`, with `id`, `author`, `subreddit-name`, `permalink`,
  nested `time[datetime]`, `[slot="title"]`, and `[slot="text-body"]`;
- logged-out state: `#login-button[href*="/login"]`.

These are site-owned selectors and therefore maintenance-sensitive.

## Design

The workflow opens Reddit, waits at least three seconds, then navigates to one
profile tab (`overview`, `posts`, or `comments`). One JavaScript step harvests
cards before and after every bounded scroll, retaining normalized objects in a
map so DOM virtualization cannot discard already-seen activity.

It returns explicit completeness fields:

- `requested_limit_reached`
- `stop_reason`
- `history_may_be_partial`
- `warnings`

A stalled short feed is partial evidence, never silently "complete" history.

## Safety And Failure Modes

- The default `require_login=false` accepts an anonymous cookie-backed browser
  session. Callers that require account authentication can opt into strict
  rejection with `require_login=true`.
- Challenge, network-block, and rate-limit pages fail instead of returning an
  empty history.
- Navigation delay is clamped to at least 3000 ms and scroll delay to at least
  1200 ms.
- Limits and scroll counts are capped to prevent unbounded runs.
- The workflow is read-only but changes browser tab/navigation state.
- Callers should serialize profile visits for one Reddit account; the workflow
  must not be used as a high-concurrency crawler.

## Verification

Required before calling the route production-ready:

1. strict per-file validation;
2. catalog validation and contract inspection;
3. live anonymous cookie-backed extraction proof;
4. live strict-auth rejection proof with `require_login=true`;
5. live logged-in proof on posts, comments, and overview against the same
   account, including at least one scroll cycle.

## Rollback

Remove `workflows/reddit/reddit-profile-history.json` if Reddit changes the
profile markup and the route begins failing. Failure is preferable to silently
returning empty or misclassified history.
