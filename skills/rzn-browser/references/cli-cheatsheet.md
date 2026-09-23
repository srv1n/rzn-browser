# CLI reference

Use installed `--help` when a release differs from these examples.

## Find and inspect

```bash
rzn-browser workflow list google
rzn-browser workflow list google search
rzn-browser workflow inspect google/search --json
rzn-browser workflow dirs
```

Detailed help supplies parameters, output fields, examples, and the source file.
Use explicit file paths when checking an edited workflow rather than an installed copy.

## Run and save results

```bash
rzn-browser run google/search --param search_query="browser automation"
rzn-browser run /absolute/path/to/workflow.json --param key="value"
rzn-browser run google/search --param search_query="browser automation" --output-file /path/to/results.json
```

`--output-file` writes the final result (Markdown if present, otherwise JSON).
`--download-dir` downloads returned asset URLs and external links; use it only
when acquiring those files is part of the request.

Runs normally use dedicated tabs. `--keep-tab-open` retains one for inspection;
`--tab-ref <observed-tab-ref>` reuses an exact existing tab. `--snapshot on-error`
controls diagnostic snapshots, not proof that a requested screenshot was produced.

## Autonomous mode

```bash
rzn-browser llm-auto "Inspect the pricing page and report plans; do not submit forms" --url "https://example.com" --max-steps 12 --json
```

Real exploration requires a configured provider. Put task constraints in the
instruction. Supported flags include `--url`, `--max-steps`, `--json`, and
`--pure-llm` (disables deterministic fast paths). The current CLI has no
`--save-workflow`, `--context`, `--constraint`, or `--prefer-cached` flags.
Translate observations into workflow JSON when reuse is requested.

## Import a finished workflow

```bash
rzn-browser workflow validate /path/to/workflow.json --strict --json
rzn-browser workflow add /path/to/workflow.json --system custom --name my-flow
```

Import changes the user catalog. Use `--force` only for an intended replacement.
Catalog refresh and runtime installation are separate operations.
