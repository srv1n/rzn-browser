# CDP Break-Glass (Design)

## Overview
- Goal: make CDP an explicit, opt-in capability used only when strictly needed (“break glass”), while keeping extension-first automation as the default control plane.
- Constraints: avoid silently attaching DevTools/CDP in normal runs; keep module boundaries clean; do not require CDP for core deterministic action execution. Privacy/redaction is deferred to round 2.

## Flow Diagrams
- End-to-end escalation flow
```
Host app → (extension-first) observe/act
        → detects repeated failure / missing capability
        → request enable_debug(mode=enrichment|rescue)  [policy gated]
        → (optional) CDP enrich snapshot or rescue action
        → detach CDP (time-bounded)
```

- Internal flow (capability ladder)
```
Tier 0: DOM-only (default)
Tier 1: CDP enrichment (read-only, rare)
Tier 2: CDP rescue (action execution, very rare)
```

## Decision Record
- Chosen: capability-based, opt-in escalation with explicit protocol/contract surface.
- Alternatives:
  - “Always attach CDP”: increases detectability/fragility; harder to justify for hostile sites.
  - “No CDP ever”: reduces recovery options for cross-origin/iframe edge cases.

## Architecture
- Modules
  - Extension: actor primitives + DOM snapshot generation; remains deterministic.
  - Broker: transport + session routing; should not become an LLM brain.
  - Host: owns policy and when/if to request escalation.
- Data contracts (planned)
  - Snapshots advertise capabilities: `cdp_available`, `cdp_attached`, `frame_observability`, etc.
  - Explicit action/handshake: `enable_debug(mode)` returning `attached|denied|unsupported`.

## Implementation Notes
- Policy-gate CDP enablement (per session/domain, time-bounded).
- Prefer Tier 1 enrichment before Tier 2 rescue.
- Detach immediately after a rescue step unless explicitly extended.

## Tasks & Status
- [x] Define capability fields in versioned contracts (v1 extension).
- [x] Add enable_debug handshake and broker routing.
- [x] Add time-bounded detach (lease-based) and capability reporting.

### Current Behavior (v1)
- CDP remains **disabled by default** in the extension (`flags.cdpEnable=false`).
- Host can explicitly request break-glass enablement via `execute_static` → `enable_debug`.
- Host-side minimal policy gate: set `RZN_ALLOW_CDP=1` to allow `enable_debug` requests.

## What Works (Do Not Change)
- Default execution remains extension-first (DOM-only).
- CDP enablement must be explicit and policy gated.

## Tried & Didn’t Work
- N/A (design-only placeholder; update as experiments occur).
