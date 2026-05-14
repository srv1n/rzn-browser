---
schema: tusker.epic/v6
id: LRT
title: Local Browser Runtime Supervisor
status: done
owner: sarav
summary: Unify browser automation runtime ownership around a durable rzn-browser supervisor
  so CLI, MCP, Reason app, and cloud jobs share one stable local contract while the
  Chrome extension and native host remain thin browser transport pieces.
created: '2026-05-08'
updated: '2026-05-13'
started: '2026-05-08'
completed: '2026-05-13'
verified_by: codex
verified_at: '2026-05-13T03:15:36Z'
closed_by: codex
closed_at: '2026-05-13T03:15:36Z'
transitions:
- at: '2026-05-08T07:11:26Z'
  kind: status
  from: draft
  to: active
  actor: codex
  reason: Architecture approved; implementation backlog created
- at: '2026-05-13T03:15:36Z'
  kind: status
  from: active
  to: done
  actor: codex
  reason: All LRT tasks closed and verification gates passed
primary_domains:
- runtime
knowledge_nodes:
- runtime/canon
created_at: '2026-05-08'
updated_at: '2026-05-13'
---

# LRT · Local Browser Runtime Supervisor

## Thesis
Browser automation needs one durable local owner. The Chrome extension and native host are mandatory browser transport pieces, but neither the Reason app nor `rzn-browser-worker` should be a hidden prerequisite for CLI, MCP, or cloud jobs. `rzn-browser` should become the single shipped browser-native binary with supervisor, native-host, CLI, MCP, and heal modes.

## Scope

In:
- `rzn-browser supervisor` local runtime process and IPC contract.
- Native-host bridge refactor so Chrome-launched processes forward to supervisor.
- CLI, MCP, cloud, and Reason app producer migration.
- Restart/heal diagnostics and integration tests across app, host, extension, and supervisor churn.
- First-class documentation for the launch topology.

Out:
- Removing the Chrome extension.
- Replacing the main browser path with Playwright/WebDriver.
- Making the Reason app mandatory for browser automation.
- Rewriting the typed browser action DSL without a runtime need.

## Success metrics

- CLI browser jobs run without Reason.app open.
- MCP clients can launch `rzn-browser mcp browser` directly and execute browser tools.
- Cloud jobs lease/execute/report through supervisor without native-host-owned cloud state.
- Reason.app submits browser tasks as a supervisor client instead of a browser worker owner.
- Restart matrix covers supervisor, Chrome/native-host, extension service-worker, CLI, MCP, app, and cloud paths with no manual pid/socket cleanup.

## Canon

- Architecture: `docs/features/local_supervisor_runtime/README.md`.
- Current CLI/runtime compatibility: `docs/features/browser_native_cli/README.md`.
- Current reliability history: `docs/features/connection_reliability/README.md`.
- Cloud control plane: `docs/features/cloud_control_plane/README.md`.
- Desktop app bridge: `docs/features/desktop_app_bridge/README.md`.

## Task stack

_Open tasks only. Closed/cancelled work is intentionally omitted; use `tusker list --epic LRT --type task --status done` for closed history._

_No open tasks._

## Open questions

- Exact supervisor startup policy on macOS: LaunchAgent, lazy auto-start, or both.
- Whether `rzn-native-host` remains a symlinked argv mode during migration or is replaced immediately by `rzn-browser native-host`.
- How long `broker_endpoint_v1.json` remains as a legacy attach hint.
