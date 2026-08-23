---
title: "Developer guide"
subject: developer-guide
keywords: [setup, build, install, development]
part_of: overview
read_when: "You need to build or run the repository."
skip_when: "You only need to author a workflow. Open workflow-authoring."
---

# Developer guide

Use this guide to build and check the repository. The Makefile is the supported
entry point.

## Requirements

Install Rust, `sccache`, Bun, and a supported browser. The Makefile requires
`sccache` for Rust builds.

## Setup and build

Use Make targets from the repository root:

```sh
make setup
make build
make build-rust
make build-ext
make build-release
```

Use `make rust ARGS='check -p <crate>'` for a focused Rust command. Do not
disable the Rust cache on a development machine.

After `make build`, load `extension/dist/chrome` in
`chrome://extensions`. Source setup also creates Edge and Chromium bundles.

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
