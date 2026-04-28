# Multi-Workflow Concurrency: Client Integration Brief

## Audience
This is for client/app teams integrating with the broker + extension stack.

## What Changed
- Broker now supports multiple app connections and multiple in-flight requests per connection.
- Broker routes responses by correlation id (`req_id` / `task_id`) instead of a single "current connection" model.
- Extension now tracks workflow state per `session_id` (`session_id -> workflowTabId + FIFO queue`).
- CDP/debugger actions are serialized behind a shared lock to avoid contention and nondeterministic failures.
- DOM/content-script actions remain parallel across sessions.

## Why This Matters
- 1->N is supported: one client can run many concurrent jobs.
- N->1 is supported: multiple clients can share one broker process.
- N->N is supported with one caveat: CDP paths are queued; DOM-tier paths run in parallel.

## Required Client-Side Changes
- Always send a stable `data.session_id` for each job.
  - One unique `session_id` per concurrent job.
  - Reuse that same `session_id` for all rounds/steps of that job.
- Keep `req_id` unique per request.
  - Do not reuse an in-flight `req_id`.
  - Correlate responses by `req_id`/`task_id`, not by arrival order.
- Persist and replay `data.current_tab_id` for resumed/retried rounds in the same session.
  - Read `current_tab_id` from responses and include it in subsequent requests for that session.
- Expect out-of-order completion across concurrent requests.
  - The broker no longer enforces request-response ordering globally.

## Backward Compatibility
- If `data.session_id` is missing, requests fall back to `default`.
- `default` mode behaves like serialized/single-session execution.
- Existing single-job clients continue to work without code changes.
- Concurrency benefits require explicit `session_id` adoption.

## Message Contract (Recommended)
Request shape:
```json
{
  "cmd": "execute_workflow",
  "req_id": "job-42-step-3",
  "task": { "...": "..." },
  "data": {
    "session_id": "job-42",
    "current_tab_id": 123
  }
}
```

Response fields to consume:
- `req_id` and/or `task_id` for correlation
- `success`, `error`/`error_msg`, `error_code`
- `current_tab_id` and `current_url` (when available)

## CDP Queue Semantics
- CDP commands are intentionally serialized (shared lock).
- DOM-tier commands run concurrently across sessions.
- If many jobs enter CDP-heavy paths, queueing delay is expected.
- Client timeouts should be set with this in mind (avoid overly aggressive per-step timeout).

## Operational Constraints (Known)
- `chrome.tabs.captureVisibleTab` remains foreground/visible-tab sensitive.
- All sessions share one Chrome profile (cookies/storage shared unless profile isolation is introduced separately).
- `chrome.debugger` attach semantics still apply; queueing reduces collisions but does not change Chrome limits.

## Suggested Rollout Plan
- Phase 1: Ship `session_id` + correlation correctness (`req_id` uniqueness and response matching).
- Phase 2: Add `current_tab_id` persistence/resume support.
- Phase 3: Tune app-level timeout/retry policy for CDP queue delays.
- Phase 4: Add client integration tests for:
  - two concurrent sessions with isolated tab state
  - out-of-order response handling
  - CDP burst behavior (no cross-session corruption)

## Quick Acceptance Checklist
- [ ] Each concurrent job has a unique `session_id`.
- [ ] No in-flight `req_id` collisions.
- [ ] Client handles out-of-order responses safely.
- [ ] `current_tab_id` is fed back for resumed sessions.
- [ ] CDP-step timeout/retry policy accounts for queue wait.
