# Workflow Manifest Authoring Guide

This is the workflow format agents and workflow authors should target.
The manifest is both the discovery contract and the executable runtime contract.

## The Take

Do not author new production workflows in the old loose workflow shape. Use the
normal workflow filename, but make the file a manifest with typed inputs,
declared side effects, executable `steps[]`, and an output contract.

Example: Google search stays at `workflows/google/google-search.json`. It does
not become `google-search.manifest.json`.

That rule is deliberate: one production workflow gets one canonical file. Do
not add `*.manifest.json` or `*.manifest.v2.json` sidecars, and do not keep a
V1/V2 pair in the production catalog. Drafts can live outside the production
catalog while iterating, but promotion means replacing the normal workflow JSON
with a manifest-shaped body.

## Agent Instruction

Build workflows to this contract, in this order:

1. Find the nearest shipped workflow under `workflows/<system>/`.
2. Decide whether this is a new capability or a mode/param on an existing
   capability. Consolidate when the output shape and side-effect class match.
3. Edit the normal workflow JSON file. Do not create a manifest sidecar.
4. Declare all params in `params.properties` with the narrowest honest type.
5. Declare top-level and step-level side effects before running the workflow.
6. Add or update `result.output_selector` and `result.output_schema`.
7. Run `workflow inspect`; if inspect output is not enough for another agent to
   call the workflow, the workflow is not done.
8. Run strict validation, strict catalog validation, and a safe smoke.

## What A Manifest Owns

| Area | Manifest field | Notes |
|---|---|---|
| Identity | `id`, `system`, `capability`, `version` | `id` is the workflow implementation id. `capability` is the stable route. |
| Inputs | `params.properties` | Each parameter declares type, required/optional, default, enum, and sensitivity. |
| Side effects | `side_effects[]`, `steps[].action.side_effects[]` | Runtime policy blocks undeclared effects when enforced. |
| Runtime | `runtime` | Actor and timeout. `workflow_ref/path` are migration-only escape hatches. |
| Steps | `steps[]` | The runner can execute these directly. Prefer this over parallel legacy files. |
| Output | `result.output_selector`, `result.output_schema` | Defines what the workflow returns and the expected type shape. |
| Human help | `help` | Prose for humans. Not the ABI. |

## Input Contract

Declare every parameter used by placeholders or scripts.

```json
"params": {
  "properties": {
    "search_query": {
      "kind": "string",
      "required": true,
      "description": "Query text."
    },
    "vertical": {
      "kind": "string",
      "required": false,
      "description": "Search vertical.",
      "enum_values": ["web", "news", "books"],
      "default": "web"
    },
    "max_results": {
      "kind": "integer",
      "required": false,
      "default": 10,
      "min": 1,
      "max": 50
    }
  },
  "additional_params": false
}
```

Supported parameter kinds: `string`, `integer`, `number`, `boolean`, `object`,
and `array`.

Rules:

- Required means callers must provide it unless a default exists.
- Use `enum_values` for closed sets.
- Set `sensitive: true` for secrets, tokens, passwords, or session material.
- Keep `additional_params: false` unless the workflow intentionally accepts an
  open JSON bag.
- Use `array` only for real list values. The CLI/native runner accepts array
  params as JSON arrays, comma-separated strings, or a single value and
  normalizes them before validation.
- Do not mark scalar routing choices as arrays. `mode`, `filter`, ids, URLs,
  handles, and search queries are usually `string`.

## Step Contract

Each step wraps an engine action:

```json
{
  "id": "extract",
  "name": "Extract organic results",
  "action": {
    "kind": "extract_structured_data",
    "target": {
      "selector": ".result"
    },
    "inputs": {
      "fields": [
        { "name": "title", "selector": "h3" },
        { "name": "url", "selector": "a", "attribute": "href" }
      ]
    },
    "side_effects": ["read_only"]
  },
  "timeout_ms": 12000
}
```

Known action kinds map to engine step types. If a genuinely new step type is
needed, add it to the shared contract action enum and the migration script.
`custom` should be rare, not the default path.

## Side Effects

Declare side effects at both levels:

- Top-level `side_effects[]` is the workflow's allowed envelope.
- Step-level `action.side_effects[]` says what that step actually does.

Example:

```json
"side_effects": [
  { "class": "browser_state" },
  { "class": "external_write", "confirmation_required": true }
]
```

The supervisor compares observed step behavior against the declared contract
taxonomy. A submit/post/send workflow that omits `external_write` should fail
policy enforcement.

CLI post-processing is also part of the policy surface. If a workflow example
uses `--output-file`, the manifest must declare `file_write`. If it uses
`--download-dir`, the manifest must declare `download`, `file_write`,
`external_read`, and `network_access`.

## Output Contract

Always select the output step and describe the output shape:

```json
"result": {
  "output_selector": {
    "step_id": "extract",
    "path": "$"
  },
  "output_schema": {
    "type": "array",
    "items": {
      "type": "object",
      "properties": {
        "title": { "type": "string" },
        "url": { "type": "string" },
        "snippet": { "type": "string" }
      }
    }
  },
  "artifact_policy": {
    "prefer_downloads": false
  },
  "include_debug": false
}
```

The CLI and host-facing code unwrap `RunResultV2.output`, so callers should
reason about this output contract, not raw browser step responses.

## Inspection Command

Use this before wiring a workflow into an agent:

```bash
rzn-browser workflow inspect google search
```

For programmatic consumers:

```bash
rzn-browser workflow inspect google search --json
```

That output includes:

- required and optional inputs
- input types, defaults, enums, and sensitivity
- output selector and schema
- declared side effects
- runtime actor, timeout, and step count

Inspect output is the contract downstream agents read. If an agent would need
to open the workflow file to discover parameters, result shape, or mutation
risk, the manifest is not done.

## Migration Workflow

Mechanical migration:

```bash
python3 scripts/migrate_workflows_to_manifest.py --write --force
```

Then validate:

```bash
rzn-browser workflow validate-catalog --strict --json
rzn-browser workflow validate workflows/google/google-search.json --strict --json
rzn-browser workflow inspect google search --json
```

After that, smoke the workflow through the normal run path. For a pack-level
change, test one read-only workflow, one download-heavy workflow, one
authenticated workflow, and one mutating workflow for the changed pack. For
mutating workflows, stop at draft/review state unless the operator explicitly
approves the irreversible submit/send/post action.

Sidecar cleanup check:

```bash
find workflows -path 'workflows/fixtures' -prune -o -name '*.manifest*.json' -print
```

The production result should be empty. Fixture-only sidecars are allowed only
when a test explicitly needs them.

## Copy/Paste Acceptance Gate

Use this gate for every new workflow, migration, or downstream handoff:

```bash
# 1. The file is the canonical production path, not a sidecar.
test -f workflows/google/google-search.json

# 2. The JSON body declares the manifest schema.
rg -n '"schema_version"\\s*:\\s*"rzn.workflow_manifest"' workflows/google/google-search.json

# 3. The single workflow validates strictly.
rzn-browser workflow validate workflows/google/google-search.json --strict --json

# 4. The full production catalog validates strictly.
rzn-browser workflow validate-catalog --strict --json

# 5. Inspect exposes the agent contract.
rzn-browser workflow inspect google search
rzn-browser workflow inspect google search --json

# 6. Run a safe smoke through the normal route.
rzn-browser run google search --param search_query="browser automation"
```

For mutating workflows, use a safe route-specific smoke that proves navigation,
drafting, policy, and inspectability without performing an irreversible write.
Example shape:

```bash
rzn-browser workflow inspect x reply-post --json
rzn-browser workflow validate workflows/x/x_reply_post.json --strict --json
# Smoke only through draft/review. Do not submit unless explicitly approved.
```

## Review Checklist

| Check | Why it matters |
|---|---|
| `schema_version` is `rzn.workflow_manifest` | Ensures the strict Rust contract validator owns the file. |
| `steps[]` is populated | Avoids drift against a separate runtime workflow file. |
| Inputs are complete | Agents can call the workflow without reading step internals. |
| Required vs optional is accurate | Prevents false missing-param errors and silent bad defaults. |
| Side effects are declared | Policy enforcement can block unsafe calls. |
| Output selector points to a real step | Final output is not guessed from the last payload. |
| Output schema matches observed output | Hosts and downstream agents can type the result. |
| `workflow inspect --json` looks sane | This is the authoring and host inspection surface. |
| Strict catalog validation is green | The workflow is part of the production capability surface. |
| Safe smoke evidence exists | Validation proved shape; smoke proves the route can actually run. |
| No production sidecar exists | Prevents catalog drift and split ownership. |

## When To Rebuild Instead Of Migrate

Rebuild the workflow if the generated manifest has many opaque
`execute_javascript` blocks, unclear outputs, or missing parameter intent. The
generator is useful for bulk conversion, not for laundering weak workflow
design into a typed contract.
