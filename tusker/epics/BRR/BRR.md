---
schema: "tusker.epic/v5"
id: "BRR"
title: "Bridge Reliability Hardening"
type: "epic"
status: "active"
owner: "sarav"
summary: "Make the supervisor-native-host-extension readiness path recover from MV3 service-worker suspension, stale native-host handles, and stale extension bundles without routine user reloads."
created: "2026-05-14"
updated: "2026-05-14"
started: "2026-05-14"
transitions:
  - at: "2026-05-14T07:53:38Z"
    kind: "status"
    from: "draft"
    to: "active"
    actor: "codex"
    reason: "Started focused bridge reliability hardening after repeated reload-extension readiness failures"
---

# BRR · Bridge Reliability Hardening

## Thesis
The repeated "reload extension" failures are not one bug. They are a fragile
readiness state machine across four owners: CLI, supervisor, Chrome native host,
and MV3 service worker. Prior LRT work fixed real failures, but the run path
still treats a live native-host stdio handle as too close to "extension ready",
while MV3 can suspend, restart, or run an older bundle independently.

This epic owns the bridge-readiness hardening work required to make cold-start,
post-idle, and stale-bundle failures recover automatically or fail with a
specific, actionable diagnosis.

## Scope

In:
- CLI run-path readiness and heal behavior before first workflow step.
- Supervisor native-host bridge probing, stale-handle teardown, and diagnostics.
- Extension MV3 service-worker native messaging reconnect behavior.
- Capability/build-signature handling for stale extension bundles.
- Native-host/SW heartbeat and health-beacon design if cheap recovery is not
  enough.
- Regression coverage for cold start, idle eviction, stale handle, stale bundle,
  and true bridge-down cases.

Out:
- Replacing the Chrome extension/native-host product path with Playwright,
  WebDriver, or a temporary Chrome profile.
- Reintroducing `rzn-browser-worker` or `broker_endpoint_v1` as runtime authority.
- Silent replay of side-effectful workflow steps after dispatch.
- Broad runtime rewrites unrelated to supervisor/native-host/extension readiness.

## Success metrics

- `rzn-browser run ...` after long idle does not require a manual extension
  reload when Chrome and the extension are installed and enabled.
- `native_host_bridge.connected=true,responsive=false` is either auto-healed
  before the first step or classified as a concrete failure type.
- Stale extension bundles report expected/loaded build or capability mismatch
  directly instead of the generic bridge readiness message.
- Probe timeouts retire stale supervisor bridge handles instead of preserving a
  dead fd as "connected".
- The steady hot path stays fast; expensive heal logic runs only after stale or
  failed readiness signals.
- Verification covers Rust supervisor/run-path tests, extension build/tests,
  native-host tests when heartbeat changes land, and at least one real
  extension/native-host smoke.

## Canon

- `docs/features/bridge_reliability_hardening/README.md`
- `tusker/domains/runtime/CANON.md`
- `docs/features/local_supervisor_runtime/README.md`
- Historical context only: `docs/features/connection_reliability/README.md`
- Closed predecessor tasks: `LRT-T-0008`, `LRT-T-0010`

## Task stack

_Open tasks only. Closed/cancelled work is intentionally omitted; use `tusker list --epic BRR --type task --status done` for closed history._

- [[BRR-T-0006]] — Add sub-30s bridge heartbeat and health beacon (blocked, p1, high)
- [[BRR-T-0007]] — Add supervisor queue and profile bridge scheduler (backlog, p1, high)
- [[BRR-T-0008]] — Add macOS and Windows bridge install doctor parity (backlog, p1, high)

## Current status

BRR remains active while [[BRR-T-0006]] is being implemented and follow-up
hardening stays tracked in [[BRR-T-0007]] and [[BRR-T-0008]]. The cheaper
recovery and diagnostics work is complete through [[BRR-T-0005]], but live
repeated-workflow evidence showed that a zombie native-host/native-port path
can still require manual extension reload. The epic should not be closed unless
zombie recovery, supervisor queueing, and macOS/Windows bridge diagnostics are
completed, cancelled with replacement evidence, or split into a future
reliability epic.

## Open questions

- What is the smallest `BRR-T-0006` implementation that recreates a zombie
  native-host/native-port path without manual extension reload?
- Is a bounded heartbeat required after zombie recovery exists, or is beaconed
  request/response health enough?
- Should stale keepalive capability remain a hard readiness gate, or become a
  per-workflow capability requirement?
- What exact timeout budget keeps cold-start recovery robust without making
  real bridge-down failures feel hung?
