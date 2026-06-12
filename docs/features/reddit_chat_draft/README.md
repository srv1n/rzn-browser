# Reddit Chat Draft

## Overview
- Goal: Provide a deterministic workflow that opens a Reddit user's profile on new Reddit, launches chat, types a draft message, and pauses for manual review before the human sends it.
- Constraints: Reddit chat is only available on new Reddit, the browser session is already authenticated in Chrome, manual review may take several minutes, and selectors must remain workflow-level rather than being added as site-specific engine code.

## Flow Diagrams
- End-to-end flow
```text
CLI run
  -> supervisor
  -> native host
  -> Chrome extension background
  -> content script
  -> reddit.com profile/chat UI
  <- content script
  <- extension background
  <- native host
  <- supervisor
  <- CLI
```

- Internal flow
```text
navigate_to_url
  -> assert_url_matches(new Reddit profile)
  -> find profile chat href
  -> navigate to /chat/user/{t2_id}
  -> poll RS-APP -> RS-DIRECT-CHAT -> RS-MESSAGE-COMPOSER shadow roots
  -> set textarea value with native setter + input/change events
  -> leave typed draft for manual review
```

## Decision Record
- New Reddit is required for chat. Old Reddit may still be useful for legacy inbox/PM flows, but it is the wrong target for chat composition.
- The workflow now fails early if the browser gets redirected away from `https://www.reddit.com/user/...`. This is preferable to a false-positive run that types into the wrong page.
- The worker now forwards step-specific timeouts to the native host bridge. Without that, manual-review steps fail after 20 seconds regardless of the workflow timeout.
- Reddit's current chat entry opens a full page instead of a sidebar. Following the profile's chat href is more reliable than trying to close or retarget stale sidebar state.
- The composer sits behind multiple shadow roots. The workflow first checks the known `RS-APP -> RS-DIRECT-CHAT -> RS-MESSAGE-COMPOSER` chain and only then falls back to a generic deep shadow traversal.
- Async polling steps must carry an explicit `timeout_ms`; otherwise the engine's default JS timeout can kill a longer wait even when the script's internal loop keeps polling.

## Architecture
- Modules:
  - `/Users/sarav/Downloads/side/rzn/rzn-browser/workflows/reddit/reddit-draft-dm.json`: Workflow data for the Reddit chat draft flow.
  - `/Users/sarav/Downloads/side/rzn/rzn-browser/crates/rzn_browser/src/supervisor.rs`: Supervisor bridge that forwards browser calls to the native host with per-step timeouts.
  - `/Users/sarav/Downloads/side/rzn/rzn-browser/extension/src/contentScript.ts`: Implements `request_user_intervention` in-page UI and content-script actions.
- Data contracts:
  - `browser.execute_step` payload carries `{ session_id, step }`.
  - The worker now derives the native-host timeout from `step.timeout_ms` / `step.timeoutMs` and adds a small grace window.
  - Reddit draft inputs are passed as URL-encoded `message_body`, then restored in-page with `decodeURIComponent(...)` before typing.

## Implementation Notes
- Entry points:
  - `BrowserWorker::call_browser_session(...)`
  - `actionHandlers.execute_javascript`
- Key calls and event flow:
  - The profile page exposes a chat link whose href contains Reddit's internal `t2_...` user id; the workflow extracts that href and navigates directly to the full chat page.
  - Composer lookup is repeated in both the "wait" and "type" steps instead of caching a DOM node across steps, which makes the workflow resilient to SPA re-renders.
  - Typing uses the native `HTMLTextAreaElement.prototype.value` setter plus `input` and `change` events; a contenteditable fallback remains wired for future Reddit UI shifts.
- Error handling & retries:
  - Redirects to `old.reddit.com` now fail at URL assertion instead of silently passing.
  - Missing chat links fail early with `No chat link found on profile`, which covers suspended/deleted users and users with chat disabled.
  - Composer discovery polls for up to 15 seconds, and the step timeout is set above that budget so hydration delays do not fail the run prematurely.

## Tasks & Status
- [x] Reproduced the real failure in the native Chrome path
- [x] Identified the 20s manual-review timeout bug in the worker/native-host bridge
- [x] Replaced stale sidebar assumptions with full-page chat navigation
- [x] Reworked composer lookup for Reddit's current deep shadow-DOM structure
- [x] Added explicit JS step timeouts for long composer polling
- [ ] Re-run against the authenticated Chrome session after rebuilding worker + extension

## What Works (Do Not Change)
- Keep Reddit-specific targeting in workflow JSON, not in shared engine heuristics.
- Keep chat as a new-Reddit-only flow; old Reddit can be handled with separate legacy-message workflows if needed.
- Preserve the generic worker/native-host timeout propagation because other long-running steps may rely on it.
- Keep the typing logic idempotent at the workflow layer: rediscover the composer each step instead of depending on cross-step DOM-node state.

## Tried & Didn’t Work
- Playwright/browser sandbox reproduction: not representative of the logged-in Chrome session and triggered Reddit anti-bot checks.
- Broad selectors plus fixed sleep: produced misleading "success" statuses without proving the chat composer was actually targeted.
- Reusing a cached composer reference across execute-JS steps: vulnerable to Reddit SPA re-renders and execution-world boundaries.
