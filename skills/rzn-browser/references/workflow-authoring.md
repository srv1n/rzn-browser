# Workflow Authoring

Use this when the task is to create, edit, validate, or promote RZN workflow JSON.

## Hard Rules

- Inspect nearby workflows under `workflows/<system>/` before creating a new file.
- Consolidate before splitting. Add params, enums, or modes to an existing workflow when the output contract and side-effect class are the same.
- Keep shared runtime code generic. Site-specific selectors belong in workflow JSON or workflow-local docs.
- Every production workflow keeps the normal `workflows/<system>/<workflow>.json` filename and declares `schema_version: "rzn.workflow_manifest"` in the JSON body.
- Every parameter belongs in `params.properties`; every output belongs in `result.output_selector` and `result.output_schema`.
- Validate strictly, inspect the contract, and run end-to-end before calling the workflow done.
- For write actions, prefer draft/review modes or explicit approval gates.
- Do not create `*.manifest.json` or `*.manifest.v2.json` production sidecars.

## Build Paths

### Deterministic First

Use when the browser steps are obvious.

```bash
rzn-browser list <system>
rzn-browser workflow inspect <system> <nearby-workflow> --json
rzn-browser workflow validate workflows/<system>/<file>.json --strict --json
rzn-browser run workflows/<system>/<file>.json --param key="value"
```

Edit the JSON in small steps. Rerun after each meaningful change.

### Discovery First

Use when the site path is fuzzy.

```bash
rzn-browser llm-auto "Task to discover" --save-workflow true --name "<system>-<workflow-name>"
rzn-browser workflow dirs
```

Then:

1. Find the saved workflow in the user/generated catalog.
2. Remove unstable or redundant steps.
3. Replace vague steps with deterministic actions where possible.
4. Convert the draft into the manifest contract: typed `params`, declared
   `side_effects`, executable `steps[]`, `result`, and `help`.
5. Validate and rerun:

```bash
rzn-browser workflow validate /path/to/generated.json --strict --json
rzn-browser workflow inspect /path/to/generated.json --json
rzn-browser run /path/to/generated.json --param key="value"
```

6. Move it into `workflows/<system>/` only after it is stable.

## Manifest Contract Shape

Every durable workflow should expose its callable contract without forcing
agents to read step internals:

```json
{
  "schema_version": "rzn.workflow_manifest",
  "id": "google/search",
  "name": "Google Search",
  "version": "2.0.0",
  "system": "google",
  "capability": "google.search",
  "params": {
    "properties": {
      "search_query": {
        "kind": "string",
        "required": true,
        "description": "Query text."
      }
    },
    "additional_params": false
  },
  "side_effects": [
    { "class": "browser_state" },
    { "class": "read_only" }
  ],
  "steps": [],
  "result": {
    "output_selector": { "step_id": "extract", "path": "$" },
    "output_schema": { "type": "array" }
  },
  "help": {
    "summary": "What this workflow does in one sentence.",
    "parameters": {
      "search_query": "Query text."
    },
    "examples": [
      {
        "description": "Run the workflow with required parameters.",
        "command": "rzn-browser run google search --param search_query=\"browser automation\""
      }
    ],
    "returns": ["Array of search result rows."],
    "notes": [
      "Auth, write behavior, session requirements, and known limitations."
    ]
  }
}
```

## Validation Bar

Minimum:

```bash
rzn-browser workflow validate <path-or-id> --strict --json
rzn-browser workflow inspect <system> <workflow> --json
rzn-browser workflow validate-catalog --strict --json
rzn-browser run <path-or-id> --param key="value"
```

If docs changed:

```bash
rzn-browser list <system> <workflow>
```

That command should show sane params, examples, returns, notes, and source path.

Param type sanity:

- scalar choices and ids are `string`
- counters are `integer`
- toggles are `boolean`
- structured bags are `object`
- true lists are `array`

The CLI accepts array params as JSON arrays, comma-separated strings, or a
single value before manifest normalization.

## Contribution Shape

When contributing back to the repo:

- workflow JSON: `workflows/<system>/<workflow>.json`
- pack README: `docs/workflows/<system>/README.md`
- per-workflow doc when the flow is non-trivial
- focused test, smoke run, or validation note
- explicit statement of read-only, draft-only, or real write behavior
