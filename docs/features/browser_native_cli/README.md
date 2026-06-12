# RZN Browser CLI Runner

## Overview
- Goal: provide a fast CLI workflow harness that talks to `rzn-browser-worker` over the current rzn-browser runtime (attach if running, else spawn) without rebuilding the desktop app, and provide a one-shot `make install` path that installs the native pieces cleanly for local distribution. The public-launch target superseding the worker-owned runtime is documented in `docs/features/local_supervisor_runtime/README.md`.
- Constraints: MV3 extension boundaries; native messaging host name must remain stable; avoid site-specific selectors; tolerate missing runtime by spawning local worker.
- Coexistence requirement: standalone CLI traffic must not steal ownership from the desktop app. Desktop broker traffic and standalone browser-run traffic must be able to share the same native host + extension connection.
- Note: CLI speaks **framed MCP JSON-RPC** to the worker control socket published in `broker_endpoint_v1.json` (`browser_worker`), not the legacy `{cmd, req_id, payload}` envelope.
- Runtime invariant: endpoint files are cache hints, not authority. A cached pid/socket must pass liveness checks before any CLI/native-host path trusts it.
- Launch target: future CLI execution should call `rzn-browser supervisor` through `rzn.local.v1`; `rzn-browser-worker` and `broker_endpoint_v1.json` should remain migration compatibility, not the primary runtime contract.

## Flow Diagrams
- End-to-end flow
```
CLI (rzn-browser run --via native)
  ↕ framed MCP JSON-RPC (tools/call)
rzn-browser-worker
  ↕ native host (com.rzn.browser.broker)
Chrome Extension (background + content script)
  ↕
Web Page
```

- Internal flow(s)
```
attach-or-spawn:
  prune stale APP_BASE/secure/broker_endpoint_v1.json sections
    ├─ attach only to live browser_worker → handshake → MCP initialize → tools/call
    └─ or spawn worker → wait for browser_worker endpoint → connect → handshake → MCP initialize
  run steps → execute_step
  optional snapshot → browser.snapshot
  session_close → detach CLI client only
  shared browser worker stays alive for later attaches/spawns
```

```
desktop + standalone coexistence:
  Chrome extension
    ↕ one native host process
  native host
    ├─ broker upstream (desktop app, rzn_debug/rzn)
    └─ browser-bridge upstream(s) (standalone CLI, rzn-browser)
  route browser.session by upstream request id
  route extension cmd envelopes to broker only
```

## Decision Record
- Prefer `run` as the user-facing execution verb. `native-run` and `desktop-run` leak plumbing; `run` tells the truth.
- Prefer attach-or-spawn so the CLI works both with a running desktop runtime and in isolation.
- Prefer the worker-owned control socket (`browser_worker`) so the CLI can attach to the **same running worker** used by the desktop app (no protocol translation layer).
- Use length-prefixed frames + token handshake for transport consistency and local auth.
- Use a dedicated standalone app base (`~/Library/Application Support/rzn-browser`) so spawned CLI workers do not overwrite desktop-owned browser bridge sockets/endpoints.
- The native host must multiplex multiple upstream sockets at once; a single-owner handoff model breaks the “desktop + CLI both usable” requirement.

## Architecture
- Modules:
  - `crates/rzn_browser/src/native_runner.rs`: endpoint resolution, attach/spawn, request/response framing, step execution.
  - `crates/rzn_browser/src/main.rs`: CLI wiring + args.
  - `crates/rzn_native_host/src/main.rs`: native-host upstream multiplexer (desktop broker + standalone browser bridge).
  - `crates/rzn_broker_endpoint/src/lib.rs`: shared endpoint read/write and stale-section pruning.
  - `start_app.sh`: mode routing plus default standalone app-base selection.
- Data contracts:
  - Handshake (frame 1):
    - `{ type: "rzn_browser_worker_handshake", v: 1, token, client: { name, pid } }`
  - MCP requests (subsequent frames):
    - `{ jsonrpc: "2.0", id, method: "tools/call", params: { name, arguments } }`
  - CLI returns the worker's `structuredContent` as the "tool payload" for downstream logging and helpers.
  - Native-host bridge routing:
    - upstream `browser.session` requests are forwarded to the extension as `{ cmd, req_id, payload }`
    - `req_id` is rewritten to a native-host-generated wire id so concurrent upstreams cannot collide
    - extension responses are mapped back to the original upstream correlation id before replying

## Implementation Notes
- Entry points:
  `rzn-browser run ...` is the preferred surface.
  `rzn-browser run ... --via native|desktop` selects the backend explicitly when needed.
  `rzn-browser run --via native ...` and `rzn-browser run --via desktop ...` remain temporary compatibility aliases.
  `rzn browser run ...` for the umbrella CLI wrapper, which should forward argv after `browser` unchanged.
- Key calls: `browser.session_open` → `browser.execute_step` (per step) → optional `browser.snapshot` → `browser.session_close`.
- Native app-base handling is now automatic for direct CLI runs. `rzn-browser run --via native` first scans common runtime locations (`RZN`, `rzn-browser`, legacy `rzn` / `rzn_debug`) for an attachable endpoint, and only falls back to the standalone `rzn-browser` namespace when it needs to spawn a worker.
- Native attach is intentionally narrow: it only accepts `browser_worker` endpoints. Desktop `broker` and worker `browser_bridge` sections are not CLI worker transports.
- Stale endpoint healing is automatic on discovery and also available explicitly through `rzn-browser heal`. Useful modes:
  - `rzn-browser heal` prunes dead broker/bridge/worker sections and restarts browser-launched native hosts discovered through live workers.
  - `rzn-browser heal --spawn-worker` also warms a worker and reports `rzn.worker.health`.
  - `rzn-browser heal --reset-worker` terminates an unresponsive cached worker, removes socket artifacts, and prunes the endpoint again.
- Installer: `make install` runs `setup.sh` in `release` mode, rebuilds `extension/dist-chrome`, installs stable copies of `rzn-browser`, `rzn-browser-worker`, and `rzn-native-host`, writes the Chrome native-host manifest, installs a stable runtime extension copy, and links PATH-facing binaries (`rzn-browser`, `rzn-browser-worker`, `rzn-native-host`) into a writable user bin directory.
- Workflow catalog: the installer copies shipped JSON workflows plus packaged examples into the runtime `workflows/builtin` catalog, keeps user imports/generated files in `workflows/user`, supports `rzn-browser workflow pull` for refreshes, and `rzn-browser run|native-run|desktop-run|run-workflow` resolve canonical workflow ids like `google/search` while also accepting the preferred CLI form `google search`.
- Error handling: stop on first failed step; optionally snapshot on error; keep the shared worker alive on exit by default so later CLI runs and parallel sessions can reuse the same browser-side attachment. Set `RZN_KILL_BROWSER_WORKER_ON_EXIT=1` only when you explicitly want the old ephemeral-worker behavior.
- Worker lifetime: the spawned `rzn-browser-worker` no longer exits just because the spawning CLI process closes stdin. Socket-mode workers now stay alive until explicit shutdown, which makes the "shared worker stays alive on exit" policy real instead of aspirational.
- Native-host reconnect model: the browser-launched native host keeps its broker connection and separately discovers/attaches browser-bridge endpoints as standalone workers appear.
- Session isolation: spawned workflow sessions are expected to own dedicated tabs. Background helpers must fail closed when a non-default session has no workflow tab instead of silently stealing the user's active tab. Built-in catalog workflows should avoid active-tab legacy fields; active-tab access is only a low-level manual debugging escape hatch.

## Tasks & Status
- [x] Add attach-or-spawn CLI runner for workflows
- [x] Add preferred `run` verb with backend selection via `--via`
- [x] Log per-step status + key results
- [x] Add dev loop documentation
- [x] Add `make install` release install flow with PATH-facing CLI/runtime binaries
- [x] Allow desktop broker traffic and standalone browser-run traffic to coexist without native-host ownership handoff
- [x] Reuse a live browser worker for parallel `run --via native --mode spawn` sessions under one `APP_BASE`
- [x] Keep spawned workflow sessions isolated to dedicated tabs unless an existing-session workflow explicitly requires otherwise
- [x] Prune stale broker/bridge/worker endpoint sections before attach/discovery
- [x] Add `rzn-browser heal` for explicit endpoint/native-host/worker recovery
- [ ] Add integration test covering attach + spawn modes

## What Works (Do Not Change)
- Native host name `com.rzn.browser.broker` and extension-side behavior (MV3 constraints)
- Generic targeting and workflow parameter substitution (`{param}`) without site-specific selectors
- Desktop broker mode remains the default under `RZN_RUN_MODE=auto`
- Standalone native backend uses its own app-base namespace by default; do not collapse it back onto `rzn_debug`
- Native CLI runs do not require the Reason/RZN desktop app to be open. They require Chrome/Edge with the extension enabled so the browser-launched native host can connect to the worker-owned browser bridge.

## Tried & Didn’t Work
- Relying on `run-workflow` for the fast loop: it pulls in LLM/autonomy and does not target the rzn-browser runtime.
- Reusing the desktop app base for spawned standalone workers caused endpoint/socket collisions and forced native-host restarts.
- A native host that selected only one upstream at startup was not sufficient for desktop + standalone coexistence.
- Treating `broker_endpoint_v1.json` as proof of liveness was brittle; dead pids and orphan socket files must be pruned before attach or discovery.
