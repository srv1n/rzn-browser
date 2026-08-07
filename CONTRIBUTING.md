# Contributing

Most useful contributions to RZN Browser are workflows, not runtime changes. A workflow is a
single JSON file under `workflows/<system>/`, so adding support for a new site usually means
adding one file and one doc — no Rust required.

## Build From Source

You need a Rust toolchain, [Bun](https://bun.sh) for the extension build, and Google Chrome.

```sh
cargo build --release -p rzn-browser -p rzn-native-host
cd extension && bun install && bun run build
```

For a full local install (runtime, native host, extension payload, bundled catalog):

```sh
make install
```

Then load the unpacked extension from `extension/dist/chrome` at `chrome://extensions` with
Developer mode enabled. See [docs/BROWSER_DEV_LOOP.md](docs/BROWSER_DEV_LOOP.md) for the
day-to-day dev loop.

## Authoring A Workflow

1. Copy a nearby workflow from `workflows/<system>/` — starting from something close beats
   starting from a blank file.
2. Edit the JSON against your real browser session until the flow is stable.
3. Fill in the help block and validate:

   ```sh
   rzn-browser workflow validate workflows/<system>/<workflow>.json --write-help
   ```

   `--write-help` fills in the boring param docs and then tells you what still needs a human
   pass. Fix whatever it flags, then re-run with `--strict --json` for the final check.
4. Confirm the callable contract and the catalog still validate:

   ```sh
   rzn-browser workflow inspect <system> <workflow>
   rzn-browser workflow validate-catalog --strict --json
   ```
5. Smoke it through the normal run path:

   ```sh
   rzn-browser run <system> <workflow> --param <name>=<value>
   ```

   For workflows that write, post, or send anything, smoke only up to the draft/review step
   unless you explicitly intend the irreversible action.

The authoring rules that the validator enforces — parameter types, side-effect classes, tab and
session policy — are documented in [workflows/README.md](workflows/README.md). Read that before
your first workflow.

## Submitting

The expected shape of a workflow submission (files, docs, examples, what makes one easy to
accept) is described in the [Submit Workflows Back To The Repo](README.md#submit-workflows-back-to-the-repo)
section of the README. Follow it.

A few things that apply to every PR:

- Keep site-specific selectors and page logic inside the workflow JSON. Shared engine code stays
  generic.
- Run `cargo fmt`, `cargo clippy`, and `cargo test` if you touched Rust; `bun run test` in
  `extension/` if you touched the extension.
- Keep commits conventional (`feat:`, `fix:`, `docs:`, `chore:`) with a subject and, when the
  "why" is not obvious, a body.

## Bugs And Broken Workflows

Sites change their DOM and workflows break. That is expected, and reporting it is genuinely
useful. When a workflow fails, the CLI prints a self-contained
`rzn-browser report workflow-broken ...` command listing exactly the non-private fields it would
send. Paste that command into the **Workflow broken** issue template — it tells us the system,
workflow, version, failing step, and error class without leaking your inputs or page content.

For anything security-related, do not open an issue — see [SECURITY.md](SECURITY.md).
