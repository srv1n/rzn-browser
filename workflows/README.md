# RZN Workflows

This directory contains the production browser workflow catalog. Each shipped
workflow is one manifest-shaped JSON file under `workflows/<system>/`.

## Authoring Standard

- Keep the normal filename, for example `workflows/google/google-search.json`.
- Put `schema_version: "rzn.workflow_manifest"` in the JSON body.
- Do not create production `*.manifest.json` or `*.manifest.v2.json` sidecars.
- Declare typed inputs in `params.properties`.
- Declare top-level and step-level side effects honestly.
- Put executable actions in `steps[]`.
- Declare the returned value in `result.output_selector` and
  `result.output_schema`.
- Keep human-facing examples and notes in `help`; do not rely on help as the
  machine contract.

The canonical developer guide is
[`docs/features/workflow_contract_platform/AUTHORING_GUIDE.md`](../docs/features/workflow_contract_platform/AUTHORING_GUIDE.md).

## Installed Runtime Behavior

- `make install` copies the shipped catalog into
  `~/Library/Application Support/RZN/workflows/builtin`.
- `rzn-browser workflow pull` refreshes that builtin catalog from the latest
  release payload.
- Packaged examples from `examples/browser_automation/` install under the
  `examples/*` namespace.
- Preferred deterministic run surface is
  `rzn-browser run <system> <workflow>`.
- `rzn-browser list <system>` keeps catalog output compact.
- `rzn-browser list <system> <workflow>` shows detailed workflow help.
- `rzn-browser workflow inspect <system> <workflow>` shows the callable
  manifest contract: inputs, optionality, types, side effects, runtime, and
  output shape.
- `rzn-browser workflow validate <path-or-ref> --strict --json` validates one
  workflow through the contract validator.
- `rzn-browser workflow validate-catalog --strict --json` validates the
  production capability surface.

## Tab And Session Policy

Shipped workflows should be parallel-safe by default.

- Prefer dedicated workflow tabs that reuse the operator's Chrome profile.
- Use `runtime.requires_existing_session: true` only when the workflow must
  continue an already-open browser state.
- Do not add `use_current_tab`, `use_active_tab`, or `current_tab_id` to
  production workflow JSON. Those are legacy/manual-debug concepts, not the
  authoring standard.

## Parameter Rules

Use the narrowest honest manifest type:

| Shape | Use |
|---|---|
| `string` | Search text, ids, URLs, handles, modes, filters, labels. |
| `integer` | Counts, limits, retry values, days. |
| `number` | Non-integer numeric values. |
| `boolean` | True binary toggles. Prefer string enums when more modes may appear. |
| `object` | Structured JSON bags. |
| `array` | Real lists, especially file lists. |

The CLI/native runner accepts array params as JSON arrays, comma-separated
strings, or a single value before manifest normalization. Do not mark scalar
choices such as `mode`, `filter`, `playlist_id`, or URLs as arrays.

## Side-Effect Rules

Declare what the workflow may do, then declare what each step actually does.

| Class | Meaning |
|---|---|
| `read_only` | Reads page or browser-visible content without mutation. |
| `external_read` | Reads data from a remote origin or third-party URL. |
| `network_access` | Performs outbound network access outside local browser state. |
| `browser_state` | Changes tab, DOM state, navigation, focus, or session-local browser state. |
| `file_write` | Writes local files. |
| `download` | Starts or records browser downloads. |
| `external_write` | Writes to a remote service or user account. |
| `auth` | Uses or changes authentication/session state. |
| `destructive` | Deletes, posts irreversible changes, or performs high-risk mutation. |

CLI post-processing is part of the policy surface:

- `--output-file` requires `file_write`.
- `--download-dir` requires `download`, `file_write`, `external_read`, and
  `network_access`.

## Handoff Gate

Before handing a workflow to another agent or team:

```bash
# 1. Validate the edited file.
rzn-browser workflow validate workflows/google/google-search.json --strict --json

# 2. Inspect the callable contract.
rzn-browser workflow inspect google search
rzn-browser workflow inspect google search --json

# 3. Validate the effective catalog.
rzn-browser workflow validate-catalog --strict --json

# 4. Run a safe smoke through the normal route.
rzn-browser run google search --param search_query="browser automation"
```

For mutating workflows, smoke only through a draft/review/approval gate unless
the operator explicitly approves the irreversible submit/send/post action.

## Discovering Available Workflows

The catalog changes more often than this README should. Use the CLI as the
source of truth:

```bash
rzn-browser list
rzn-browser list google
rzn-browser workflow capability list --json
```

For docs and examples for a specific pack, start with that pack's README under
`workflows/<system>/` and the durable docs under `docs/workflows/<system>/`.
