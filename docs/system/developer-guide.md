---
title: "Developer guide"
subject: developer-guide
keywords: [setup, build, install, development]
part_of: overview
kind: doc
lifecycle: current
code_conformance: unverified
read_when: "You need to build or run the repository."
skip_when: "You only need to author a workflow. Open workflow-authoring."
---

# Developer guide

Use this guide to build and check the repository. The Makefile is the supported
entry point. Do not invoke Cargo or Bun directly for project builds.

## Requirements

Install Rust, `sccache`, Bun, and a supported browser. The Makefile requires
`sccache` for Rust builds. CI and release runners set `RUSTC_WRAPPER: ""`
because `sccache` is absent there; keep that exception.

## Setup and build

Use Make targets from the repository root:

```sh
make setup
make build
make build-rust
make build-ext
make build-release
```

| Target | Result |
| --- | --- |
| `make setup` | Prepare the development environment. |
| `make build` | Build Rust and the extension. |
| `make build-rust` | Build the Rust workspace only. |
| `make build-ext` | Build the extension bundles only. |
| `make build-release` | Optimized Rust and extension builds. |
| `make install` | Build the runtime, register the native host, copy the stable extension payload, refresh the bundled workflow catalog. |

Use `make rust ARGS='check -p <crate>'` for a focused Rust command. Do not
disable the Rust cache on a development machine.

After `make build`, load `extension/dist/chrome` in `chrome://extensions`.
Source setup also creates Edge and Chromium bundles.

## Run locally

Use the normal run path:

```sh
make run W=workflows/google/google-search.json \
  PARAMS='--param search_query="browser automation"'
```

Or use the built CLI:

```sh
rzn-browser run google search --param search_query="browser automation"
```

The extension must be loaded and the native host must be registered. Use a
disposable browser profile for live tests.

## Useful checks

```sh
make test
make test-ext-unit
make test-ext-e2e
make schema-check
make doctor
```

`doctor` checks local files and registration. It does not prove a live browser
action. Read [Testing](testing.md) before you report a result.

## Skills and plugins

`skills/` holds packaged Agent Skills: a `SKILL.md` definition paired with
scripts under `scripts/`. Skill scripts call the `rzn-browser` binary and honor
`RZN_BROWSER_CLI` when set. Install or refresh them with
`rzn-browser skill install --global`, `skill update`, and `skill remove`.

`crates/rzn_plugin_devkit` scaffolds signed plugin packages that bundle the
supervisor binary, native host, and resource payloads per platform.
`examples/` holds non-core reference integrations such as `rzn_tool_explainer`,
`action_surface`, and `tauri_bridge`.
