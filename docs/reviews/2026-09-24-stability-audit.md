# Stability audit — 2026-09-24

> **Status:** first tranche landed. See [Remaining work](#remaining-work-handoff) at the end
> for what is left before the live test.

Five read-only audits (extension service worker, content script / page bridge, native host,
supervisor, CLI + process glue) produced 90 findings. This file merges them into 22 tickets.
Finding IDs (NH-, SUP-, EXT-SW-, EXT-PAGE-, CLI-) are kept for traceability. Line numbers are
as of commit 9bce130.

## Root cause of "Native host timeout for supervisor_rpc"

Confirmed in code, three pieces together:

1. The native host awaits every `supervisor_rpc` inside its only stdin reader loop
   (`crates/rzn_native_host/src/main.rs:1441`), with no deadline (`main.rs:496-534`).
2. `runs.start` / `runs.replay` reply only after the whole workflow finishes
   (`crates/rzn_browser/src/supervisor.rs:1420`, `deadline: None`).
3. Workflow step replies and heartbeat pings arrive on that same blocked stdin reader.

Result: the dashboard times out at 10s, steps time out, two missed pings (~25s) make the
extension kill the host (`extension/src/background.ts:708-733`), and every in-flight run dies.
Any slow supervisor call (`diagnostics.export`, `logs.tail`) triggers the same chain.

## Design rules every ticket enforces

1. A reader loop only parses and dispatches. It never awaits remote I/O.
2. Every cross-process call has a deadline on the waiting side. Timeouts fail one request,
   not the whole process.
3. Anything longer than ~1s is a job: start returns an id, callers poll or subscribe.
4. Every flag, lock, lease, or pending entry is released by a guard (Drop / finally) or a TTL.
5. Liveness checks measure the thing that matters (bridge up, not just process up) and run on a
   path normal work cannot block.
6. Each layer restarts the layer below it. No user action required.
7. Every failure names the broken hop and the fix command.

## Phase 0 — stop the current error

### STAB-01 Native host: supervisor RPCs off the reader loop, with deadlines — P0, S
- **Findings:** NH-1, NH-2, NH-3, NH-14, SUP-1, EXT-SW-1 (root), CLI-1 (partial)
- **Change:**
  - `main.rs:1441`: `tokio::spawn` the forward; reply via `native_tx`. Reader never awaits it.
  - Wrap `call_supervisor_client` (`main.rs:496`) in `timeout` (10s default, per-method override
    table). Return structured `SUPERVISOR_RPC_TIMEOUT`.
  - Wrap `connect_supervisor_runtime` + `runtime.hello` read (`main.rs:389-415`) in a 5s timeout
    so `endpoint_manager_loop` falls through to retry.
  - `supervisor_rpc` without `payload.method` replies `success:false` (`main.rs:900`).
- **Acceptance:** test with a fake supervisor that never answers one RPC: pings and extension
  step responses still round-trip; the RPC fails with `SUPERVISOR_RPC_TIMEOUT` inside its deadline.

### STAB-02 Supervisor: runs.start returns a run id; run slot guarded — P0, M
- **Findings:** SUP-2, SUP-15, SUP-18, CLI-11, CLI-8 (partial)
- **Change:**
  - `start_local_run` (`supervisor.rs:1371`): atomic check-and-set on `now_running`
    (`supervisor_control.rs:31`), spawn `execute_workflow`, return `{run_id}` immediately.
  - Run slot released by an RAII guard inside the spawned task (panic, cancel, drop all release).
  - Overall run deadline (replace `deadline: None`, `supervisor.rs:1415`).
  - Write a `running` record to the run store at start; store failures after the run are warnings,
    not RPC errors.
  - "a run is already in progress" includes `run_id`, elapsed time, and the cancel command.
- **Acceptance:** `runs.start` returns in <500ms for a 60s workflow; `runs.get` shows progress;
  a panicking step leaves the slot free; two concurrent `runs.start` → exactly one accepted.
- **Depends on:** dashboard already routes to `#runs/<id>` and polls `runs.get`; verify.

### STAB-03 Extension: honest liveness and honest RPC errors — P0, M
- **Findings:** EXT-SW-1, EXT-SW-2, EXT-SW-3, EXT-SW-6, EXT-SW-8, EXT-SW-9, EXT-SW-15
- **Change:**
  - Heartbeat (`background.ts:708-733`): a fresh `native_host_heartbeat` (<2× interval) counts
    as alive; reconnect only when ping and stdout heartbeat are both stale.
  - Per-method RPC timeouts instead of flat 10s (`background.ts:8007`); fix the false comment in
    `ui/rpc.ts`.
  - Transport failures carry `error_code` / `transport_error:true`; `ui/rpc.ts` throws
    `SupervisorUnreachable` for them; popup and fleet page show "unreachable" + retry, not the
    enroll form (`fleet/index.ts:11`).
  - `disconnectNativePortAfterResponseFlush` (`background.ts:832-837`) checks `nativePortEpoch`.
  - Drop late `*_response` messages whose callback timed out (tombstone set) instead of routing
    them to broker dispatch (`background.ts:569-570, 3933`).
  - UI polling (popup 2s, fleet 5s, logs 2s, badge alarm) schedules the next tick after the
    current call finishes; skip if in flight.
  - Timeout error text names the RPC method and a next step.
- **Acceptance:** with a supervisor call stalled 60s, the port is not recycled and other RPCs
  still complete; host down → popup shows "unreachable" within one poll.

## Phase 1 — no restart-required states

### STAB-04 Page bridge: install once, survive DOM churn, never re-execute — P0, M
- **Findings:** EXT-PAGE-3, EXT-PAGE-5, EXT-PAGE-6
- **Why P0:** in production builds main-world evals can run N+1 times (e.g. a prompt sent twice).
- **Change:**
  - `window.__rznPageBridgeInstalled` guard in `pageBridge.ts`; readiness check
    (`background.ts:2549-2570`) tests that flag, not `__rznExecuteStep` (test-only).
  - Both sides observe `document.documentElement` and re-bind when the container is replaced
    (`pageBridge.ts:699-711`, `contentScript.ts:6475-6495`); `callPageBridge` sets owner metadata
    (`contentScript.ts:715-723`).
  - `ensureContentReady` also injects `pageBridge.js` (`background.ts:2336`).
  - `eval_main_world` falls back to another backend only when the bridge is absent; script
    errors and timeouts pass through (`contentScript.ts:1413-1422, 3776-3779`).
- **Acceptance:** e2e run on a **production-flavor** build: 5 CDP evals then 1 main-world eval →
  the script runs exactly once; removing the container mid-session still works.

### STAB-05 Never re-send a step after navigation — P0, S
- **Findings:** EXT-PAGE-4
- **Change:** `sendMessageTopFrame` (`background.ts:2407-2426`) falls back only on "Receiving end
  does not exist". "Message channel closed" maps to non-retriable `NAVIGATED_DURING_STEP` with
  `current_url`. Never broadcast `execute_step` without a frame id.
- **Acceptance:** a click step that navigates returns `NAVIGATED_DURING_STEP` (or success) and
  runs exactly once; no iframe receives it.

### STAB-06 Debugger ownership: one owner, always detachable — P0, M
- **Findings:** EXT-PAGE-1, EXT-PAGE-8, EXT-PAGE-9, EXT-PAGE-10, EXT-PAGE-12, EXT-SW-7
- **Change:**
  - `detachFromTab` always calls `chrome.debugger.detach`; "not attached" is success
    (`frameRouter.ts:183-186`).
  - `markTabDetached` only from `onDetach`, not from lifecycle errors (`cdpClient.ts:104-108`,
    `cdp_session_manager.ts:134-135`, `frameRouter.ts:135-161`).
  - "Another debugger is already attached": detach once, re-attach; else throw
    `CDP_ATTACH_CONFLICT` (`frameRouter.ts:138-141`).
  - `frameRouter` is the single ref-counted owner; `cdpHelper.ts` and `cdp_session_manager.ts`
    route through it.
  - One in-flight attach promise per tab (`frameRouter.ts:82-132`).
  - Reset `domainRefs` on detach (`cdpClient.ts:31, 146-151`).
  - Screenshot path uses the per-tab lock and command timeouts (`background.ts:3570-3611`).
  - Startup reconcile detaches via `chrome.debugger.detach({tabId})` directly
    (`background.ts:172-176`).
- **Acceptance:** CDP eval that navigates the page → next CDP step on that tab succeeds;
  SW restart with a leased tab → infobar gone after reconcile.

### STAB-07 CDP lock: per tab, bounded, every command timed — P0, S
- **Findings:** EXT-PAGE-2, EXT-PAGE-7, EXT-SW-5, EXT-SW-17
- **Change:**
  - `withCdpLock` (`background.ts:1888-1901`) becomes per-tab with an acquire deadline.
  - Timeouts on `frameRouter.sendCommand` (`frameRouter.ts:339-360`),
    `CdpSessionManager.sendCommand` (`cdp_session_manager.ts:127-146`), `cdpHelper.send`
    (`cdpHelper.ts:122-139`) — reuse the pattern in `cdpClient.ts:73-82`.
  - `ensureContentReady` CDP reconcile is best-effort, not blocking (`background.ts:2349-2360`);
    its `sendMessage` / `executeScript` awaits use `withMessageTimeout` with the remaining budget.
  - Aborting a guarded side effect releases the lock (`background.ts:1418-1459`).
- **Acceptance:** a 5-minute CDP eval on tab A does not delay DOM steps on tab B.

### STAB-08 A timeout fails one request, not the process — P1, S
- **Findings:** NH-4, NH-13, SUP-4
- **Change:**
  - Native host (`main.rs:1141-1174`): on extension-call timeout, fail that request only. Shut
    down only after N consecutive timeouts or a failed ping round-trip.
  - Clamp `timeout_ms` to [1s, 10min] (`main.rs:816-820`).
  - Supervisor (`supervisor.rs:3281-3292`): restart the bridge only after N consecutive failed
    health pings, never on a step or notice timeout (`fleet_run_notice` 500ms, `:1321`).
- **Acceptance:** one slow step times out; concurrent steps on other sessions complete; host pid
  unchanged.

### STAB-09 Native host reports bridge truth; routes control to the same supervisor — P1, S
- **Findings:** NH-5, NH-16
- **Change:** `ping` and `runtime_bridge_get_status` (`main.rs:932-969`) report
  `bridge_connected`, `last_bridge_ok_ms`, `supervisor_boot_id` (drop hardcoded
  `available:true`). Extension recycles the port if the bridge has been down > N seconds.
  Control RPCs go to the endpoint recorded from the bridge hello (`main.rs:419-425`).
- **Acceptance:** kill the supervisor while Chrome is open → extension sees `bridge_connected:false`
  and the chain recovers without user action.

### STAB-10 Native host task lifecycle — P1, M
- **Findings:** NH-6, NH-7, NH-8, NH-11, NH-12
- **Change:**
  - On upstream EOF: remove the `active_bridges` key and abort the writer at once; session tasks
    get a per-connection cancel token (`main.rs:1060, 1221-1223`).
  - Send the shutdown reason before awaiting the writer (`main.rs:1022-1036, 1222-1226`).
  - `select!` also on `endpoint_manager` and `native_writer_task` handles (`main.rs:1507`);
    `active_bridges` released by a Drop guard.
  - Forced shutdown drains pending with a reason, flushes, `std::process::exit(0)` (`main.rs:1520`).
  - Bounded channels (`main.rs:984, 1368`); stdout write under `timeout` (`main.rs:577-579`).
- **Acceptance:** restart the supervisor mid-run → new bridge within 2s regardless of in-flight
  call timeouts.

### STAB-11 Supervisor bridge registry hygiene — P1, M
- **Findings:** SUP-7, SUP-13, SUP-16, CLI-2 (TTL part)
- **Change:**
  - On register, shut down and close older bridges with the same `browser_instance_id`
    (`supervisor.rs:3333-3377`); close the stream on bridge restart.
  - Periodic supervisor→bridge ping to catch half-open bridges.
  - Prune `native_bridge_health` on unregister; TTL sweep for `sessions`; bounded per-bridge
    write channel with write timeout (`:5136, :5158`).
  - Frame errors are logged and answered, not treated as silent disconnects (`:5165-5168, :5054`).
- **Acceptance:** wedge an old host (SIGSTOP) and start a new one → untargeted `runs.start`
  succeeds, no `AMBIGUOUS_BROWSER_TARGET`.

### STAB-12 Supervisor process lock, logs, force-stop — P1, M
- **Findings:** SUP-6, CLI-1, CLI-3, CLI-7, CLI-15, NH-10
- **Change:**
  - Replace the PID-file check (`supervisor.rs:4736-4816`) with an `flock` held for the process
    lifetime; handle SIGTERM in `serve` (`:4937`).
  - `ensure_running` (`:4969-4993`): on unresponsive supervisor, read the lock pid and report
    "supervisor pid N unresponsive — run `rzn-browser supervisor shutdown --force`"; do not spawn.
  - Add `supervisor shutdown --force` (SIGTERM then SIGKILL via the lock pid).
  - Spawned supervisor stderr → `<app_base>/run/supervisor.log` (both spawners:
    `supervisor.rs:5007-5010`, `rzn_native_host/src/main.rs:347-349`); include the tail in
    "did not become ready".
  - Cache the token in memory; rewrite the file if missing (`:5020`).
  - Native host spawns only when the socket is absent/refused, with backoff + jitter; reap the
    child (`rzn_native_host/src/main.rs:1287-1300, 355`).
- **Acceptance:** `kill -9` the supervisor, reboot-style stale lock with a reused pid, and a
  SIGSTOPped supervisor all recover with one command or automatically.

### STAB-13 Run store crash safety — P1, S
- **Findings:** SUP-8, SUP-19, CLI-17
- **Change:** `load_index` reads bytes, splits on `\n`, skips bad lines (`run_store.rs:276-277`).
  Startup `refresh_snapshot_cache` is best-effort (`supervisor.rs:4864`). `flock` the index for
  appends and GC rewrites (`run_store.rs:83, 288-296`).
- **Acceptance:** append a torn, non-UTF-8 line to `index.jsonl` → supervisor starts, lists runs.

### STAB-14 Cancel actually cancels, end to end — P1, L
- **Findings:** SUP-3, SUP-5, SUP-14, SUP-20, CLI-2, EXT-PAGE-11
- **Change:**
  - Per-run cancellation token checked in the transport `select!` (`supervisor.rs:3811`), in
    `wait_for_timeout` (`workflow_runner/mod.rs:285`), and in the bridge reconnect wait (`:2658`).
  - Dropping a step future removes its `native_bridge_pending` entry (`:3147`) and sends a cancel
    to the extension; the extension forwards `RZN_CANCEL_REQUEST` to the content script
    (`background.ts:2429-2445`).
  - Inner bridge timeout strictly shorter than the outer watchdog (`workflow_runner/mod.rs:283`,
    `supervisor.rs:5365-5377`).
  - `session_close` runs outside the timed future (`workflow_runner/mod.rs:471-492`).
  - `runs.cancel` honours `run_id` and reports "no run" (`supervisor_control.rs:64-71`);
    `automation.resume` does not clear a pending cancel.
  - CLI Ctrl-C cancels the step and calls `browser.session_close`.
  - `handle_connection` aborts a request when the client disconnects (`supervisor.rs:5062`).
- **Acceptance:** cancel during a 60s `wait_for_timeout` and during a hung step → run ends within
  2s, tab closed, content script stops typing.

### STAB-15 One run slot for dashboard, fleet, and CLI runs — P1, M
- **Findings:** SUP-9, CLI-14
- **Depends on:** STAB-02, STAB-14
- **Change:** `SupervisorState` owns one run slot and cancel token. Fleet jobs
  (`supervisor_fleet.rs:1051`) and CLI runs (`native_runner.rs:85`) acquire it and honour pause /
  cancel per step.
- **Acceptance:** fleet job running → dashboard start is rejected with the fleet run id; popup
  Stop stops a fleet job.

### STAB-16 Extension reconnect backoff that works — P1, S
- **Findings:** EXT-SW-4, EXT-SW-13
- **Change:** reset `reconnectAttempts` only after the first pong or stdout heartbeat
  (`background.ts:3983`). All wake paths (keepalive `:6392`, `RZN_WAKE_NATIVE` `:8031`,
  `ensureNativeHostConnected` `:584`) go through `scheduleReconnect`. Add jitter and a cap.
  "Host not found" / "forbidden" → terminal state with the `native-host doctor` hint.
  `chrome.alarms.get` before `create` (`:6393`, `:224`).
- **Acceptance:** remove the host manifest → one clear error, no host respawn loop.

## Phase 2 — visibility and upgrade safety

### STAB-17 Logs that exist — P1, S
- **Findings:** NH-9, NH-15, SUP-16 (logging), SUP open question on `tracing-log`
- **Change:** native host default filter `info`, rotating file log under
  `<app_base>/logs/native-host.log` with pid and boot id (`main.rs:1335`). Counters for lost
  upstream responses and late extension responses in the ping payload. Verify supervisor `log::`
  macros reach the log buffer (enable `tracing-log`, `Cargo.toml:30`).
- **Acceptance:** after a forced bridge drop, the reason is in a file on disk.

### STAB-18 Version skew detection — P1, S
- **Findings:** CLI-4
- **Change:** `version` + `exe` in the client handshake, native-host hello, and `runtime.status`
  (`supervisor.rs:2067-2086, 5238-5241, 5099-5101`). CLI warns and offers a restart on mismatch.
  `setup.sh` reuses the release installer's shutdown-and-restart.
- **Acceptance:** rebuild the CLI while an old supervisor runs → CLI prints the mismatch and the
  restart command.

### STAB-19 `runtime doctor` names the broken hop — P1, M
- **Findings:** CLI-5, CLI-12, CLI-13
- **Change:** one command, checks in order, each with a fix command: lock pid alive and
  responsive, versions equal, CLI socket path == native host self-test `candidate_endpoints[0]`,
  native host signature valid, bridge connected, extension connected. Only infer app base from
  `<X>/bin` when `<X>` has `run/`, `secure/`, or `extension/` (`runtime_paths.rs:39-46`).
  Bridge-down message gets the doctor hint and `app_base` (`supervisor.rs:5481-5484`).
- **Acceptance:** each failure injected by the chaos harness (STAB-21) is named correctly.

### STAB-20 CLI never silent for more than 5s — P2, S
- **Findings:** CLI-6, CLI-16
- **Change:** print `[HEAL]`, `[SNAPSHOT]`, `[CLOSE]`. Cap failure snapshot and close at ~5s;
  skip the snapshot after a transport timeout. Heal gets one server-side deadline.

### STAB-21 Chaos harness — P1, M
- **Change:** a test script that, during a running workflow, kills or SIGSTOPs each of: native
  host, supervisor, extension service worker (`chrome.runtime.reload` or DevTools stop), and
  navigates the tab mid-step. Assert: the run fails with a named error within N seconds, and the
  next run succeeds with no manual step. Run in CI nightly.
- **Acceptance:** all scenarios green; each STAB ticket above adds its regression case here.

## Phase 3 — hygiene batch

### STAB-22 Remaining P2 / edge findings — P2, M (split as you go)
- **Fleet and cloud loops:** SUP-10 (enroll/disable needs restart), SUP-11 (job panic silent),
  SUP-12 (websocket idle timeout + per-command tasks), CLI-9 (`cloud run-workflow` sync + retry
  double-run).
- **MCP server:** CLI-10 (`ready_checked` cached forever, sessions leak on EOF, serial loop).
- **Extension:** EXT-SW-10 (`session_close` queued behind step; dead `native_input`),
  EXT-SW-11 (sessions persisted across browser restart), EXT-SW-12 (`observeCache` never evicts),
  EXT-SW-14 (pairing fetch timeout), EXT-SW-16 (dashboard route race leaks timers),
  EXT-PAGE-13 (orphaned content scripts reconnect forever), EXT-PAGE-14 (duplicate content-script
  instances), EXT-PAGE-15 (typed retryable errors), EXT-PAGE-16 (leaks, cached failure replay).
- **Supervisor / CLI:** SUP-17 (actionable messages), CLI-18 (curl `--max-time`, self-test
  timeout), CLI-19 (`rzn` vs `RZN` Linux dir), CLI-20 (document: no launchd service; native host
  + `ensure_running` restart the supervisor).
- **Native host:** NH-17 (backoff + jitter on the 1s poll).

## Suggested order

STAB-01 → STAB-02 → STAB-03 fix the reported error. STAB-04 and STAB-05 next: they cause
duplicate side effects (double posts). Then STAB-06, STAB-07, STAB-21 (so every later fix gets a
regression case), then the rest of Phase 1, then Phase 2.

## Open questions from the audits

- Does Chrome keep a `chrome.debugger` attachment across a service-worker termination (not an
  extension reload)? Sets the severity of STAB-06's startup path.
- Does `chrome.tabs.sendMessage` to a frozen tab hang or reject?
- Does Chrome kill a native host whose port was disconnected while the host is not reading stdin?
- No renderer-crash ("sad tab") detection exists. Decide: auto-reload the tab or fail the run.
- Does the cloud control plane drop the handler on client disconnect (CLI-9)?

## Remaining work (handoff)

State after the first tranche (2026-09-24). Tests green at that point: `rzn-browser` 290 passed /
1 ignored, `rzn-native-host` 27 passed, extension vitest 143/143, `tsc` no new errors vs the base
commit (51 pre-existing errors remain).

### Done — do not redo

| Ticket | What landed |
|---|---|
| STAB-01 | Native host spawns each `supervisor_rpc` off the stdin reader; per-method deadlines (10s default, `logs.tail` 30s, `diagnostics.export` 60s); 5s connect/hello timeout; `INVALID_SUPERVISOR_RPC` on missing method. |
| STAB-02 | `runs.start` / `runs.replay` return `{run_id}` at once; run slot is an atomic `try_begin_run` + `LocalRunGuard` (Drop); 30 min run deadline; `runs.get` reports in-flight runs; store failures after a run are warnings. |
| STAB-03 | Heartbeat accepts a fresh stdout heartbeat; per-method UI timeouts; `SUPERVISOR_UNREACHABLE` transport errors; epoch check on delayed disconnect; late control responses dropped; fleet/logs polling uses `setTimeout` chains. |
| STAB-04 | `__rznPageBridgeInstalled` guard; readiness checks the flag; both bridge sides re-bind when the container is replaced; `ensureContentReady` injects `pageBridge.js`; main-world eval errors pass through instead of re-executing. |
| STAB-05 | `sendMessageTopFrame` only falls back on "Receiving end does not exist"; channel/port closed → `NAVIGATED_DURING_STEP`; no frame-less broadcast. |
| STAB-07 | Per-tab CDP lock with a 10s acquire deadline (FIFO preserved on timeout); `frameRouter.sendCommand` has a 30s timeout; `CdpSessionManager` and `cdpHelper` route through `frameRouter`. |
| STAB-13 | `index.jsonl` read is byte-tolerant; startup snapshot refresh is best-effort; `flock` on `index.lock` for append and GC. |
| STAB-18 | `version` + `exe` in all three handshakes and `runtime.status`; client compares versions. |

### Left to build

Ordered by impact. **P0 = must land before the live test.**

**1. STAB-08 supervisor half — P0, S.** The native host no longer shuts itself down on a timeout,
but the supervisor still restarts the bridge on every extension-call timeout
(`supervisor.rs` ~3380-3391) and on the host's `-32003` timeout error (~3399-3416, via
`native_host_bridge_transport_error_cause`). Net effect: one slow step still kills the native host
and every other in-flight call. Change: restart only after N consecutive failed **health pings**
(`cmd == "ping"`); step and notice timeouts fail that request only. Test: one slow step times out,
a concurrent step on another session succeeds, the host pid is unchanged.

**2. STAB-09 remainder — P1, S.**
- `handleNativeHostStdoutHeartbeat` resets `reconnectAttempts` (`background.ts`). While the
  supervisor is down, the heartbeat marks the port bad every ~20s, and the backoff never grows →
  endless port recycling. Reset the counter only on a pong with `bridge_connected: true`.
- NH-16: control RPCs pick their own endpoint (`call_supervisor_client`) instead of the endpoint
  the bridge connected to. Record the endpoint + `supervisor_boot_id` from the bridge hello and
  route control RPCs there.

**3. STAB-10 native host task lifecycle — P1, M.** Not started. Upstream EOF must drop the
`active_bridges` key and abort the writer at once (today it waits for every in-flight session
task); send the shutdown reason before awaiting the writer; `select!` on `endpoint_manager` and
writer handles; Drop guard for `active_bridges`; forced shutdown drains pending and calls
`process::exit`; bounded channels; stdout write timeout. Test: restart the supervisor mid-run →
new bridge within 2s.

**4. STAB-14 cancel end to end — P1, L.** Not started. Per-run cancellation token through the
transport `select!`, `wait_for_timeout`, and the bridge reconnect wait; dropping a step future
removes its pending entry and sends a cancel to the extension, which forwards
`RZN_CANCEL_REQUEST` to the content script; inner bridge timeout < outer watchdog; `session_close`
outside the timed future; CLI Ctrl-C cancels and closes the session; client disconnect aborts
the request. `runs.cancel` now accepts `run_id` — verify it returns "no run" when idle.

**5. STAB-12 remainder — P1, S.**
- `supervisor shutdown --force` (SIGTERM then SIGKILL via the pid in the lock file).
- `ensure_running`: when the lock is held but the supervisor does not answer, report
  "supervisor pid N unresponsive — run `rzn-browser supervisor shutdown --force`" instead of
  spawning a second one.
- Cache the token in memory; rewrite the file if it disappears.
- NH-10: native host spawns the supervisor only when the socket is absent or refused, with
  backoff + jitter; reap the child.

**6. STAB-11 remainder — P1, M.** Done: evict older bridges with the same `browser_instance_id`
on register. Left: periodic supervisor→bridge ping, prune `native_bridge_health` on unregister,
TTL sweep for `sessions`, bounded per-bridge write channel with a write timeout, log frame errors
instead of silent disconnects.

**7. STAB-15 one run slot for dashboard, fleet, CLI — P1, M.** Not started. Depends on 4.

**8. STAB-16 remainder — P1, S.** Done: jitter, reset on first pong, "host not found" / "forbidden"
stop the `onDisconnect` retry. Left: the content keepalive, `RZN_WAKE_NATIVE`, and
`ensureNativeHostConnected` still call `connectToNative()` directly, so the terminal state is
bypassed and backoff is skipped. Route all of them through `scheduleReconnect` and respect a
terminal flag. `chrome.alarms.get` before `create` for the keepalive and lease-sweep alarms.

**9. STAB-06 remainder — P1, S.** Done: always detach, one in-flight attach per tab,
"Another debugger" → detach + re-attach or `CDP_ATTACH_CONFLICT`, `domainRefs` reset on detach,
startup reconcile detaches directly. Left: screenshot path (`captureScreenshotForTab`,
`background.ts` ~3613) should take the per-tab lock; add a test that a CDP eval which navigates
leaves the next CDP step on that tab working.

**10. STAB-17 remainder — P1, S.** Done: native host logs to `<app_base>/logs/native-host.log`
at `info`; spawned supervisors log stderr to `<app_base>/run/supervisor.log`. Left: counters for
lost upstream / late extension responses in the ping payload; confirm supervisor `log::` output
reaches the log buffer (`tracing-log` feature, `Cargo.toml`).

**11. STAB-04 polish — P2, S.** `ensureContentReady` re-injects `pageBridge.js` every call; the
guard throws "already installed" and logs a warning each time. Check the flag first and skip.
Note: `window.__rznPageBridgeInstalled` is visible to page scripts — acceptable, but a
bot-detection surface.

**12. STAB-19 runtime doctor, STAB-20 CLI progress, STAB-21 chaos harness, STAB-22 hygiene
batch.** Not started. Build STAB-21 before or alongside items 3-7 so each gets a regression case.

### Missing regression tests for landed work

- CDP lock: a waiter that times out must not let the next waiter run while the holder is active.
- `sendMessageTopFrame`: both Chrome "closed before a response" messages map to
  `NAVIGATED_DURING_STEP`.
- Heartbeat: fresh stdout heartbeat + missed ping → no reconnect; `bridge_connected:false` → counts
  as a miss.
- Supervisor: `runs.start` returns before the workflow ends; a panicking run frees the slot.

### Live test checklist (after items 1-2 land)

1. Dashboard: start a workflow that runs > 60s. Expect the run id at once, progress in
   `#runs/<id>`, no "Native host timeout", host pid unchanged.
2. Same, plus open the logs page with auto-refresh on. Expect no port recycle.
3. Kill the supervisor mid-run. Expect the run to fail with a named error, the supervisor to
   respawn, and the next run to succeed without a browser restart.
4. `kill -9` the native host. Expect Chrome/extension to reconnect and the next run to succeed.
5. Run a CDP-eval workflow that navigates (e.g. ChatGPT send). Expect exactly one send and the
   next CDP step on that tab to work.
6. Check `<app_base>/logs/native-host.log` and `<app_base>/run/supervisor.log` explain 3 and 4.

### Tranche 2 status (supersedes the matching items above)

Landed: STAB-08 (bridge restarts only after 2 consecutive failed health pings), STAB-09 (stdout
heartbeat no longer resets backoff; control RPCs use the active bridge endpoint), STAB-12
(`supervisor shutdown --force`, SIGTERM handling, locked-but-unresponsive detection that waits
out a starting supervisor), STAB-16 (all wake paths go through `scheduleReconnect`; terminal state
on host-not-found; alarms get-before-create), STAB-06/04 polish (screenshots take the per-tab
lock; `pageBridge.js` injected only when the flag is absent), STAB-17 (`tracing-log` enabled).

Still open from those tickets:

- **STAB-14 is now end-to-end for explicit cancel and CLI Ctrl-C.** `runs.cancel` sends an
  out-of-band `RZN_CANCEL_REQUEST` which bypasses the workflow queue, aborts the matching broker
  lease, and reaches the content script. CLI Ctrl-C sends the same cancel then closes its tracked
  session. Client disconnect now drops the in-flight dispatch future. A dropped bridge call is
  removed on its bounded inner timeout or response; an async Drop hook was deliberately not added.
- **STAB-10 now has EOF release, bounded native/upstream channels, a 5s stdout-write timeout, and
  pending-call drain during forced shutdown.** Per-connection calls are supervised by a `JoinSet`
  and aborted on disconnect; the main task selects endpoint-manager/writer termination and exits
  explicitly. `active_bridges` is still released explicitly rather than through an async Drop guard.
- **STAB-11 periodic health detection landed.** The supervisor pings every registered bridge every
  10s, so the two-strike restart rule no longer depends on readiness traffic. Health entries are
  pruned at unregister, disconnected sessions expire after one hour, and malformed JSON frames get
  a JSON-RPC parse error. Supervisor-side bridge writers remain unbounded.
- **STAB-16 terminal recovery landed.** Popup Retry sends an explicit `RZN_WAKE_NATIVE` that clears
  terminal state and reconnect backoff before scheduling a new connection attempt.
- **STAB-12:** the supervisor caches its token in memory and restores a deleted token file on the
  next connection. NH-10 spawn error classification, jitter, and child reaping remain open.
- **STAB-15 landed:** dashboard, fleet, and direct CLI runs claim the same supervisor slot; fleet
  observes the shared cancel flag; stale releases cannot free another run's slot.
- **STAB-17 counters landed:** native-host pings report lost upstream and late extension responses.
- **STAB-19/20 landed as the smallest shared implementation:** `runtime doctor` reuses the existing
  native-host/runtime chain diagnosis, app-base inference rejects unrelated `bin` parents, and CLI
  emits `[HEAL]`, `[SNAPSHOT]`, and `[CLOSE]` with 5s snapshot/close bounds.
- **STAB-21 started:** scheduled/manual extension E2E now builds the real binaries, enables the
  native-host smoke lane, and includes a kill-and-replace supervisor recovery case. Native-host
  kill, service-worker stop, and mid-step navigation chaos scenarios remain open.
- **STAB-22 partial:** MCP readiness is no longer cached forever; observe cache is TTL/size bounded;
  cloud pairing/command fetches have a 10s abort deadline; Linux app-base casing already matched the
  installer. The remaining fleet/cloud, MCP concurrency/session, and extension lifecycle findings
  still need separate changes and live coverage.

### Tranche 5 status (final sweep)

Landed:

- **Native host (NH-10):** only NotFound/ConnectionRefused on the supervisor socket allows a spawn;
  any other connect error means a supervisor exists and the host waits. Spawn cooldown 5s→60s with
  ±20% jitter; the spawned child is reaped on a thread.
- **Supervisor:** a supervisor is only ever spawned when its lock is free; a locked-but-silent one
  gets a 5s grace, then a `shutdown --force` hint. The process lock is taken before the token is
  read (no concurrent-start token race). A CLI run slot records its owner pid and is reclaimed when
  that process is dead. Bridge writes have a 10s timeout that clears the bridge (SUP-13); cloud
  dispatch runs each request concurrently (SUP-12); paused/claim conflicts name the command that
  fixes them (SUP-17). New `fleet.start` RPC; `rzn fleet enroll` calls it, so no supervisor restart.
  `status.snapshot` now carries the real fleet status (was always null).
- **Fleet/cloud:** the fleet loop is restartable, a panicking job reports `job_crashed`, a
  panicking loop reports `crashed`. The cloud websocket pings every 20s and reconnects after 60s
  of silence; commands run on their own ordered task.
- **Extension lifecycle:** `session_close` is control-plane and cancels its leases; boot-id tagged
  sessions; orphaned content scripts stop and a new instance tears down the old one; per-tab
  frame-router state; failed step responses are not cached; `RESTRICTED_URL` is a typed,
  non-retryable error; `native_input` path removed.
- **MCP/CLI:** MCP closes the sessions it opened on EOF; `cloud run-workflow` has a per-request
  deadline; curl calls have `--max-time`; doctor self-test has a 10s deadline.
- **`rzn-browser heal`** is now a repair ladder: re-register a broken native-host manifest,
  force-restart a supervisor that holds its lock but does not answer (`--restart` forces it), heal
  the bridge (retrying once after a supervisor restart), then rerun the doctor and exit non-zero
  with the failing checks if anything still needs a human.
- **STAB-21 chaos cases written:** native-host SIGKILL, `chrome.runtime.reload()` mid-run, and a
  navigating click that must run exactly once.

Still open:

- The native-host smoke/chaos lane does not run on branded Chrome ≥137 (it ignores
  `--load-extension`); the harness is being moved to Chrome for Testing. Until then the chaos cases
  are unverified live.
- MCP request cancellation (the MCP loop is serial).
- `download_catalog_source` has no timeout.
- Supervisor bridge writer channels are unbounded (the 10s write timeout bounds a stuck bridge).
