---
title: "Architecture"
subject: architecture
keywords: [runtime, supervisor, extension, native-host, workflow]
part_of: overview
read_when: "You need a component boundary or execution path."
skip_when: "You need a build command. Open developer-guide."
---

# Architecture

A workflow uses one local execution chain. Each component has one job. The
browser performs the final page action.

## Execution path

```text
rzn-browser run
      |
      v
native_runner.rs
      |
      v
workflow_runner/
      |
      | local JSON-RPC over an authenticated socket
      v
supervisor
      |
      | native_host.extension_call
      v
rzn-native-host -- browser native messaging -- extension
      |
      v
content scripts, page bridge, or CDP
      |
      v
web page
```

The `run` handler is in [`main.rs`](../../crates/rzn_browser/src/main.rs).
The CLI adapter is [`native_runner.rs`](../../crates/rzn_browser/src/native_runner.rs).
The shared loop is [`workflow_runner`](../../crates/rzn_browser/src/workflow_runner/).
The supervisor owns the local method router and browser bridge.

Cloud and fleet are separate control paths. Cloud uses a paired actor and a
WebSocket connection. Fleet uses device enrollment, HTTP polling, workflow
fetch, and result posting. Both paths call the same local browser runtime on a
machine that the operator owns.

## Main code areas

| Path | Role |
| --- | --- |
| [`crates/rzn_browser`](../../crates/rzn_browser/) | CLI, supervisor, workflow catalog, saved runs, cloud, and fleet. |
| [`crates/rzn_native_host`](../../crates/rzn_native_host/) | Browser native-messaging transport to the supervisor. |
| [`crates/rzn_core`](../../crates/rzn_core/) | Framing, runtime paths, secure files, and generated action types. |
| [`crates/rzn_contracts`](../../crates/rzn_contracts/) | Workflow, result, cloud, and fleet data shapes. |
| [`crates/rzn_plan`](../../crates/rzn_plan/) | Planning, LLM providers, and recovery helpers. |
| [`crates/rzn_sdk`](../../crates/rzn_sdk/) | Rust embedding APIs and native-host helpers. |
| [`extension`](../../extension/) | Service worker, content scripts, page bridge, UI, and browser tests. |
| [`workflows`](../../workflows/) | Workflow manifest files. |

## Authority boundaries

- `rzn-browser run` is the normal deterministic run surface.
- `workflow_runner` owns parameter handling, step execution, retry, timeout,
  output selection, and run-result assembly.
- The supervisor owns readiness, bridge inventory, sessions, saved runs,
  settings, and method dispatch.
- The native host forwards frames. It does not choose workflow policy.
- The extension owns page actions and browser-side health.
- Contract crates and `schema/actions.json` define data shapes. Do not copy
  a shape into a second module.

Read [`supervisor.rs`](../../crates/rzn_browser/src/supervisor.rs),
[`manifest.base.json`](../../extension/src/manifest.base.json), and
[`rzn_contracts`](../../crates/rzn_contracts/src/lib.rs) when a boundary is
unclear.
