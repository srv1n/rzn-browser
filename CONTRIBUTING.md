# Contributing

The smallest useful contribution is usually one workflow file. Site selectors
belong in workflow JSON, not in shared runtime code.

Read [`docs/system/00-overview.md`](docs/system/00-overview.md), then read
[`docs/system/workflow-authoring.md`](docs/system/workflow-authoring.md) before
you edit a workflow.

## Source setup

Install Rust, `sccache`, Bun, and a supported browser. Use Make targets:

```sh
make setup
make build
make test
```

Use `make rust ARGS='test -p <crate>'` for a focused Rust check. Do not call
Cargo or Bun directly for a project build. Load the built Chrome extension from
`extension/dist/chrome` and use a disposable browser profile for live tests.

## Workflow change

1. Copy a nearby file under `workflows/<system>/`.
2. Edit parameters, steps, output, help, and side effects.
3. Validate the file and the catalog.
4. Run a safe read-only smoke through the normal CLI path.

```sh
rzn-browser workflow validate workflows/<system>/<name>.json --strict --json
rzn-browser workflow inspect <system> <name>
rzn-browser workflow validate-catalog --strict --json
rzn-browser run <system> <name> --param <name>=<value>
```

For a write workflow, stop at draft or review unless the operator approved the
final action. Do not put credentials or page content in a workflow, test, log,
or commit.

## Code change

Trace the composition root and its callers before you edit. Keep the local
supervisor socket, native-host bridge, extension message routes, and contract
schemas aligned in one change. Put shared data shapes in the contract crate.
Keep site-specific behavior in workflows.

Run the smallest check that proves the change. Then run the wider check that
the change can affect. Report the exact command and result. A green focused
check does not prove an installed release or a live browser run.

## Broken workflows and security

When a selector has drifted, use the CLI output from:

```sh
rzn-browser report workflow-broken ...
```

Do not include private inputs or page content in a report. Send security issues
through [`SECURITY.md`](SECURITY.md), not a public issue.

## Review checklist

- The changed files have a clear owner and live caller.
- The workflow or code uses the current contract.
- Side effects are declared honestly.
- Tests cover the changed surface.
- No private data or generated output is committed.
- The report states source, install, runtime, and human proof separately.
