---
schema: "tusker.knowledge/v6"
node: "runtime/canon"
title: "Runtime canon"
domain: "runtime"
kind: "canon"
audience: "developer"
agent_layer: "capsule"
canonical_status: "draft"
summary: "Current local browser runtime model: supervisor, native host, extension, sessions, heal, and logs."
aliases:
  - "runtime canon"
  - "runtime"
  - "local browser runtime"
source_of_truth:
  - "crates/rzn_browser/src/main.rs"
  - "crates/rzn_browser/src/supervisor.rs"
  - "crates/rzn_browser/src/native_runner.rs"
  - "crates/rzn_native_host/src/main.rs"
  - "extension/src/background.ts"
  - "extension/src/contentScript.ts"
  - "docs/features/local_supervisor_runtime/README.md"
  - "docs/features/connection_reliability/README.md"
  - "docs/features/bridge_reliability_hardening/README.md"
  - "AGENTS.md"
stale_when:
  paths:
    - "crates/rzn_browser/src/main.rs"
    - "crates/rzn_browser/src/supervisor.rs"
    - "crates/rzn_browser/src/native_runner.rs"
    - "crates/rzn_native_host/src/**"
    - "extension/src/background.ts"
    - "extension/src/contentScript.ts"
    - "extension/src/runtime/**"
    - "docs/features/local_supervisor_runtime/**"
    - "docs/features/connection_reliability/**"
    - "docs/features/bridge_reliability_hardening/**"
publish:
  include_in_llms: true
  lane: "internal"
  path: "runtime/canon"
created_at: "2026-05-12"
updated_at: "2026-05-14"
tags:
  - "runtime"
---

# Runtime canon

## Read this when

Read this before changing browser execution, supervisor behavior, native messaging, session routing, heal/status commands, extension bridge timeouts, CDP leases, or local runtime install paths.

## Do not read this when

Do not use this as a workflow authoring guide; read [[workflow/CANON]] and `docs/workflows/AGENT_PLAYBOOK.md` for workflow JSON. Do not use task files as canon unless this page sends you to a task for proof.

## Current model

`rzn-browser` has one durable local runtime owner: `rzn-browser supervisor`. The Chrome extension remains mandatory because it executes inside the user's real Chrome profile. The Chrome native host remains mandatory because it is Chrome's local bridge to native code. The supervisor is the state authority for CLI, MCP, app, and cloud producers.

| Component | Current role | Target boundary |
|---|---|---|
| `rzn-browser run` | CLI entrypoint; submits workflows through the supervisor and formats output. | Short-lived producer. |
| `rzn-browser supervisor` | Local socket server using `rzn.local.v1`; owns status, `ensure_ready`, heal, sessions, native-host bridge registration, and cloud actor lifecycle. | Durable local runtime authority. |
| `rzn-native-host` | Chrome-owned process that forwards extension frames upstream to the supervisor. | Thin extension-to-supervisor bridge only. |
| Chrome extension | MV3 browser actor. Background script talks to native host, routes tabs, manages CDP leases, and dispatches content/page actions. | Browser execution and observation only. |

The default runtime path should be:

```mermaid
flowchart LR
  CLI["CLI / MCP / app / cloud producer"] --> Sup["rzn-browser supervisor\nrzn.local.v1"]
  Sup --> Host["Chrome native host\nnative_host.extension_call"]
  Host --> Ext["Chrome extension"]
  Ext --> Page["Real Chrome tab\ncontent script / CDP"]
```

## Runtime protocol and paths

- Local supervisor protocol is `rzn.local.v1`.
- Default supervisor files are under the app base:
  - `run/rzn-supervisor.sock`
  - `secure/rzn-supervisor-token-v1`
- `SupervisorConfig::app_base_dir()` resolves from `RZN_SUPERVISOR_APP_BASE`, `RZN_NATIVE_APP_BASE`, `RZN_APP_BASE`, `APP_BASE`, then platform default app data.
- Native-host supervisor overrides are `RZN_LOCAL_RUNTIME_SOCKET_PATH`, `RZN_LOCAL_RUNTIME_TOKEN_PATH`, `RZN_SUPERVISOR_SOCKET_PATH`, and `RZN_SUPERVISOR_TOKEN_PATH`.
- `broker_endpoint_v1.json` and browser-worker socket files are ignored migration debris. Runtime discovery is supervisor socket/token only.

## Supervisor methods

The implemented supervisor dispatch includes:

| Method/tool | Responsibility |
|---|---|
| `runtime.hello` / `runtime.status` | Return protocol, pid, app base, socket/token paths, proxy mode, and bridge status. |
| `runtime.ensure_ready` | Wait for native-host bridge, optionally probe extension readiness. |
| `runtime.heal` | Run explicit repair/readiness checks with longer bridge waits and structured diagnostics. |
| `runtime.shutdown` | Stop the supervisor loop. |
| `rzn.supervisor.health` | Tool-compatible health view. |
| `browser.session_open`, `browser.execute_step`, `browser.snapshot`, `browser.poll_events`, `browser.session_close` | Browser tool calls routed through the native-host bridge. |

Browser tools fail clearly if the native-host bridge is unavailable.

## Extension execution model

The extension has two major lanes:

| Lane | Source anchors | Notes |
|---|---|---|
| Background/service worker | `extension/src/background.ts` | Native messaging, tab routing, CDP session leases, debugger attach/detach, circuit-breaker flags, action normalization. |
| Content script/page bridge | `extension/src/contentScript.ts`, `extension/src/pageBridge.ts` | DOM snapshots, deep/shadow DOM querying, enhanced actions, input synthesis, result normalization. |

CDP is an escalation path, not the normal path. The background script tracks per-tab CDP lease expiry, avoids repeated attach/detach churn, and can disable CDP or batching for poor-performing hosts through circuit-breaker flags.

## Invariants

- Use the user's existing Chrome session for product-path browser automation. Do not substitute Playwright-managed Chrome, temporary profiles, or WebDriver unless the task is explicitly a repo-owned Playwright test or the human asks for it.
- The extension/native-host path is the system under test for runtime work.
- Chrome owns native-host lifetime; native host must not become the durable runtime owner.
- MV3 service workers can suspend; bridge code must tolerate reconnects and explicit readiness checks.
- Supervisor status should distinguish "supervisor alive, extension unavailable" from process failure.
- Cloud/browser commands must dedupe before side-effectful extension dispatch. The extension should not see duplicate live commands for one cloud `command_id`.
- Heals verify IPC, start/check supervisor, verify native-host bridge readiness, and must not replay side effects silently.

## Current defaults

- `rzn-browser run` uses the supervisor; native/desktop backend flags are removed.
- Snapshot mode defaults to `on-error`.
- Native-host bridge request timeout defaults are short enough to fail visibly.
- Supervisor bridge probe defaults are intentionally small for normal readiness and longer for heal.

## Deprecated behavior

- `rzn-browser-worker` and `rzn_broker_endpoint` are deleted runtime surfaces.
- `broker_endpoint_v1.json` is historical debris and must not be treated as source authority.
- Native-host-owned cloud or app-owned browser worker discovery are migration paths, not the final product architecture.
- Do not add new runtime ownership to Reason app, native host, or workflow JSON.

## Source of truth

- CLI and command defaults: `crates/rzn_browser/src/main.rs`
- Supervisor protocol/state: `crates/rzn_browser/src/supervisor.rs`
- Supervisor workflow execution and heal: `crates/rzn_browser/src/native_runner.rs`
- Native messaging bridge: `crates/rzn_native_host/src/main.rs`
- Browser execution: `extension/src/background.ts`, `extension/src/contentScript.ts`
- Target topology: `docs/features/local_supervisor_runtime/README.md`

## Open questions

- Final macOS startup policy for the supervisor: lazy auto-start, LaunchAgent, or both.
- Whether cloud execution is launch-scope or remains behind internal flags until result replay/dedupe is fully proved.

## Related

- [[runtime/INDEX]]
- [[workflow/CANON]]
- [[codebase/CANON]]

## Recent changes

<!-- tusker:backrefs:begin -->
- [[OPS-T-0002]] touched this knowledge node.
<!-- tusker:backrefs:end -->
