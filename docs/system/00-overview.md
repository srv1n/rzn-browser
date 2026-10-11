---
title: "System overview"
subject: overview
keywords: [system, architecture, install, workflow]
kind: doc
lifecycle: current
code_conformance: unverified
read_when: "You need the map of the product."
skip_when: "You need one subsystem rule. Open that subsystem page."
---

# System overview

RZN Browser runs browser tasks on a local machine. It drives a browser profile
that you own, with your real sessions and logins. A workflow is one JSON file
that describes a repeatable task. An agent or a human runs it with one command.

The product exists to remove per-run LLM cost. A site is figured out once, by a
human or by `llm-auto`, and stored as a workflow. Every replay after that is
deterministic and needs no model.

## Execution path

```mermaid
flowchart TD
  cli["rzn-browser run"] --> runner["native_runner"]
  runner --> wf["workflow_runner"]
  wf -->|local JSON-RPC, authenticated socket| sup["supervisor"]
  sup -->|native messaging| nh["rzn-native-host"]
  nh --> ext["extension service worker"]
  ext --> cs["content script"]
  ext --> cdp["CDP fallback"]
  cs --> page["web page"]
  cdp --> page
```

Each hop has one job:

| Hop | Job |
| --- | --- |
| CLI (`rzn-browser`) | Parses commands, loads workflows, talks to the supervisor. |
| `workflow_runner` | Executes steps, retries, timeouts, and result assembly. |
| Supervisor | Owns local readiness, browser targets, runs, and method dispatch. |
| Native host | Forwards frames between the supervisor and the browser. It chooses no policy. |
| Extension | Performs the page work through content scripts or CDP. |

The supervisor, native host, and extension are the bridge. The browser is the
only component that touches the page.

## Coordination paths

Cloud and fleet are optional coordination layers. They do not replace the local
path.

- **Cloud** pairs an actor and dispatches commands over a WebSocket.
- **Fleet** enrolls a device, polls for jobs over HTTP, fetches workflow data,
  and posts results.

Both paths end in the same local supervisor and extension chain on a machine
that the operator owns.

## Reading order

Read the pages in this order:

1. [Architecture](architecture.md) — component boundaries and the run path.
2. [CLI and runtime](cli-runtime.md) — commands and local runtime state.
3. [Extension and native host](extension-native-host.md) — the browser bridge.
4. [Contracts and Rust SDK](contracts-sdk.md) — data shapes and embedding.
5. [Workflow authoring](workflow-authoring.md) — write or change a workflow.
6. [Installation and operations](installation-operations.md) — install and diagnose.
7. [Testing](testing.md) — pick a check, report proof.
8. [Release](release.md) — package and publish.
9. [Security](security.md) — permissions, side effects, and LLM data.
10. [Developer guide](developer-guide.md) — build and check the repository.
11. [Documentation guide](documentation.md) — write or update these pages.

## Key sources

| Path | Role |
| --- | --- |
| [`crates/rzn_browser/src/main.rs`](../../crates/rzn_browser/src/main.rs) | Command definitions. |
| [`crates/rzn_browser/src/native_runner.rs`](../../crates/rzn_browser/src/native_runner.rs) | CLI adapter into the runtime. |
| [`crates/rzn_browser/src/workflow_runner/`](../../crates/rzn_browser/src/workflow_runner/) | Shared execution loop. |
| [`crates/rzn_browser/src/supervisor.rs`](../../crates/rzn_browser/src/supervisor.rs) | Local method router and runtime state. |
| [`extension/src/background.ts`](../../extension/src/background.ts) | Extension service worker. |
| [`workflows/`](../../workflows/) | Built-in workflow catalog. |

<!-- tusker:docs-map:begin -->
```mermaid
graph TD
  n_architecture["Architecture"]
  n_cli_runtime["CLI and runtime"]
  n_contracts_sdk["Contracts and Rust SDK"]
  n_developer_guide["Developer guide"]
  n_documentation["Documentation guide"]
  n_extension_native_host["Extension and native host"]
  n_installation_operations["Installation and operations"]
  n_overview["System overview"]
  n_release["Release"]
  n_security["Security and privacy"]
  n_testing["Testing"]
  n_workflow_authoring["Workflow authoring"]
  n_overview --> n_architecture
  n_architecture --> n_cli_runtime
  n_architecture --> n_contracts_sdk
  n_overview --> n_developer_guide
  n_overview --> n_documentation
  n_architecture --> n_extension_native_host
  n_developer_guide --> n_installation_operations
  n_overview --> n_release
  n_overview --> n_security
  n_overview --> n_testing
  n_overview --> n_workflow_authoring
```
<!-- tusker:docs-map:end -->
