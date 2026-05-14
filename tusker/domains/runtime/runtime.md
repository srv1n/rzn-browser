---
schema: tusker.knowledge/v6
title: Runtime state and readiness reference
node: runtime/runtime
audience: developer
agent_layer: capsule
kind: reference
domains:
- runtime
source_of_truth:
- crates/rzn_browser/src/supervisor.rs
- crates/rzn_browser/src/native_runner.rs
- crates/rzn_native_host/src/main.rs
canonical_status: draft
created: '2026-05-08'
updated: '2026-05-13'
domain: runtime
stale_when:
  paths:
  - crates/rzn_browser/src/supervisor.rs
  - crates/rzn_browser/src/native_runner.rs
  - crates/rzn_native_host/src/main.rs
publish:
  lane: internal
  path: runtime/runtime
  include_in_llms: true
summary: Supervisor readiness, bridge state, endpoint files, and compatibility rules.
---

# Runtime state and readiness reference

## Read this when

Read this when debugging `runtime.status`, `runtime.ensure_ready`, native-host bridge availability, stale endpoint files, or legacy worker fallback.

## Do not read this when

Do not use this as the target architecture overview; read [[runtime/CANON]] first.

## Readiness model

Supervisor readiness is layered:

| Layer | Healthy signal | Failure shape |
|---|---|---|
| Supervisor process | `runtime.status` responds over `rzn.local.v1`. | Socket/token missing, stale socket, handshake timeout. |
| Native-host bridge | `native_host_bridge.connected == true`. | Chrome has not launched host, extension asleep, host cannot find supervisor. |
| Bridge probe | Supervisor can call extension through `native_host.extension_call`. | Extension timeout, missing keepalive capability, MV3 restart. |
| Legacy worker | Optional `rzn.worker.health` response. | Only relevant when fallback is explicitly allowed. |

`runtime.ensure_ready` is allowed to prune stale legacy endpoints and wait/probe for the native-host bridge. It should return structured diagnostics rather than hiding a degraded extension state.

## Endpoint authority

`broker_endpoint_v1.json` and pid/socket files are compatibility hints. They are stale until proven live through handshake. The supervisor socket/token pair is the preferred discovery path.

## Practical diagnostics

- `rzn-browser supervisor status` or the equivalent `runtime.status` call checks supervisor availability.
- `rzn-browser heal` should be used for repair-oriented checks.
- Stale worker symptoms usually present as timeout or dead pid/socket references; prune before retrying.
- If the extension/native-host connection is missing in product-path work, ask the operator to reload/reconnect the existing Chrome session instead of switching automation stacks.

## Source of truth

- `crates/rzn_browser/src/supervisor.rs`
- `crates/rzn_browser/src/native_runner.rs`
- `crates/rzn_native_host/src/main.rs`

## Related

- [[runtime/CANON]]

## Recent changes

<!-- tusker:backrefs:begin -->
- [[OPS-T-0002]] - Replaced generated scaffold with runtime readiness reference.
<!-- tusker:backrefs:end -->
