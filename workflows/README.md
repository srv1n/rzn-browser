# Workflow catalog

Each production workflow is one JSON file under `workflows/<system>/`. The
installed CLI copies these files into its built-in catalog. User workflows live
in a separate user catalog.

## Manifest shape

Use the workflow manifest contract:

- `schema_version: "rzn.workflow_manifest"`
- stable `id`, `name`, `system`, and `capability`
- typed input definitions in `params.properties`
- ordered browser actions in `steps`
- honest top-level and step side effects
- output selection and output schema
- user-facing examples and notes in `help`

Do not add sidecar manifest files. Keep one canonical workflow file per route.

## Run and inspect

Use the CLI. Do not rely on a table in this file for the current catalog.

```sh
rzn-browser workflow list
rzn-browser workflow list google
rzn-browser workflow inspect google search
rzn-browser workflow validate workflows/google/google-search.json --strict --json
rzn-browser workflow validate-catalog --strict --json
rzn-browser run google search --param search_query="browser automation"
```

`workflow pull` refreshes the built-in catalog. `workflow add` imports a file
or directory into the user catalog. Check the resolved source when a user and a
built-in workflow have the same identity:

```sh
rzn-browser workflow dirs
rzn-browser workflow list --all-sources
```

## Shipping a catalog update

The catalog ships on its own cadence, separate from the CLI and the extension.
A selector fix is JSON only, so it never needs a runtime rebuild.

```sh
rzn-browser workflow pull                          # canonical catalog from the channel
rzn-browser workflow pull --catalog-version 0.1.4  # a retired catalog version
rzn-browser workflow pull --ref main               # a branch, before it is published
rzn-browser workflow pull --repo-root .            # a local checkout
```

The channel is the rolling `workflows-latest` GitHub release. It always holds one
canonical `rzn-browser-workflows.tar.gz`, plus every retired catalog kept under its
version number as `rzn-browser-workflows-<version>.tar.gz`.

To publish, tag the catalog and push the tag:

```sh
git tag workflows-v0.1.5
git push origin workflows-v0.1.5
```

`.github/workflows/release-catalog.yml` then validates the manifests, runs every
`workflows/**/*.test.mjs`, builds the bundle, and republishes the channel. Runtime
tags (`vX.Y.Z`) stay in `release.yml` and keep shipping the CLI and the extension
together.

Run the same gate locally before tagging:

```sh
python scripts/release/check_workflow_catalog.py
node workflows/chatgpt/chatgpt_send_picker.test.mjs
```

`pull` warns when a `user/` fork shadows a builtin workflow it just installed. Do
not ignore that warning: a fork outranks builtin everywhere outside a repo
checkout, so the update you just installed will not run.

## Parameters

Use the narrowest honest type:

| Type | Use |
| --- | --- |
| `string` | Text, ids, URLs, labels, and modes. |
| `integer` | Counts, limits, days, and retry values. |
| `number` | Non-integer numbers. |
| `boolean` | A true/false switch. |
| `object` | Structured values. |
| `array` | A real list of values or files. |

Mark secret inputs as sensitive. Do not put a secret value in the JSON file.

## Side effects

Declare every effect that a workflow can produce:

| Class | Meaning |
| --- | --- |
| `read_only` | Reads browser-visible data without changing it. |
| `external_read` | Reads from a remote origin. |
| `network_access` | Makes an outbound request. |
| `browser_state` | Changes navigation, tabs, focus, DOM, or session state. |
| `file_write` | Writes a local file. |
| `download` | Starts or records a download. |
| `external_write` | Changes a remote service or account. |
| `auth` | Uses or changes login state. |
| `destructive` | Deletes or performs a high-risk mutation. |

The CLI checks post-processing too. `--output-file` needs `file_write`.
`--download-dir` needs `download`, `file_write`, `external_read`, and
`network_access`.

These declarations are a policy check, not a sandbox. Review actions such as
`execute_javascript` and `same_origin_request` as code.

## Tabs and sessions

Prefer a dedicated workflow tab. Use
`runtime.requires_existing_session: true` only when an already-open state is
required. Do not add manual-debug fields such as `use_current_tab` or
`current_tab_id` to a production workflow.

## Authoring gate

Before review:

1. Validate the changed file.
2. Inspect its callable contract.
3. Validate the effective catalog.
4. Run a read-only smoke through `rzn-browser run`.
5. Stop a mutating flow at draft or review unless final approval is explicit.

See [`docs/system/workflow-authoring.md`](../docs/system/workflow-authoring.md)
for the full authoring loop.
