# Connection Reliability: worker ↔ native-host ↔ extension

**Status:** partially implemented; handshake restart work still open
**Owner:** TBD (side agent)
**Related code:** `crates/rzn_browser_worker/`, `crates/rzn_native_host/`, `extension/src/actions/upload_file.ts`, `extension/src/background.ts`

## Overview

rzn-browser runs workflows through a three-process topology: a persistent `rzn-browser-worker` (the bridge), short-lived `rzn-native-host` processes (spawned by Chrome per native-messaging port), and the Chrome extension's MV3 service worker. The happy path works. The restart paths do not.

Two distinct reliability bugs were observed on 2026-04-24 and are blocking a clean agent experience:

1. **Stale-worker handshake after Chrome restart.** If the user restarts Chrome while `rzn-browser-worker` is still running from a previous session, the freshly-spawned `rzn-native-host` connects to the old worker, but the worker never surfaces that connection in its health report. `native_host_connected=false, extension_connected=false` remain stuck until the worker is killed by hand.
2. **CDP debugger lifecycle must be explicit and short.** The extension now uses JS-first eval for `execute_javascript`; CDP attaches only for explicit trusted paths such as `upload_file`, `click_element { use_cdp: true }`, `type_text { use_cdp: true }`, `eval_with_cdp`, and `use_cdp_eval`. Remaining work is to keep every CDP-backed action on the shared lifecycle, release sessions cleanly, and preserve regression coverage.

A third latent issue was exposed while patching (2) and is included for completeness: (3) in-step navigation (`window.location.assign`) can cancel the in-flight eval context, producing errors such as `{"code":-32000,"message":"Inspected target navigated or closed"}` on CDP-backed eval.

This doc is the single source of truth for fixing 1, 2, and 3. Do not split them across disconnected PRs — they share the same connection/session-lifecycle surface.

## Flow Diagrams

### Happy path (works today)

```
rzn-browser CLI
    │
    │ 1. spawn rzn-browser-worker (if not running)
    ▼
rzn-browser-worker ──── bridge socket ──── native host socket
    ▲                                              ▲
    │ 2. opens session                             │ 4. connects to worker
    │                                              │
    │                                     rzn-native-host
    │                                              ▲
    │                                              │ 3. spawned by Chrome
    │                                              │    via connectNative()
    │                                   chrome-extension service worker
    │                                              ▲
    │ 5. worker → ext → run steps                  │
    └──────────────────────────────────────────────┘
```

### Bug 1 — Stale worker across Chrome restart

```
T0  CLI run → worker W1 starts → Chrome extension open → native host N1 → W1 registers N1 → workflow OK
T1  (Chrome quits. W1 keeps running.)
T2  Chrome starts → extension service worker activates → connectNative() → native host N2 spawns
T3  N2 connects to W1's bridge socket (accepts connection)
T4  W1's health reports: bridge_connected=true, native_host_connected=FALSE, extension_connected=FALSE
    → No remediation message from W1, no self-heal, no error in logs.
T5  CLI tries to run a workflow → times out after 45s waiting for native_host_connected.
```

Observed: bridge_hosts count grows on every failed CLI run (each CLI run adds a new broker session), but the underlying handshake never completes.

### Bug 2 — Debugger detach between steps

```
Historical failure:
  step N  : upload_file
            attach(tabId) → Runtime.evaluate(getFileInputObjectId) → setFileInputFiles → detach(tabId)
  step N+1: CDP-backed action/eval
            worker sends Runtime.evaluate → CDP returns "Debugger is not attached to the tab with id: <tabId>"
            step fails → workflow fails

Current shape:
  upload_file → cdpSessionManager.acquire(sessionId, tabId) → shared frameRouter attachment
  ordinary execute_javascript → chrome.scripting / page bridge (no debugger)
  explicit CDP action/eval → attach just-in-time, run, release/expire
  workflow/tab end → release coverage and regression tests
```

### Bug 3 — Navigation during an evaluate

```
execute_javascript world:"main" script:"… window.location.assign('/chat/<id>'); await …"
    │
    │ Runtime.evaluate awaits the script promise.
    │ Navigation tears down the execution context.
    │ CDP returns: {code:-32000, message:"Inspected target navigated or closed"}
    ▼
step fails
```

## Decision Record

| # | Decision | Alternatives considered | Rationale |
|---|----------|-------------------------|-----------|
| 1 | Make the **worker** the sole owner of the CDP debugger lifecycle, per session. | Per-action attach/detach (status quo). Detach-only-on-error (landed as interim fix in `upload_file.ts`). | Per-action attach is racy between steps and requires every action that wants CDP to coordinate. Session ownership means the worker attaches once when the workflow starts using CDP on a tab, releases on session end, and every step shares it. Detach-on-error is a band-aid. |
| 2 | Worker should **detect stale-handshake state** and either self-restart or reject new CLI sessions with a clear "please restart worker" error. | Kill old worker on every CLI run (expensive, breaks concurrency). Rely on users noticing. | Self-heal preserves state when possible; explicit error is at least correct when it can't. Silent failure is the worst outcome — users burn time. |
| 3 | Any workflow step whose script triggers navigation should **not await after navigation** (inside the script); the worker should also treat `-32000 / navigated or closed` as a retryable condition once the new context is ready. | Disallow navigation in execute_javascript (too restrictive, breaks the `send` workflow's thread redirect). | Works with the current workflow surface; matches expected SPA behavior. |
| 4 | Do not introduce a fourth process (e.g. a supervisor). | A supervisor daemon watching the worker. | Existing three-process model is enough if the worker gains self-healing. Adding a supervisor adds install complexity. |

## Architecture

### Current

- `rzn-browser-worker`: long-lived, listens on two Unix sockets (`bridge`, `worker`). Accepts broker sessions from CLI. Accepts incoming native-host connections. Publishes health via the broker endpoint file.
- `rzn-native-host`: spawned by Chrome on extension `connectNative()`. Connects to the worker's bridge socket. Proxies native-messaging frames between Chrome and the worker.
- Extension service worker: on activation, calls `chrome.runtime.connectNative("com.rzn.browser.broker")`.

### Proposed — session-scoped CDP

- **Extension owns a `CdpSessionManager`**: keyed by (sessionId, tabId). It attaches on first CDP-requiring step and should release on session/tab end.
- **Actions that need CDP (upload_file, CDP-backed click, future: setFileInputFiles, input dispatch) request a handle** from the manager rather than attaching themselves. The handle is a no-op to release — the manager owns actual attach/detach. `upload_file` has migrated; the remaining CDP-backed actions still need an audit.
- **`execute_javascript` world:"main" is JS-first** through the page bridge or `chrome.scripting.executeScript`; it no longer attaches the debugger by default.
- **Explicit trusted paths use CDP** and must go through the shared lifecycle: `upload_file`, CDP-backed click/type/key actions, AX/CDP context reads, and eval steps marked `use_cdp_eval`.

### Proposed — handshake self-heal

- CLI native-run has a first-line self-heal in `crates/rzn_browser/src/native_runner.rs`: restart the native host by installed path by default, detect the stale accepted-bridge shape (`bridge_connected=true`, `native_host_connected=false`, `extension_connected=false`, `bridge_host_count>0`), terminate only the worker PID published in `broker_endpoint_v1.json`, remove that worker's socket artifacts/spawn lock, and retry once before any workflow step executes.
- CLI retry is preflight-only. Errors are wrapped as `NativeRunPreflightFailure` before session/step dispatch; after a workflow step is sent, native-run reports the failure instead of replaying possibly write-capable actions.
- Self-heal is scoped. With no explicit `--app-base` / `--endpoint-path`, cleanup targets the standalone `rzn-browser` runtime, not random desktop/debug endpoints. Explicit endpoints remain authoritative.
- Worker health uses short pings and prunes failed bridge sessions. A dead native-host bridge is evicted on send failure, response-channel close, request timeout, native-host JSON-RPC error, or malformed response. Health retries once after pruning so another live bridge can win.
- Worker bridge keepalive is faster (`10s` interval, `5s` timeout) and health exposes bridge diagnostics.
- `llm-auto` no longer defaults to the legacy `/tmp/rzn.sock` planner pipe. `rzn_plan::BrokerClient` has a `native` transport that discovers fresh `broker_endpoint_v1.json` files, ignores dead worker PIDs, and self-spawns a standalone `rzn-browser` worker when every discovered endpoint is stale. `RZN_TRANSPORT=pipe` remains as an explicit legacy override only.
- Planner snapshot/static calls are bridged onto the worker MCP surface where possible (`browser.snapshot` / `browser.execute_step`) so autonomous mode can populate selector inventory through the same extension/native-host bridge as workflow runs.

### Proposed — `rzn-browser worker restart`

Add a CLI subcommand that:
1. Sends SIGTERM to any process listening on the bridge/worker sockets (read from broker endpoint file).
2. Waits up to 3s for graceful exit, then SIGKILL.
3. Removes stale socket files.
4. Does NOT spawn a new worker — the next `rzn-browser run` will do that lazily.

## Implementation Notes

### Bug 1 fix (stale-worker handshake)

**Touch points:**
- `crates/rzn_browser/src/native_runner.rs`: implemented pre-step native runtime self-heal for app-embedded/distributed CLI runs. Defaults: native-host restart enabled unless `RZN_RESTART_NATIVE_HOST=0` or `RZN_DISABLE_NATIVE_HOST_RESTART=1`; one preflight retry for `auto|spawn` unless `RZN_DISABLE_NATIVE_SELF_HEAL=1` or `RZN_NATIVE_SELF_HEAL_ATTEMPTS=<n>` overrides it. Self-heal uses the existing spawn lock to avoid concurrent reset/spawn races.
- `crates/rzn_browser_worker/src/main.rs`: worker health now uses `HEALTH_PING_TIMEOUT_MS=2500`; bridge keepalive is `10s/5s`; failed native-host request paths evict the bridge session immediately.
- Future: add `rzn-browser worker restart` subcommand as a user-visible manual fallback. It should call the same endpoint-scoped cleanup path, not broad process cleanup.

**Extension side:**
- `extension/src/background.ts`: on service-worker activation, unconditionally tear down any stale native port before calling `connectNative()` (belt-and-braces against Chrome caching the dead port).
- Emit a local native-host `ping` every 10s through the port. `crates/rzn_native_host/src/main.rs` answers with `ping_response` directly so the extension can detect a genuinely dead native port without routing heartbeat traffic through the browser worker.

### Bug 2 fix (session-scoped debugger)

**Touch points:**
- `extension/src/runtime/cdp_session_manager.ts` — owns `chrome.debugger` attach/detach keyed by tabId + session.
- `extension/src/actions/upload_file.ts` — uses the manager for `DOM.setFileInputFiles`.
- `extension/src/background.ts` — routes ordinary eval through JS-first scripting and explicit CDP actions through just-in-time attach/release.
- Worker side: `crates/rzn_browser_worker/src/` — verify a stable `session_id` field is passed on every action request so the extension manager can key correctly.

**Test plan for bug 2:**
- Workflow A: navigate → upload_file (no-op, empty path) → execute_javascript. Must succeed without keeping CDP attached.
- Workflow B: navigate → upload_file (real file) → execute_javascript. Must succeed.
- Workflow C: navigate → upload_file → click_element (CDP trusted) → execute_javascript → upload_file again → execute_javascript. Stress test multiple explicit CDP attach cycles with JS steps between them.

### Bug 3 fix (in-step navigation tolerance)

**Touch points:**
- `crates/rzn_browser_worker/src/` step executor: when a `Runtime.evaluate` returns `-32000 "Inspected target navigated or closed"` AND the script itself triggered a navigation (heuristic: contains `location.assign|location.href|location.replace`), treat as soft-success and wait for the new execution context (Runtime.executionContextCreated) up to step timeout.
- Workflow-side workaround (already landed in `claude_send.json` s3): `setTimeout(() => window.location.assign(target), 0); return { redirected: true }` — fire-and-return pattern.

**Acceptance:** `rzn-browser run claude send --param thread_id=<id> --param message_text="..."` succeeds without any special handling by the workflow author.

## Tasks & Status

- [x] Add native-run preflight self-heal: restart native host, reset stale worker endpoint, retry before first step (bug 1 mitigation).
- [x] Land worker-side keepalive tracking + health reflects liveness (bug 1).
- [x] Prune dead native-host bridge sessions immediately on request failure/timeout (bug 1).
- [ ] Land `rzn-browser worker restart` subcommand (bug 1).
- [x] Land extension-side native-port teardown + native-host keepalive echo (bug 1).
- [x] Build `CdpSessionManager` in extension and migrate `upload_file` to it (bug 2).
- [ ] Audit remaining CDP-backed actions and route them through `CdpSessionManager`.
- [ ] Wire reliable workflow/tab-end release for `CdpSessionManager`.
- [ ] Add soft-retry for `-32000 navigated or closed` in worker step executor (bug 3).
- [ ] Regression workflows A / B / C under `workflows/tests/`.
- [ ] Doc update: `README.md` troubleshooting section should link back here.

## What Works (Do Not Change)

- The three-process topology is correct — do not collapse worker+native-host into one process.
- The broker endpoint file (`~/Library/Application Support/rzn-browser/secure/broker_endpoint_v1.json`) is how the CLI finds the worker. Keep it.
- The `extension/dist-chrome/` stable copy at `~/Library/Application Support/RZN/extension/dist-chrome/` is what Chrome actually loads. Keep `make install` copying there.
- The current interim patch in `upload_file.ts` (detach only on error) is correct — it can be simplified once the manager exists, but it must not regress to always-detach-on-success in the meantime.

## Tried & Didn't Work

- **Killing only the native host** — Chrome immediately respawns it, which is fine, but the root-cause stale-worker state isn't touched. Tabs accumulate in `bridge_hosts` count as stale sessions.
- **Reloading just the extension** — doesn't fix the stale-worker case because the worker is still running and doesn't invalidate its accepted native-host connection.
- **Opening any page in Chrome to wake the service worker** — works as a wake signal for the extension, but the worker's handshake state is orthogonal; if that's stale, the wake signal reaches the extension but the two can't agree.

## Repro (captured 2026-04-24, Chrome 141.x, macOS 25.3)

Minimal repro of bug 1:
```bash
rzn-browser run claude recent-chats --param limit=1   # succeeds, worker PID W is created
# quit Chrome entirely, wait 30s, relaunch Chrome
rzn-browser run claude recent-chats --param limit=1   # hangs 45s, fails with "Timed out waiting for native host connection"
# health: bridge_connected=true, native_host_connected=false, extension_connected=false
kill <worker PID W>                                    # fixes it
rzn-browser run claude recent-chats --param limit=1   # succeeds again
```

Minimal repro of bug 2:
```bash
rzn-browser run claude send \
  --param message_text="smoke test" \
  --param attachment_file_paths="/tmp/rzn-smoke.txt"
# Fails at s8 (execute_javascript) with "Debugger is not attached to the tab with id: N"
# Same workflow without attachment_file_paths succeeds.
```

Minimal repro of bug 3:
```bash
rzn-browser run claude send \
  --param thread_id="<some existing thread id>" \
  --param message_text="smoke test"
# Fails at s3 (execute_javascript) with "Inspected target navigated or closed"
# (s3 runs window.location.assign to redirect into the existing thread.)
```

## Acceptance Criteria

A reviewer accepting this feature should verify:

1. `rzn-browser run claude recent-chats` succeeds on a fresh session **without** killing any process manually after Chrome restart.
2. `rzn-browser run claude send --param message_text=... --param attachment_file_paths=...` succeeds end-to-end (not just structurally). The Claude thread receives both the attachment and the prompt.
3. Regression workflow C (upload → evaluate → upload → evaluate) passes — proves `CdpSessionManager` handles multiple attach cycles.
4. Running `rzn-browser run claude recent-chats` immediately after `rzn-browser worker restart` succeeds. No leftover sockets, no 45s hang.
5. Extension `chrome://extensions/` Errors pane shows zero errors after a full CLI run.
