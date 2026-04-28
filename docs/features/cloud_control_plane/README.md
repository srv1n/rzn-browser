# Cloud Control Plane

## Overview
- Goal: move orchestration, scheduling, and LLM planning to a hosted control plane, while a **local supervisor daemon** remains the durable device-side actor and the extension remains the typed browser executor inside the user's real Chrome session.
- Product outcome: a cloud service can dispatch workflows to a paired browser device, receive snapshots/results, and run multi-step autonomous loops without the CLI being the primary runtime entrypoint.
- Technical posture: preserve the existing execution contract surface (`StepKind`, extraction plans, snapshots, CDP escalation), avoid arbitrary remote code execution, and keep browser interaction auditable.
- Constraints:
  - Chrome native messaging is local-only. The cloud cannot literally become the native host.
  - MV3 service workers are suspendable. They are not strong enough to be the canonical long-lived device actor.
  - Browser execution must remain typed and policy-gated.
  - Existing local workflows and broker-driven operation must keep working during migration.

- Non-goals:
  - No generic remote shell on the machine.
  - No arbitrary JavaScript as the primary remote execution mechanism.
  - No second browser automation stack for the main path. Playwright/WebDriver remain test-only or explicit break-glass tools.
  - No site-specific targeting logic.

## Flow Diagrams
- Current local-first flow
```mermaid
flowchart LR
  A["CLI / Desktop caller
  rzn-browser / rzn_plan"] -->|"length-prefixed JSON"| B["rzn_broker
  local broker/native host"]
  B -->|"Chrome native messaging"| C["Extension service worker
  background.ts"]
  C --> D["Content script
  contentScript.ts"]
  C --> E["CDP / frameRouter"]
  D --> F["Web page DOM"]
  E --> F
  F --> D --> C --> B --> A
```

- Recommended long-term flow
```mermaid
flowchart LR
  A["Cloud control plane
  API + scheduler + planner"] <-->|"WebSocket + command leases"| B["Local supervisor daemon
  canonical device actor"]
  B -->|"native messaging / local IPC"| C["Extension bridge
  background.ts"]
  C --> D["Content script executor"]
  C --> E["CDP / frameRouter"]
  D --> F["Real Chrome page"]
  E --> F
```

- Optional lite mode, not canonical
```mermaid
flowchart LR
  A["Cloud control plane"] <-->|"WebSocket"| B["Extension actor"]
  B --> C["Content script / CDP"]
  C --> D["Web page"]
```

- Planner loop in the target architecture
```mermaid
sequenceDiagram
  participant C as Cloud Planner
  participant S as Local Supervisor
  participant B as Extension Bridge
  participant P as Page

  C->>S: command.execute(get_dom_snapshot)
  S->>B: broker-style command
  B->>P: capture snapshot
  P-->>B: dom_snapshot + dom_hash
  B-->>S: command.result
  S-->>C: command.result
  C->>C: choose next typed action
  C->>S: command.execute(execute_step)
  S->>B: broker-style command
  B->>P: DOM -> synthetic -> CDP
  P-->>B: step result
  B-->>S: command.result
  S-->>C: command.result
```

- Control split
```text
Cloud owns:
  - account/workspace auth
  - run scheduling and queueing
  - LLM planning and policy defaults
  - lease/retry decisions at the run level
  - durable run state and telemetry aggregation

Local supervisor owns:
  - durable cloud connectivity
  - local credential storage
  - command spooling and redelivery coordination
  - extension reachability and recovery
  - native / OS capabilities

Extension owns:
  - browser session and tab affinity
  - DOM snapshots, observe, extraction plans
  - deterministic typed step execution
  - DOM -> synthetic -> CDP escalation
  - browser-side policy enforcement
```

## Decision Record
- The cloud is the control plane, not the native host. Native messaging is a browser-to-local-process channel, so the correct long-term topology is `cloud -> local supervisor -> extension -> page`.
- The local supervisor daemon is the canonical device actor. It is a better fit than an MV3 service worker for long-lived connectivity, buffering, and secret storage.
- The extension remains the browser bridge and executor. `background.ts` already owns tab/session/CDP routing, and `contentScript.ts` already owns the execution surface.
- The cloud reuses the existing typed action model rather than inventing a new browser DSL. We should anchor on `rzn_core::StepKind` and `rzn_contracts::v1`.
- `rzn_broker` should evolve, not be replaced. Its core job changes from "CLI transport endpoint" to "durable local supervisor and bridge adapter."
- Extension-direct mode is useful for low-friction onboarding and browser-only scenarios, but it should be treated as a lite mode with reduced guarantees.

## Architecture
- Canonical component map

| Layer | Responsibility | Current anchor | Target state |
| --- | --- | --- | --- |
| Cloud API | actor pairing, workspace auth, run CRUD | none | new service |
| Cloud dispatch | queueing, leases, run assignment | none | new service |
| Cloud planner | LLM loops, retries, policy defaults | `crates/rzn_plan` | rehosted service |
| Local supervisor | durable device actor, cloud WS, spool, native capabilities | `rzn_broker` | evolved daemon |
| Extension bridge | browser transport, workflow tab/session routing, CDP leases | `extension/src/background.ts` | preserved and refactored |
| Content executor | snapshots, observe, extraction, action ladder | `extension/src/contentScript.ts` | preserved |
| Typed contracts | action/snapshot/result model | `crates/rzn_contracts` | preserved and extended |
| SDK/session model | host-facing deterministic actor surface | `crates/rzn_sdk` | adapted for cloud client/server usage |

- Canonical deployment topology

| Deployment unit | Runs where | Must persist | Notes |
| --- | --- | --- | --- |
| Cloud control plane | hosted infra | accounts, actors, runs, command leases, telemetry | multi-tenant |
| Local supervisor daemon | user machine | device credentials, command spool, last-known extension state | installed binary/service |
| Extension | user Chrome profile | tab/session affinity hints, pairing metadata, feature flags | MV3 |
| Content scripts | target pages | ephemeral page-local state only | reinjected as needed |

- Repo/module mapping

| Repo path | Current purpose | Spec direction |
| --- | --- | --- |
| `rzn_broker/src/main.rs` | broker/native host | split into `device actor`, `cloud transport`, `extension bridge adapter`, `spool/persistence` |
| `rzn_broker/src/protocol.rs` | broker/extension messages | preserve as local bridge protocol; do not force cloud to speak a different browser DSL |
| `extension/src/background.ts` | native host connection + routing | refactor into `transport adapters` + `command executor` + `session store` |
| `extension/src/contentScript.ts` | execution surface | preserve as-is except for command/result metadata improvements |
| `crates/rzn_plan/*` | local orchestration | extract/rehost planning logic into cloud planner services |
| `crates/rzn_contracts/src/v1.rs` | typed action/snapshot/result schema | become the stable cloud-facing action contract baseline |
| `crates/rzn_sdk/*` | host-side deterministic SDK | split into local SDK and cloud client SDK later |

- Capability model

| Capability | Source | Used for |
| --- | --- | --- |
| `extension_actor` | extension | deterministic browser steps |
| `cdp_available` / `cdp_enabled` / `cdp_attached` | extension | trusted input, cross-frame rescue |
| `native_input` | supervisor | OS-level input when browser ladder is insufficient |
| `local_files` | supervisor | file resolution/uploads/download handling |
| `desktop_prompts` | supervisor | human confirmations, notifications |
| `local_llm` | supervisor optional | future fallback/planner rescue mode |

- Runtime identities

| Entity | Cardinality | Purpose |
| --- | --- | --- |
| Workspace | many actors | tenant boundary |
| Actor | one installed device supervisor | cloud-addressable device identity |
| Browser session | one Chrome profile/extension binding | device-local browser availability |
| Run | one workflow execution | server-side orchestration record |
| Session | logical browser automation context within a run | tab affinity + dom hash continuity |
| Command | one executable typed unit | leased and idempotent |

- Data model
```text
Actor
  actor_id
  workspace_id
  display_name
  status { online, offline, degraded, blocked }
  capabilities
  version
  paired_at
  last_seen_at
  heartbeat_expires_at

Run
  run_id
  workspace_id
  actor_id
  planner_mode { deterministic, llm_server, llm_hybrid }
  requested_by
  status { queued, assigned, running, blocked, completed, failed, canceled }
  input
  created_at
  updated_at

Session
  session_id
  run_id
  actor_id
  current_tab_id
  current_url
  dom_hash
  extension_session_version
  state { pending, active, recovering, closed }

Command
  command_id
  run_id
  session_id
  lease_owner_actor_id
  lease_expires_at
  attempt
  dedupe_key
  payload
  status { queued, leased, acked, completed, failed, expired }
```

## Data Contracts
- Contract baseline
  - Reuse `rzn_contracts::v1` for typed browser actions and results.
  - Keep broker/local extension messages compatible with the current `cmd`/`req_id`/`payload` shape.
  - Introduce a cloud envelope around the existing local action payloads rather than replacing them.

- Cloud envelope
```json
{
  "version": "rzn.cloud.v1",
  "type": "command.execute",
  "actor_id": "act_123",
  "run_id": "run_456",
  "session_id": "sess_789",
  "command_id": "cmd_abc",
  "lease_id": "lease_xyz",
  "deadline_ms": 1710000000000,
  "payload": {
    "kind": "browser_command",
    "command": {
      "cmd": "execute_step",
      "payload": {
        "step": {
          "id": "step-1",
          "name": "Click Sign In",
          "type": "click_element",
          "selector": "@e12"
        },
        "use_current_tab": true
      }
    }
  }
}
```

- Local bridge envelope
```json
{
  "cmd": "execute_step",
  "req_id": "cmd_abc",
  "payload": {
    "session_id": "sess_789",
    "step": {
      "id": "step-1",
      "name": "Click Sign In",
      "type": "click_element",
      "selector": "@e12"
    },
    "use_current_tab": true
  }
}
```

- Command kinds

| Cloud `payload.kind` | Purpose | Maps to current surface |
| --- | --- | --- |
| `browser_command` | typed action or snapshot/extraction request | `cmd`/`payload` broker-style message |
| `run_control` | cancel, pause, resume, cleanup | new supervisor-only behavior |
| `policy_resolution` | continue after confirmation or deny | new supervisor + extension coordination |
| `health_probe` | diagnostics, capabilities refresh | mix of supervisor and extension info |

- Browser commands that must be supported first

| Command | Current support | Notes |
| --- | --- | --- |
| `get_dom_snapshot` | yes | compact snapshot + dom hash |
| `observe` | yes | selector discovery / candidate summary |
| `execute_extraction_plan` | yes | deterministic structured extraction |
| `execute_step` | yes | typed action execution |
| `enable_debug` / `disable_debug` | yes | break-glass only |
| `process_dom` / `detect_auto_list` | yes | optional for planner enrichment |

- Result envelope
```json
{
  "version": "rzn.cloud.v1",
  "type": "command.result",
  "actor_id": "act_123",
  "run_id": "run_456",
  "session_id": "sess_789",
  "command_id": "cmd_abc",
  "lease_id": "lease_xyz",
  "success": true,
  "finished_at_ms": 1710000001234,
  "result": {
    "success": true,
    "current_url": "https://example.com/home",
    "current_tab_id": 321,
    "dom_hash": "a1b2c3",
    "raw": {
      "success": true
    }
  }
}
```

- Event envelope

| Event | Emitter | Purpose |
| --- | --- | --- |
| `actor.hello` | supervisor | announce actor identity/capabilities |
| `actor.ready` | cloud | accepted config/heartbeat intervals |
| `actor.state` | supervisor | online/degraded/blocking state |
| `run.assigned` | cloud | actor receives run |
| `command.ack` | supervisor | command accepted locally |
| `command.progress` | supervisor | optional long-running progress |
| `command.result` | supervisor | final result |
| `policy.prompt` | supervisor or extension | high-risk action requires confirmation |
| `policy.resolved` | cloud | approved/denied continuation |

## Protocol Specification
- Transport
  - Cloud <-> supervisor: WebSocket with JSON messages, heartbeat, and resumable actor session.
  - Supervisor <-> extension: current native messaging / local IPC path with existing JSON framing semantics.
  - Extension <-> content script: existing `chrome.tabs.sendMessage`, `chrome.scripting`, and CDP APIs.

- Supervisor WebSocket session

| Step | Description |
| --- | --- |
| connect | supervisor opens WebSocket to cloud |
| hello | supervisor sends `actor.hello` with actor_id, capabilities, versions |
| ready | cloud responds with accepted actor session, heartbeat interval, feature gates |
| resume | supervisor may send unresolved lease ids and spool cursor |
| stream | cloud dispatches `command.execute` / `run.control` |
| heartbeat | both sides exchange ping/pong or heartbeat messages |
| reconnect | supervisor reconnects with `resume_token` after disconnect |

- Pairing flow
```mermaid
sequenceDiagram
  participant U as User
  participant D as Local Supervisor
  participant C as Cloud API
  participant E as Extension

  U->>C: request pairing
  C-->>U: short-lived pairing code
  U->>D: enter pairing code
  D->>C: redeem pairing code
  C-->>D: actor credential + actor_id
  D->>E: register local actor metadata
  E-->>D: extension_id + browser capabilities
  D->>C: actor.hello
  C-->>D: actor.ready
```

- Pairing requirements
  - Pairing codes are short-lived and one-time use.
  - Durable credential is stored by the supervisor, not only in extension storage.
  - Extension stores only the minimum metadata needed to trust local supervisor commands and expose actor status.

- Lease semantics

| Field | Rule |
| --- | --- |
| `command_id` | globally unique per run |
| `lease_id` | unique per dispatch attempt |
| `deadline_ms` | hard timeout for command completion |
| `dedupe_key` | stable across retries to prevent duplicate side effects |
| `attempt` | incremented every redelivery |

- Lease rules
  - Cloud marks a command `leased` when sent to one actor.
  - Supervisor must `ack` within a short window or the lease may be reassigned/retried.
  - Once acked, only the same actor may complete that lease.
  - If supervisor crashes after ack but before result, reconnect must include unresolved lease ids.
  - The extension should never see duplicate live commands for the same `command_id`; dedupe happens before the bridge.

- Idempotency rules
  - `get_dom_snapshot`, `observe`, `process_dom` are naturally idempotent.
  - `execute_step` is not always idempotent. The supervisor must avoid reissuing an already completed side-effecting step unless the cloud explicitly redrives it.
  - Commands may include `side_effecting: true/false` and `idempotency_policy`.

- Command metadata

| Field | Required | Purpose |
| --- | --- | --- |
| `run_id` | yes | ties command to run record |
| `session_id` | yes | preserves tab/session affinity |
| `command_id` | yes | dedupe and tracing |
| `parent_command_id` | optional | chain correlation |
| `trace_id` | yes | end-to-end telemetry |
| `planner_step_index` | optional | planner debugging |

## Session and Tab Model
- Session semantics
  - A `session_id` is the stable browser context key for a run.
  - It determines workflow tab affinity, current URL continuity, and DOM hash continuity.
  - The extension bridge remains the source of truth for `current_tab_id`; the supervisor caches and mirrors it.

- Tab affinity rules
  - Default sessions may bind to the current active tab when `use_current_tab=true`.
  - Dedicated sessions may create and own a workflow tab.
  - The extension must persist enough session metadata to recover after service-worker restart.
  - The supervisor must tolerate stale tab ids and request resynchronization.

- Recovery cases

| Failure | Recovery |
| --- | --- |
| service worker restart | supervisor reconnects to extension, extension reloads session metadata from storage |
| workflow tab closed | extension returns explicit `NO_WORKFLOW_TAB`-style error; supervisor reports failure or asks planner to recover |
| browser closed | supervisor goes degraded/offline and keeps cloud state |
| command in flight during disconnect | supervisor resumes unresolved lease and either completes or fails explicitly |

## Local Supervisor Specification
- Responsibilities
  - Maintain the authenticated cloud session.
  - Expose actor health and capabilities.
  - Keep a local spool of unresolved commands/events.
  - Translate cloud commands into current broker/local extension messages.
  - Aggregate extension results and send structured `command.result`.
  - Own native capabilities (`native_input`, file system, desktop prompts).
  - Coordinate policy prompts and user confirmations.

- Internal modules

| Module | Responsibility |
| --- | --- |
| `cloud_client` | WebSocket connect/auth/heartbeat/resume |
| `command_dispatcher` | lease handling, dedupe, in-flight command state |
| `extension_bridge` | current broker-to-extension transport adapter |
| `spool_store` | durable queue/event store |
| `capability_registry` | actor capability snapshot |
| `policy_manager` | prompt/approval resolution |
| `native_services` | native input, file access, notifications |
| `telemetry_sink` | local logs + uplinked telemetry |

- Persistence requirements

| Store | Durability | Contents |
| --- | --- | --- |
| actor credential store | durable | actor token, actor_id, pairing metadata |
| in-flight spool | durable | unresolved commands/events |
| capability cache | soft durable | last extension/browser capabilities |
| session mirror | soft durable | last known session_id -> tab/url/dom hash |

- Process model
  - The supervisor should run as a long-lived background process.
  - The extension should be able to connect to it on demand via native messaging.
  - The supervisor should tolerate extension absence and advertise degraded state rather than crashing.

## Extension Bridge Specification
- Responsibilities
  - Continue to own browser-specific routing and execution.
  - Expose a transport-agnostic command execution entrypoint so commands can come from broker/supervisor now and other adapters later.
  - Persist session/tab affinity metadata beyond in-memory `workflowSessions`.
  - Preserve current DOM -> synthetic -> CDP ladder and `frameRouter` behavior.

- Required refactor in `background.ts`

| Current concern | Target split |
| --- | --- |
| `connectToNative()` | transport adapter only |
| `handleBrokerMessage(...)` | transport-agnostic `executeCommand(...)` |
| `workflowSessions` in memory | `sessionStore` with durable backing |
| direct response posting | response adapter per transport |

- Durable session store
  - Persist `session_id`, `workflow_tab_id`, `current_url`, `updated_at`, and optional `lease_id`.
  - Use `chrome.storage.local` for MV3-compatible persistence.
  - Reload on startup before first command handling.

- Result requirements
  - Every command result should return `success`, `current_url`, `current_tab_id` when known, and `dom_hash` when relevant.
  - Prefer explicit error codes over plain text.
  - Preserve raw result payloads for forward compatibility.

## Cloud Control Plane Specification
- Services

| Service | Responsibility |
| --- | --- |
| Auth service | workspace/users, pairing codes, actor credentials |
| Actor registry | online actors, last_seen, capabilities |
| Run scheduler | queued/running run assignment |
| Command lease manager | per-command lifecycle and retries |
| Planner service | deterministic and LLM-driven loops |
| Event/telemetry ingestion | progress, logs, metrics |
| Policy service | optional hosted approval UI and rules |

- Planner modes

| Mode | Description |
| --- | --- |
| `deterministic` | execute predefined workflow JSON only |
| `llm_server` | server owns planning loop, actor only executes commands |
| `llm_hybrid` | future mode; server plans, supervisor may rescue locally |

- Planner guidance
  - Prefer `rzn_contracts::v1::ActionV1` semantics for the cloud planner surface.
  - For advanced browser operations not yet expressed in `ActionV1`, wrap the current broker-style command payloads during migration.
  - Expand `rzn_contracts` over time rather than baking broker-local shapes into the cloud API forever.

- Multi-tenant boundaries
  - An actor belongs to one workspace at a time.
  - Runs and commands are always scoped to one workspace and one actor assignment.
  - Cloud must never dispatch a workspace A command to workspace B actor, even transiently.

## Security Specification
- Trust boundaries
```text
Cloud tenant boundary
  -> authenticated actor session
    -> local supervisor trust boundary
      -> extension trust boundary
        -> untrusted web page
```

- Core rules
  - Cloud never sends executable arbitrary JS as the primary path.
  - The supervisor stores durable credentials; extension stores only narrow local metadata.
  - High-risk actions must be policy-gated locally.
  - `enable_debug` and any `eval_*` or `eval_with_cdp` command are break-glass, explicitly logged, and feature-gated.
  - All commands and results carry trace ids and actor ids for auditability.

- Policy categories

| Category | Default |
| --- | --- |
| navigation/click/fill/extract | allow |
| file upload/download | confirm or capability-gated |
| auth prompt / MFA | confirm |
| checkout/payment/delete | confirm |
| arbitrary eval | deny by default, allow only break-glass |

- Secrets
  - Actor token: supervisor-only durable secret.
  - Pairing code: short-lived one-time secret.
  - Extension local metadata: non-secret actor reference and transport state only.

## Reliability and Recovery
- Failure handling matrix

| Failure | Expected behavior |
| --- | --- |
| cloud disconnect | supervisor reconnects with resume token and unresolved leases |
| supervisor crash | cloud lease expires; actor returns offline until reconnect |
| extension unavailable | supervisor marks actor degraded, retries bridge attach, surfaces health |
| service worker restart | extension reloads durable session store and resumes bridge handling |
| command timeout | supervisor sends explicit timeout result or lease expiry signal |
| planner crash | command lease manager preserves run state; planner can resume from transcript |

- Timeouts

| Timeout | Owner | Purpose |
| --- | --- | --- |
| actor heartbeat interval | cloud + supervisor | liveness |
| command ack timeout | cloud | dispatch responsiveness |
| command deadline | cloud | hard execution cap |
| local step timeout | extension | browser action execution |
| policy prompt timeout | supervisor/cloud | avoid hanging runs forever |

- Recovery transcript
  - The supervisor should keep a short local command/result transcript for recovery and debugging.
  - The cloud remains source of truth for durable run transcript.

## Observability
- Required correlation identifiers
  - `trace_id`
  - `run_id`
  - `session_id`
  - `command_id`
  - `lease_id`
  - `actor_id`

- Logging

| Layer | Required fields |
| --- | --- |
| Cloud | actor_id, run_id, command_id, planner_step_index |
| Supervisor | actor_id, lease_id, local bridge status, reconnect state |
| Extension | session_id, tab_id, step type, CDP state, error_code |

- Metrics
  - actor online/offline count
  - command latency by type
  - command retry count
  - extension bridge attach failure count
  - CDP rescue rate
  - policy prompt rate and approval rate
  - run completion/failure rate by planner mode

## Implementation Notes
- Phase 0: architecture-hardening in the existing codebase
  - Extract `background.ts` command execution into a transport-agnostic function.
  - Introduce a durable `sessionStore`.
  - Improve result normalization and error codes.

- Phase 1: supervisor MVP
  - Evolve `rzn_broker` into a daemon with:
    - cloud WebSocket client
    - durable actor credentials
    - command spool
    - current broker/extension bridge adapter
  - Keep local CLI compatibility during this phase.

- Phase 2: cloud control plane MVP
  - Build actor registry, pairing, command lease manager, and deterministic run execution.
  - Support dispatching:
    - `get_dom_snapshot`
    - `observe`
    - `execute_extraction_plan`
    - `execute_step`
  - Persist paired actors and run transcripts to `~/Library/Application Support/rzn/cloud_control_plane_state_v1.json` for local dev restarts.
  - Replay unresolved commands after actor reconnect and suppress duplicate browser execution by caching terminal command results in the local actor.

- Phase 3: server planner migration
  - Rehost the `rzn_plan` orchestration logic server-side.
  - Start with deterministic planning and typed step loops.
  - Move autonomous LLM planning after command leasing is proven.

- Phase 4: policy + privileged local capabilities
  - Add native input and desktop prompts through the supervisor.
  - Add cloud-hosted approval UIs with local enforcement.

- Phase 5: optional extension-direct lite mode
  - Add extension-only actor path for browser-only deployments.
  - Treat it as reduced-capability mode without the same durability guarantees.

- Migration strategy

| Phase | Compatibility expectation |
| --- | --- |
| 0 | existing CLI/broker/extension flows unchanged |
| 1 | broker supports both local CLI and cloud supervisor modes |
| 2 | cloud can drive deterministic runs without removing CLI |
| 3 | planner shifts to cloud for hosted runs |
| 4+ | direct extension mode optional, not required |

## Testing Plan
- Unit tests
  - cloud envelope encode/decode
  - lease/dedupe logic
  - session store persistence and reload
  - capability negotiation
  - policy gate transitions

- Integration tests
  - supervisor <-> extension bridge command round-trip
  - reconnect with unresolved leases
  - browser closed / reopened recovery
  - duplicate command delivery suppression
  - direct `exec-command` run persistence and session-id normalization

- E2E tests
  - cloud assigns deterministic run to one actor
  - snapshot -> plan -> execute -> result loop
  - high-risk action blocks for confirmation
  - extension restart during run recovers
  - supervisor crash and reconnect resumes correctly
  - remote operator issues a single command and gets the typed result back without wrapping a workflow

- Test harness notes
  - Reuse current extension e2e fixtures where possible.
  - Add a fake cloud dispatcher for local integration tests.
  - Add transcript assertions for command ids and lease ids.
  - Keep a first-class operator smoke path:
    - `rzn-browser cloud list-actors`
    - `rzn-browser cloud exec-command`
    - `rzn-browser cloud exec-step`

## Tasks & Status
- [ ] Extract a fully transport-agnostic command executor from `extension/src/background.ts`
- [x] Add durable `sessionStore` in the extension
- [ ] Normalize broker/extension responses to always carry structured result metadata
- [x] Define `rzn.cloud.v1` cloud envelope and message schemas
- [x] Add cloud-connected local actor support in the active native-host boundary (`rzn-native-host`) and legacy broker fallback (`rzn_broker`)
- [x] Add actor pairing flow and durable credential storage
- [x] Implement cloud actor registry and WebSocket session handling
- [x] Implement command leasing, ack, dedupe, and reconnect resume
  Status: cloud tracks pending commands, records `command.ack`, replays unresolved commands after reconnect, and local actors return cached terminal results for duplicate `command_id`s.
- [x] Rehost deterministic workflow dispatch in the cloud
- [x] Add operator-facing remote command testing surface
  Status: `rzn-browser cloud list-actors`, `rzn-browser cloud exec-command`, and `rzn-browser cloud exec-step` are live, and direct browser commands persist a single-step `cloud.exec_command` run record that can be fetched via `cloud get-run`.
- [x] Add extension popup for local cloud configuration and remote smoke probes
  Status: the unpacked Chrome extension now exposes a hosted-control popup that can issue pairing codes, redeem/apply actor config through the native host, inspect local cloud actor status, and send a direct hosted browser command without env vars or CLI flags.
- [x] Validate hosted deterministic run path against a real Chrome session
  Status: live hosted runs completed end to end on March 25, 2026 (`19091e00-2ff3-4bf6-979f-52ab29a0abc7`, `5ad8267b-d643-49b8-91d6-bb07876726c3`, `d28880df-38e7-41a3-8b7e-53cda61a74bc`), including a control-plane restart with actor reconnect and no re-pair.
- [x] Validate direct remote command/result path against a real Chrome session
  Status: live commands succeeded on March 25, 2026 via the hosted server:
  `9d378d8e-6008-4ab5-bc19-aac380e1c0dc` (`get_active_tab`),
  `b5ed2bf5-323d-4237-8bed-c84700965ee0` (`navigate_to_url`),
  `9fe93cbd-6063-4a6c-a2e7-bda48e669f27` (`get_current_url`),
  `6605d777-dd9c-4b44-a2bd-972ad6962a12` (`get_dom_hash` with payload-driven `session_id`, verified again after a control-plane restart).
- [ ] Rehost LLM planner/orchestrator flows in the cloud
- [ ] Add policy prompt protocol and local enforcement hooks
- [ ] Add extension-direct lite mode after supervisor path is stable

## What Works (Do Not Change)
- `extension/src/background.ts` as the browser-side routing hub
- `extension/src/contentScript.ts` as the deterministic browser executor
- `rzn_core::StepKind` as the core action schema during migration
- `rzn_contracts::v1` as the stable typed action/snapshot/result baseline
- The DOM -> synthetic -> CDP escalation ladder
- Session-aware tab affinity via `session_id` and `current_tab_id`
- Persisted actor credentials and cloud actor registry state under `~/Library/Application Support/rzn/`
- The principle that targeting stays generic, not site-tuned
- Direct command runs are recorded as `workflow_name = "cloud.exec_command"` with one `RunStepRecord`

## Tried & Didn’t Work
- Treating the cloud as a direct replacement for native messaging: wrong boundary; the browser cannot native-message a remote server.
- Making the MV3 service worker the canonical durable actor: too fragile for the main architecture because of suspension/restart semantics.
- Sending arbitrary remote scripts: violates auditability, weakens safety, and does not fit the current typed execution posture.
- Replacing the existing broker/extension contracts wholesale: unnecessary churn; we already have usable local contracts worth preserving and wrapping.
- Treating the `example.com` outbound link as `*iana.org/domains/example*`: stale assumption; the current browser path lands on `https://www.iana.org/help/example-domains`, so the smoke workflow had to be updated.
- Letting `payload.session_id` diverge from the cloud envelope `session_id`: wrong; it created run records that did not describe the actual browser session. The control plane now normalizes single-command requests onto one authoritative session id before dispatch.
