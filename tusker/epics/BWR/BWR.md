---
schema: "tusker.epic/v5"
id: "BWR"
title: "Browser Worker Retirement"
type: "epic"
status: "active"
owner: "sarav"
summary: "Hard-remove the legacy rzn-browser-worker runtime and broker_endpoint_v1 compatibility surface so the supervisor is the sole browser automation owner for CLI, MCP, app, cloud, and native-host bridge paths."
created: "2026-05-14"
updated: "2026-05-14"
started: "2026-05-14"
transitions:
  - at: "2026-05-14T02:59:29Z"
    kind: "status"
    from: "draft"
    to: "active"
    actor: "codex"
    reason: "Browser worker retirement effort started after supervisor migration completed"
---

# BWR · Browser Worker Retirement

## Thesis

The supervisor migration is complete enough that `rzn-browser-worker` and
`broker_endpoint_v1.json` should stop being compatibility paths and become
removed code. Browser automation should ship as one durable local owner:
`rzn-browser supervisor`, with the Chrome extension and native host acting only
as browser transport.

## Scope

In:
- Removing `rzn-browser-worker` as a workspace member, packaged binary, plugin
  payload, release artifact, install target, and runtime fallback.
- Removing `broker_endpoint_v1.json` discovery/pruning as a browser-runtime
  contract.
- Removing supervisor, CLI, native-run, native-host, and rznapp flags or code
  paths that imply a worker fallback can still run.
- Updating runtime docs and Tusker canon from "deprecated compatibility" to
  "removed; supervisor is required".

Out:
- Removing the Chrome extension or native host transport.
- Removing generic plugin-worker infrastructure in rznapp for non-browser
  plugins.
- Preserving backwards compatibility for old worker-only browser installs.
- Rewriting workflow/action contracts beyond what the removal requires.

## Success metrics

- `rg` finds no live code/build/package dependency on `rzn-browser-worker`.
- `rg` finds no live browser-runtime dependency on `broker_endpoint_v1.json` or
  `rzn_broker_endpoint`.
- CLI, MCP browser, Reason app, native host, and cloud paths fail or heal through
  supervisor-only semantics, with no fallback flags.
- Release/plugin artifacts ship `rzn-browser` and `rzn-native-host`, not the
  worker.
- Tests and docs describe the supervisor-only runtime without deprecated worker
  language.

## Canon

- `tusker/domains/runtime/CANON.md`
- `tusker/domains/codebase/CANON.md`
- `docs/features/local_supervisor_runtime/README.md`
- `docs/features/browser_native_cli/README.md`
- `docs/features/desktop_app_bridge/README.md`

## Task stack

_Open tasks only. Closed/cancelled work is intentionally omitted; use `tusker list --epic BWR --type task --status done` for closed history._

- [[BWR-T-0001]] — Remove rzn-browser-worker from build, package, and release artifacts (review, p0, high)
- [[BWR-T-0002]] — Remove supervisor and CLI legacy worker fallback (review, p0, high)
- [[BWR-T-0003]] — Remove native-run and native-host broker endpoint compatibility (review, p0, high)
- [[BWR-T-0004]] — Delete rzn_broker_endpoint crate and endpoint-file contracts (review, p0, high)
- [[BWR-T-0006]] — Delete rzn-browser-worker crate source and tests (review, p0, high)
- [[BWR-T-0005]] — Update docs, Tusker canon, and verification for supervisor-only runtime (review, p1, medium)

## Open questions

- Whether `rzn_plan::BrokerClient` still owns a non-browser endpoint-file
  contract; if yes, split that from browser-runtime removal instead of deleting
  unrelated planner behavior by accident.
