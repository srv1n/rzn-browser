# RZN Browser

RZN Browser runs repeatable browser work in a browser that you own. It uses
the browser profile that you open. It does not start a rented browser.

The normal path is:

```text
rzn-browser run
  -> native runner
  -> workflow runner
  -> local supervisor socket
  -> native host
  -> browser extension
  -> page
```

Cloud and fleet commands coordinate machines that you own. The browser action
still runs on one of those machines.

## Install

Use a packaged release on a new machine:

```sh
# macOS or Linux
curl -fsSL https://raw.githubusercontent.com/srv1n/rzn-browser/main/install.sh | sh

# Windows PowerShell
irm https://raw.githubusercontent.com/srv1n/rzn-browser/main/install.ps1 | iex
```

Load the unpacked extension from the installed `extension/dist/chrome`
directory in `chrome://extensions`. Turn on Developer mode first. The release
installer registers Chrome. A source install can register Chrome, Edge, and
Chromium.

Check the local bridge:

```sh
rzn-browser native-host doctor --browser chrome --json
rzn-browser supervisor status
rzn-browser browser targets
rzn-browser supervisor ensure-ready
```

## Run a workflow

List the installed catalog, then run a workflow by its system and name:

```sh
rzn-browser workflow list google
rzn-browser run google search --param search_query="browser automation"
```

The workflow uses the open browser profile. Keep the profile signed in when a
site needs authentication. Use `llm-auto` when the task is not known yet:

```sh
rzn-browser llm-auto "Find the first three results for browser automation"
```

The MCP server exposes the same supervisor tools over standard input and
standard output:

```sh
rzn-browser mcp browser
```

## Workflow catalog

Workflow files live in `workflows/<system>/`. The CLI is the source of truth for
the installed catalog:

```sh
rzn-browser workflow list
rzn-browser workflow inspect google search
rzn-browser workflow validate workflows/google/google-search.json --strict --json
rzn-browser workflow validate-catalog --strict --json
```

Use `rzn-browser workflow add` to import a workflow into the user catalog. See
[`workflows/README.md`](workflows/README.md) and
[`docs/system/workflow-authoring.md`](docs/system/workflow-authoring.md) before
you add one.

## Build and test

Use the Makefile. Rust builds require `sccache`.

```sh
make setup       # local debug setup
make build       # Rust debug build and Chrome extension
make build-release
make test
make test-ext-unit
make test-ext-e2e
make schema-check
```

`make doctor` checks files and registration. It does not prove a live browser
action. A source build, an installed bundle, a live browser run, and human
acceptance are separate proof steps.

## Repository map

| Path | Purpose |
| --- | --- |
| `crates/rzn_browser/` | CLI, supervisor, workflow catalog, runs, cloud, and fleet. |
| `crates/rzn_native_host/` | Browser native-messaging transport to the supervisor. |
| `crates/rzn_core/` | Framing, runtime paths, secure files, and generated action types. |
| `crates/rzn_contracts/` | Workflow, run-result, local, cloud, and fleet data shapes. |
| `crates/rzn_plan/` | Planning and LLM providers. |
| `crates/rzn_sdk/` | Rust embedding APIs. |
| `extension/` | Browser service worker, content scripts, page bridge, UI, and tests. |
| `workflows/` | Shipped workflow manifests. |
| `schema/` | Action and result schemas used by code and code generation. |
| `skills/` | Agent skill packs and workflow wrappers. |
| `scripts/` | Build, install, release, and validation tools. |
| `docs/system/` | Current system documentation. |

## Security

The extension can read and change pages that the browser profile can access. A
workflow is trusted code for that profile. Review side effects before you run a
workflow that posts, sends, uploads, votes, follows, or deletes.

Page-bridge calls run in the page world. Page JavaScript can see that bridge;
do not treat it as a secret boundary. Keep tokens, page data, snapshots, and
logs out of Git. Read [`docs/system/security.md`](docs/system/security.md) and
[`SECURITY.md`](SECURITY.md) before handling private data.

## Documentation

Start with [`docs/system/00-overview.md`](docs/system/00-overview.md). Then use:

- [`docs/system/architecture.md`](docs/system/architecture.md)
- [`docs/system/cli-runtime.md`](docs/system/cli-runtime.md)
- [`docs/system/extension-native-host.md`](docs/system/extension-native-host.md)
- [`docs/system/installation-operations.md`](docs/system/installation-operations.md)
- [`docs/system/testing.md`](docs/system/testing.md)
- [`docs/system/release.md`](docs/system/release.md)

For contribution rules, read [`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

Runtime code in `crates/` and `extension/` uses AGPL-3.0-only. Workflows,
skills, and schemas use the licenses in their own directories.
