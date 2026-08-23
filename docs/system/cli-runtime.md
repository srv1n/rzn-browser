---
title: "CLI and runtime"
subject: cli-runtime
keywords: [CLI, supervisor, workflow, cloud, fleet]
part_of: architecture
read_when: "You need the command or local runtime state."
skip_when: "You need bridge internals. Open extension-native-host."
---

# CLI and runtime

The command-line interface starts workflows and manages the local runtime. Use
one command for each operation.

The command definitions live in [`main.rs`](../../crates/rzn_browser/src/main.rs).

| Need | Command |
| --- | --- |
| Run a known workflow | `rzn-browser run <system> <workflow>` |
| List workflows | `rzn-browser workflow list` |
| Inspect a workflow | `rzn-browser workflow inspect <system> <workflow>` |
| Validate a workflow | `rzn-browser workflow validate <path> --strict --json` |
| Validate the catalog | `rzn-browser workflow validate-catalog --strict --json` |
| Check supervisor state | `rzn-browser supervisor status` |
| Wait for browser readiness | `rzn-browser supervisor ensure-ready` |
| List browser bridges | `rzn-browser browser targets` |
| Check installation | `rzn-browser native-host doctor --browser chrome --json` |
| Discover an unknown task | `rzn-browser llm-auto "<task>"` |
| Serve MCP tools | `rzn-browser mcp browser` |
| Manage owned devices | `rzn-browser fleet enroll`, `status`, or `disable` |

## Run path

```text
rzn-browser run
  -> load and validate workflow
  -> start or connect to supervisor
  -> runtime.ensure_ready
  -> execute steps through local JSON-RPC
```

The entry functions are [`handle_run`](../../crates/rzn_browser/src/main.rs)
and [`run_supervisor_workflow`](../../crates/rzn_browser/src/native_runner.rs).
The supervisor method router is [`SupervisorState::dispatch`](../../crates/rzn_browser/src/supervisor.rs).

## Local state

The runtime root is selected by [`runtime_paths.rs`](../../crates/rzn_core/src/runtime_paths.rs).
It contains, as applicable:

- `run/` for the supervisor socket and lock.
- `secure/` for the supervisor token and private artifacts.
- `workflows/builtin/` for installed workflows.
- `workflows/user/` for user workflows.
- Saved run data managed by [`run_store.rs`](../../crates/rzn_browser/src/run_store.rs).

Use `--app-base` for a separate local root. Set fleet path variables too when
you need a disposable fleet test. A supervisor restart clears in-memory
browser sessions. It does not remove saved workflow results or user workflows.

## Readiness

`connected: true` is not enough. Readiness also checks bridge response health,
required extension capabilities, target identity, and native transport. Use
`supervisor ensure-ready` before a run. Use `heal` when the readiness report
provides a recovery path.

## Cloud and fleet

Cloud and fleet are coordination layers. Cloud pairs an actor and dispatches
commands over its own connection. Fleet enrolls a device, polls for jobs, fetches
workflow data, and posts results. Each job still enters the local supervisor and
extension path.
