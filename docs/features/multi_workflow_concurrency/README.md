# Multi-Workflow Concurrency (Design)

## Overview
- Goal: run multiple independent workflow “jobs” concurrently (1→N, then N→N) in a single browser profile by defaulting to content-script/JS execution, while treating CDP/`chrome.debugger` as a scarce “break-glass” capability that is queued/serialized.
- Constraints: MV3 service worker lifecycle; native messaging is a single port; `chrome.debugger` is single-attacher per tab; screenshots via `chrome.tabs.captureVisibleTab` are inherently foreground/visible; tabs share cookies/storage within a Chrome profile; avoid site-specific selectors/targeting.

## Flow Diagrams
- End-to-end (multi-job)
```
Client(s) / Orchestrator(s)
    │  (req_id + session_id/job_id)
    ▼
Broker (native host) ── routes replies by req_id ──► correct client connection
    │
    ▼
Extension (MV3 service worker)
    │  (SessionManager: session_id → tabId + per-session FIFO)
    ├─ DOM-tier (default): content script messages per tab (parallel across sessions)
    └─ CDP-tier (rare): acquire CDP_LOCK → run CDP action/context → release
    ▼
Content Script → Page
```

- Internal flow (per-session FIFO + global CDP gate)
```
onBrokerMessage(msg):
  sid = msg.data.session_id ?? "default"
  enqueue(sessionQueue[sid], async () => {
    tabId = resolveOrCreateTab(sid, msg.data.current_tab_id?)
    if (needsCDP(msg)):
      await CDP_LOCK.acquire()
      try runCDP(tabId, msg) finally CDP_LOCK.release()
    else:
      await runDOM(tabId, msg)
  })
```

## Decision Record
- Why queue CDP globally (v1)
  - Minimizes concurrency hazards in CDP-adjacent singleton helpers/actions.
  - Avoids debugger contention and makes behavior deterministic under load.
  - Matches “JS-first, CDP-rare” philosophy: throughput impact is limited in practice.
- Alternatives considered
  - Per-tab CDP locks (higher throughput): likely viable later; requires auditing all CDP call sites for shared mutable state.
  - Multiple broker processes / multiple native ports: conflicts with fixed IPC socket paths and complicates client discovery/routing.
  - “Always CDP”: increases detectability and cost; contradicts stealth-first posture.

## Architecture
- Modules
  - Broker: `/rzn_broker/src/main.rs`
    - Replace “current app connection” forwarding with a router keyed by `req_id`.
    - Allow multiple app connections + multiple in-flight requests per connection.
  - Extension SW: `/extension/src/background.ts`
    - Replace `globalWorkflowTabId` with `SessionManager` (`session_id → tab state`).
    - Replace global `brokerMessageQueue` with per-session FIFO queues.
    - Add `CDP_LOCK` gate around all `chrome.debugger` / CDP paths.
  - Client (Rust): `/crates/rzn_plan/src/broker_client.rs`
    - Continue sending `data.session_id` and `data.current_tab_id`.
    - For 1→N, run one `BrokerClient` per job (unique `session_id` per client instance), or introduce explicit `job_id`.

- Data contracts (message shapes)
  - Required (for concurrency-safe routing)
    - `req_id` (string): correlation id for request/response routing.
    - `data.session_id` (string): stable execution context key used for tab ownership + per-session FIFO.
  - Optional
    - `data.current_tab_id` (number): resume an existing tab for that session (used for reconnect/retry).
    - `data.job_id` (string): future-proofing when one client wants multiple concurrent jobs over a single connection.
  - Responses
    - Must always include `req_id`.
    - Should include `current_tab_id` + `current_url` where available.

## Implementation Notes
- Entry points
  - Extension: `handleBrokerMessage(...)`, `executeWorkflow(...)`, and all `message.cmd === "cdp_*"` branches in `/extension/src/background.ts`.
  - Broker: native read loop + response forwarding, and app-connection handling in `/rzn_broker/src/main.rs`.
- Key calls and event flow
  - DOM-tier: `sendMessageTopFrame(tabId, message)` and/or `chrome.scripting.executeScript(...)` for navigation/source capture.
  - CDP-tier: `frameRouter.attachToTab(tabId)`, CDP integration calls, and any “trusted event” break-glass actions.
- Error handling & retries
  - Timeouts: enforce per-request timeouts in broker and fail fast with a correlated error response.
  - Cleanup: if a session’s tab is closed, clear that session’s tab state; allow next step to recreate.
  - Backpressure: if per-session queues grow too large, reject or shed load with explicit error codes.

## Tasks & Status
- [x] Broker: route extension responses by `req_id` to the correct app connection (remove single “current connection” forwarding).
- [x] Broker: allow multiple in-flight requests per connection (no blocking “wait loop” that halts reads).
- [x] Extension: implement `SessionManager` (`session_id → tabId`) and remove `globalWorkflowTabId` for routing.
- [x] Extension: replace global `brokerMessageQueue` with per-session FIFO queues (parallel across sessions).
- [x] Extension: add global `CDP_LOCK` and wrap all CDP/debugger call paths.
- [x] Protocol: ensure `session_id`/`current_tab_id` is not dropped when broker forwards `perform_task`/workflow messages.
- [x] Tests: added extension e2e coverage for two concurrent workflows (DOM-tier isolation) and concurrent CDP broker commands behind the shared `CDP_LOCK` queue (`extension/tests/e2e/multi_workflow_concurrency.spec.ts`), in addition to broker-level concurrent in-flight unit tests.

## What Works (Do Not Change)
- Stealth-first default: DOM-tier execution remains the default; CDP remains opt-in/break-glass.
- Correlation: `req_id` is the single source of truth for request/response matching.
- Generic targeting: no site-specific selectors or domain-tuned rules added to core code paths.

## Tried & Didn’t Work
- One global workflow tab + one global broker queue: simple, but fundamentally blocks 1→N concurrency.
- Multiple broker instances: collides on fixed IPC socket paths and complicates discovery.
