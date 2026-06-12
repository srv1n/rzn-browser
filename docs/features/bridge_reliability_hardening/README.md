# Bridge Reliability Hardening

**Status:** active, tracked by Tusker epic `BRR`
**Owner:** sarav
**Related code:** `crates/rzn_browser/src/native_runner.rs`, `crates/rzn_browser/src/supervisor.rs`, `crates/rzn_native_host/src/main.rs`, `extension/src/background.ts`

## Overview

`native_host_disconnected` with helper text like "Native-host bridge is connected but the loaded extension failed the readiness contract" means the supervisor still has a native-host bridge handle, but the extension service worker did not answer the readiness ping in time or did not advertise the required capability. This is a distributed readiness problem, not a single reload bug: Chrome owns native-host lifetime, MV3 owns service-worker suspension, the supervisor owns the cached bridge handle, and the CLI currently gives the normal run path one short probe before aborting.

The goal is boring reliability: cold start after idle should heal automatically, stale bundles should report a precise reload/upgrade message, and true bridge-down states should fail with the component that is actually down.

## Flow Diagrams

### Runtime path

```text
rzn-browser run
    |
    v
supervisor unix socket
    |
    v
native-host bridge over stdio
    |
    v
Chrome native messaging
    |
    v
MV3 service worker background.ts
    |
    v
content script / tab / CDP
```

### Readiness contract

```text
supervisor
  sends: cmd="ping"
  waits: DEFAULT_BRIDGE_PROBE_TIMEOUT_MS = 1500
  expects:
    result.bridge_contract_version >= 8
    result.capabilities.content_keepalive_port = true
    result.capabilities.native_host_stdout_heartbeat = true
    result.capabilities.native_roundtrip_ping_health = true
    result.capabilities.native_port_epoch_fencing = true
    result.capabilities.workflow_session_epoch_fencing = true
    result.capabilities.broker_watchdog = true
    result.capabilities.request_lease_cancellation = true
    result.capabilities.watchdog_queue_unblock = true
    result.capabilities.epoch_chain_identity = true
    result.capabilities.native_control_epoch_fencing = true
    result.capabilities.supervisor_bridge_response_fencing = true
    result.capabilities.health_beacon_v2 = true
    result.capabilities.auxiliary_path_lease_guards = true
    result.capabilities.port_scoped_disconnect_suppression = true
    result.capabilities.native_message_frame_cap = true

extension service worker
  replies over chrome.runtime.Port connected to rzn_native_host
```

### Failure shape

```text
native_host_bridge.connected = true
native_host_bridge.responsive = false

Possible causes:
  1. MV3 service worker asleep or mid-restart
  2. supervisor cached a stale native-host fd
  3. loaded extension bundle is old and lacks required capability
```

### Readiness diagnostics

`runtime.ensure_ready` preserves the existing top-level `ok`, `ready`, `error`,
`remediation`, and `native_host_bridge` fields, and now adds a typed
`diagnostic` object when readiness is false:

```json
{
  "diagnostic": {
    "cause": "bridge_down | transport_timeout | stale_extension_bundle | service_worker_unresponsive",
    "inferred": true,
    "message": "short observed failure",
    "action_text": "single CLI-facing next action",
    "observed": {
      "native_host_bridge_connected": true,
      "native_host_bridge_responsive": false,
      "probe_transport_ok": false,
      "expected_bridge_contract_version": 8,
      "loaded_bridge_contract_version": 7,
      "bridge_contract_version_ok": false,
      "expected_capabilities": {
        "content_keepalive_port": true,
        "request_lease_cancellation": true,
        "watchdog_queue_unblock": true,
        "epoch_chain_identity": true,
        "native_control_epoch_fencing": true,
        "supervisor_bridge_response_fencing": true,
        "health_beacon_v2": true,
        "auxiliary_path_lease_guards": true,
        "native_message_frame_cap": true
      },
      "loaded_capabilities": {},
      "loaded_extension_build_signature": "..."
    }
  }
}
```

The normal `rzn-browser run` preflight uses `diagnostic.action_text` when it
fails before any workflow step is dispatched. Stale bundles therefore ask for a
bundle reload, bridge-down states ask for Chrome/extension startup, and probe
timeouts ask for heal/retry before reload.

Readiness also separates bridge facts that used to be blurred together:

| Field | Meaning |
|---|---|
| `native_host_bridge.connected` | Supervisor currently has a native-host bridge handle. |
| `native_host_bridge.transport_ok` | Active ping reached a responsive extension/native-host transport. |
| `native_host_bridge.required_capabilities_ok` | Loaded extension satisfies the current bridge contract version and required capability gate. |
| `native_host_bridge.capability_policy.mode` | Current policy; today this is `global_readiness_gate` for bridge contract v8 and the required bridge hardening capabilities. |

Skipped or absent probes do not satisfy the global readiness gate. Without an
active ping response, transport and required capability health are reported as
false rather than assumed.

## Decision Record

| # | Decision | Rationale |
|---|---|---|
| 1 | Normal `rzn-browser run` must call heal before aborting on pre-step bridge unreadiness. | Users should not reload an extension for a recoverable cold-start race. Heal already has longer retry semantics; the run path does not. |
| 2 | The extension should attempt native messaging reconnect on every SW wake. | Top-level SW code is the one thing Chrome runs on wake. Waiting for an alarm or external event leaves a null native port window. |
| 3 | Probe timeout should mark the cached bridge stale. | A live fd is not proof that the SW can answer. Keeping dead handles preserves the bad state. |
| 4 | Transport health and capability compatibility should be reported separately. | "Bridge down" and "old extension bundle" need different user actions. |
| 5 | Add heartbeat/beacon only after cheaper recovery paths are in place or proven insufficient. | Native-host heartbeat can pin MV3 activity, but it adds permanent background traffic and should be justified by evidence. |
| 6 | `BRR-T-0006` is now justified by field evidence. | Repeated real workflows produced zombie and idle-evicted bridges: native-host process/socket alive while step and ping replies timed out, and `heal` could not recreate the native-host/native-port path without manual extension reload. |

## Architecture

| Component | Responsibility | Reliability requirement |
|---|---|---|
| CLI `rzn-browser run` | Short-lived workflow producer. | Pre-step readiness can heal once; after step dispatch it must not replay side effects silently. |
| Supervisor | Durable local runtime owner and bridge state authority. | Probe active extension readiness, retire stale bridge handles, classify failures. |
| `rzn_native_host` | Chrome-launched native messaging bridge. | Stay thin; emit a bounded stdout heartbeat every 20s while the native messaging connection is alive. |
| Extension SW `background.ts` | Browser actor and native messaging port owner. | Reconnect native port on wake, answer readiness ping with build/capabilities, tolerate MV3 suspension. |

### Long-term bridge model

Chrome's native messaging contract is narrower than our current runtime shape:
Chrome launches a native host, talks to it over stdio, and keeps that host alive
only while the native messaging `Port` exists. In MV3, `connectNative()` keeps
the service worker alive while that port is open, but Chrome still expects code
to reconnect from `port.onDisconnect`. Timers and globals in the service worker
are not durable state.

Mature native-messaging extensions generally treat the browser-owned native host
as a disposable bridge, not as proof of end-to-end health. KeePassXC-Browser, for
example, starts only `keepassxc-proxy`; that proxy forwards stdio to the real
KeePassXC application over Unix sockets or named pipes. Bitwarden's browser
native-messaging code similarly has explicit `connected`/`connecting` state,
per-message ids, no-response timeouts, `onDisconnect` cleanup, and a
try/catch around `port.postMessage()` that force-disconnects when Chrome fails
to raise a disconnect event.

The target RZN model should copy those boundaries:

```text
durable owner:      rzn-browser supervisor
browser-owned shim: rzn_native_host, disposable, restartable
browser actor:      extension SW, reconnects native port on every wake/disconnect
request contract:   every bridge request has id, deadline, response, and cleanup
health contract:    connected means recent round-trip, not just fd/socket alive
```

That implies `BRR-T-0006` is not "add a heartbeat and hope." The long-term fix
is zombie detection and forced bridge recreation:

1. Classify `connected=true` plus ping/step timeout as `zombie_native_host`.
2. On zombie classification, tear down the cached supervisor bridge and the
   native-host process/port path, then wait for a fresh extension hello.
3. Add an extension-side broker watchdog so each inbound native message either
   responds or disconnects/reconnects the native port before the supervisor's
   outer timeout expires.
4. Add a health beacon with last successful ping/build, last failed ping, and
   last step timeout so status shows the transition instead of a vague reload
   prompt.
5. Add bounded heartbeat only as support for the above: low-frequency, stopped
   when native messaging is disconnected, consumed as a no-op by the extension,
   and never treated as a substitute for request/response health.

### Multi-producer architecture

Keep the multi-producer model, but put the fan-in above native messaging:

```text
CLI / MCP / app / cloud
        |
        v
supervisor ingress queue + session scheduler + event bus
        |
        v
single profile bridge lane
        |
        v
native-host port epoch -> extension SW -> tab/content/CDP
```

Multiple producers may submit work, subscribe to progress, and read cached
terminal results. They should not each own native-host connectivity or broadcast
directly to the extension. Chrome starts a native-host process for a native
messaging port; using many native hosts as the runtime bus multiplies lifecycle
states and makes zombie detection ambiguous.

The supervisor should own:

| Concern | Required behavior |
|---|---|
| Producer fan-in | Authenticated local IPC for CLI, MCP, app, and cloud. |
| Idempotency | `producer_id`, `run_id`, `command_id`, and terminal result replay before extension dispatch. |
| Scheduling | Queue by browser profile, session, tab, and side-effect class. |
| CDP/debugger access | Lease/lock per tab target; serialize attach-sensitive actions and always detach/release on timeout. |
| Event fanout | Broadcast progress/snapshots/results from supervisor to consumers, not from native host to consumers. |
| Backpressure | Bounded queue, per-producer limits, cancellation, and explicit "busy/degraded" status. |
| Bridge health | One bridge authority per profile with port/native-host epochs and recent round-trip proof. |

The extension should own browser execution only. It may multiplex internal tab
work, but it should never decide global producer fairness or replay policy.

### macOS and Windows contract

The long-term design must be platform-neutral above the install boundary:

| Layer | macOS | Windows | Shared invariant |
|---|---|---|---|
| Native messaging registration | Manifest under Chrome's `NativeMessagingHosts` location with absolute binary path. | Registry entry points Chrome to the host manifest; host stdio must run in binary mode. | Installer/doctor validates manifest, allowed extension id, binary path, executable permission, and build signature. |
| Supervisor IPC | User-scoped Unix-domain socket plus token under app base. | User-scoped named pipe or local socket equivalent plus token under app base. | Producers never trust pid files; they authenticate and handshake `rzn.local.v1`. |
| App base | `~/Library/Application Support/RZN` by default. | `%LOCALAPPDATA%/RZN` or installer-chosen per-user app data. | Native host, supervisor, CLI, app, and doctor resolve the same app base and expose it in status. |
| Process control | Terminate stale native-host PID when known; avoid overwriting live binaries directly. | Terminate stale native-host process/handle when known; respect Windows file locking. | Recovery must recreate bridge epochs without requiring extension reload in the common case. |
| Diagnostics | Chrome stderr/logging plus supervisor status. | Chrome `--enable-logging`, registry validation, supervisor status. | `rzn-browser doctor` reports platform-specific install faults separately from runtime bridge health. |

## Implementation Notes

### Why prior fixes did not hold

- `chrome.alarms` is too coarse for this alone. Current Chrome allows a 30s minimum repeating alarm, may delay alarms further, and does not provide sub-30s recovery guarantees.
- `setInterval` in the background script dies when the service worker is evicted.
- The content-script keepalive port only helps while a tab has an active injected content script holding the port open.
- `runtime.heal` has retries. The normal CLI run path now invokes it once before `browser.session_open` when the initial readiness probe finds a connected but unresponsive bridge.
- Stale bundles and sleeping service workers previously collapsed into similar user-facing reload text.
- Field evidence after `BRR-T-0001` through `BRR-T-0005` showed a sharper failure: a workflow step timed out after `browser.session_open`, then readiness reported `transport_timeout` with `connected=true`; killing the native host moved the system to `bridge_down`, and `heal` did not recover without an extension reload.
- `BRR-T-0006` now implements the first zombie-recovery slice: supervisor timeouts are classified as `zombie_native_host`, native bridge health records ping/failure/restart timestamps, supervisor sends `native_host.shutdown`, native host exits on that request, and the extension broker handler has a deadline watchdog that responds/fails before the outer supervisor timeout. Follow-up field evidence on 2026-05-15 showed MV3 service-worker eviction in 30-60s idle windows and slow page loads, so the native host now emits an unsolicited `native_host_heartbeat` stdout frame every 20s while native messaging is connected; the extension consumes this frame before broker dispatch so it wakes the SW without creating workflow responses. Extension watchdog timeouts are treated as epoch boundaries: the watchdog response wins, later stale handler responses are dropped, native-port messages/responses are fenced by port epoch, workflow-session epoch, and internal request lease id. Watchdog/disconnect cancellation now sends content-script cancellation, prevents queued stale work from starting, holds the session queue until the cancelled handler unwinds, and checks high-risk tab/CDP/content-script side effects in `execute_step`, `execute_workflow`, and auxiliary broker paths before and after async browser API calls. Native-host extension-call timeout/channel-close/stdout-write-failure paths self-retire the native-host/native-port epoch after reporting the error upstream when possible. Native messaging frame caps now follow the Chrome protocol split: host-to-Chrome writes are rejected above 1 MiB, Chrome-to-host reads use the larger protocol-side cap, and oversized extension responses to the supervisor are spilled to artifact refs instead of killing the bridge.
- Crash #5 exposed a second lie: `runtime.heal` could return green from one successful ping while the bridge epoch disappeared before the next workflow dispatch. Heal now performs a stability probe after a short settle delay and requires final supervisor status to still have a connected bridge. Browser tool dispatch also performs one dispatch-time readiness recovery/retry when the first attempt finds no bridge, so the operator gets either a real retry or the current readiness diagnostic instead of a stale no-bridge error.
- The post-fix smoke split the failure again: the immediate no-bridge desync was gone, but a `google/search` navigation still timed out mid-step and left the bridge at response-channel-closed/service-worker-unresponsive. The extension watchdog now schedules reconnect before disconnecting, gives its timeout response a 150ms native-port flush window, and the supervisor reports response-channel close during non-ping browser calls as an explicit bridge error instead of collapsing it into the generic no-bridge path. The outer supervisor client timeout now intentionally outlives the inner native-host bridge timeout, so the explicit watchdog/native-host error wins instead of a generic `Supervisor request timeout`.
- Local inspection also found an older orphaned `rzn-browser supervisor serve` process still holding the same socket path after a replacement supervisor was spawned. The supervisor now writes an app-base-scoped process lock under the run directory and refuses a second live supervisor for the same app base, while replacing stale pid locks. This prevents future split-brain runtime state where heal and dispatch can observe different supervisor processes.
- The external architecture review found one remaining correctness hole after stale responses were fenced: stale side effects from auxiliary broker paths could still continue after watchdog cancellation. Contract v6 closed that smaller but nasty gap by requiring `auxiliary_path_lease_guards`; direct DOM snapshot/observe/send-to-tab/CDP/AX-tree/content-readiness paths now resolve tabs through lease-aware helpers, guard browser mutations, attach lease metadata to content messages, and clean up late-created tabs/windows when an aborted side effect resolves after cancellation. Supervisor bridge replacement now drains all pending calls for the retired bridge with typed `NATIVE_HOST_DISCONNECTED` errors instead of leaving sibling requests to hit their own outer timeout.
- Contract v7 adds `watchdog_queue_unblock`: when the broker watchdog wins, the session queue is released immediately while the old handler unwinds detached under lease/session/native-port fences. The watchdog disconnect path now schedules reconnect after the old port is actually torn down, and native-host pending response correlation is keyed by request id before envelope validation so malformed extension responses complete with `EXTENSION_PROTOCOL_ERROR` instead of timing out.
- Contract v8 adds the visible epoch chain and callback/response fencing required by the architecture review: supervisor boot id, supervisor bridge epoch/id, native-host boot id/pid, extension worker boot id, native-port epoch, recent heartbeat age/sequence, and active request diagnostics are carried through ping/readiness health. Extension native-control callbacks are fenced by native-port epoch, supervisor response completion is fenced by owning bridge id/epoch, and native host keeps one active supervisor owner at a time.

### Ordered work

1. Auto-heal the run path before the first workflow step when readiness is false or the bridge is connected-but-unresponsive.
2. Reconnect native messaging at service-worker top level on wake, guarded by existing in-flight state.
3. Treat readiness probe timeout as a stale bridge signal and force the next attempt through reconnect.
4. Split failure diagnostics into sleeping SW, stale bundle/capability mismatch, bridge down, and transport timeout.
5. Revisit capability gating so transport can be healthy even when a specific workflow capability is missing.
6. Implement `BRR-T-0006`: zombie native-host classification, forced native-host/native-port recovery, supervisor health beacon, and bounded native-host stdout heartbeat.

## Tasks & Status

- [x] `BRR-T-0001` - Auto-heal run-path bridge readiness before aborting. Implemented in `crates/rzn_browser/src/native_runner.rs`; focused Rust coverage proves heal success, post-heal failure diagnostics, and stale-bundle no-heal behavior.
- [x] `BRR-T-0002` - Reconnect native messaging on MV3 service-worker wake. Current `extension/src/background.ts` already connects on service-worker load and keeps startup/install/alarm/content-port reconnect paths behind the shared `nativePort`/`nativeConnectInFlight` guard; extension build and independent review passed.
- [x] `BRR-T-0003` - Retire stale native-host bridge handles on probe timeout. Supervisor timeout handling clears the cached bridge, and focused regression coverage proves readiness timeout updates later status away from the stale connected handle.
- [x] `BRR-T-0004` - Classify bridge readiness failures with typed diagnostics. `runtime.ensure_ready` now reports typed causes and observed probe/build/capability facts, and the CLI run preflight renders the mapped action text.
- [x] `BRR-T-0005` - Decouple readiness capability checks from bridge transport health. Readiness now reports transport and required-capability health separately, with an explicit global keepalive gate policy and stale-bundle metadata.
- [ ] `BRR-T-0006` - Add sub-30s bridge heartbeat and health beacon. First zombie-recovery slice implemented and code gates pass; rework adds native-host stdout heartbeat for MV3 idle/slow-load eviction, preserves last failure cause/error after successful ping, fences stale responses and high-risk side effects across native-port/session/request lease epochs, guards auxiliary broker side-effect paths under bridge contract v6, releases same-session queues immediately after watchdog under bridge contract v7, carries/fences the full supervisor/native-host/extension epoch chain under bridge contract v8, propagates content-script cancellation, hard-gates stale extension bundles by bridge contract/capability map, self-retires native host on stdout pipe failure, drains retired supervisor bridge pending calls with typed errors, enforces Chrome's host-to-browser native-message size cap with artifact fallback for oversized supervisor responses, closes the heal-probe/workflow-dispatch desync with a heal stability probe plus dispatch-time readiness retry, and fixes the timeout layering race that let the CLI timeout before the bridge watchdog error surfaced.
- [ ] `BRR-T-0007` - Add supervisor queue and profile bridge scheduler. Backlog follow-up for multi-producer fairness, idempotency, backpressure, event fanout, and per-tab CDP/debugger leases.
- [ ] `BRR-T-0008` - Add macOS and Windows bridge install doctor parity. Backlog follow-up for platform registration, binary/build fingerprinting, and install diagnostics.

## What Works (Do Not Change)

- The supervisor remains the sole durable local runtime owner.
- The real Chrome extension/native-host path is the product path and the system under test.
- Chrome owns native-host launch lifetime; do not make native host a second durable runtime owner.
- Readiness and heal are allowed before a workflow step is sent. After dispatch, failures must report rather than replay write-capable steps.
- MV3 service-worker inactive state is normal. Treat it as a condition to recover from, not as a bug by itself.

## Tried & Didn't Work

- Relying on extension reloads: clears some stale-bundle or asleep-SW states, but punts the architectural problem to the user.
- Relying on `chrome.alarms` alone: the production minimum period is too slow for a sub-minute MV3 eviction race.
- Relying on a background `setInterval`: timers do not survive service-worker eviction.
- Relying on content-script keepalive: tab-scoped; no tab or no injected script means no pin.
- Treating fd-open as connected: preserves stale native-host handles when the extension can no longer answer.
