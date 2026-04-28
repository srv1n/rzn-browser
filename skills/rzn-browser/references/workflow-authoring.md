# Workflow Authoring

Use this when the task is to create, edit, validate, or promote RZN workflow JSON.

## Hard Rules

- Inspect nearby workflows under `workflows/<system>/` before creating a new file.
- Consolidate before splitting. Add params, enums, or modes to an existing workflow when the output contract and side-effect class are the same.
- Keep shared runtime code generic. Site-specific selectors belong in workflow JSON or workflow-local docs.
- Every workflow needs `name`, `description`, `required_variables` when params are required, and a `help` block.
- Validate structurally and run end-to-end before calling the workflow done.
- For write actions, prefer draft/review modes or explicit approval gates.

## Build Paths

### Deterministic First

Use when the browser steps are obvious.

```bash
rzn-browser list <system>
rzn-browser workflow show <system> <nearby-workflow> --json
rzn-browser workflow validate workflows/<system>/<file>.json --write-help
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
4. Add or tighten `help`.
5. Validate and rerun:

```bash
rzn-browser workflow validate /path/to/generated.json --write-help
rzn-browser run /path/to/generated.json --param key="value"
```

6. Move it into `workflows/<system>/` only after it is stable.

## Help Block Shape

Every durable workflow should explain itself without forcing agents to open JSON:

```json
{
  "help": {
    "summary": "What this workflow does in one sentence.",
    "parameters": [
      {
        "name": "search_query",
        "required": true,
        "shape": "text",
        "description": "Query text.",
        "example": "browser automation"
      }
    ],
    "examples": [
      {
        "description": "Run the workflow with required parameters.",
        "command": "rzn-browser run google search --param search_query=\"browser automation\""
      }
    ],
    "returns": "High-signal output shape.",
    "notes": [
      "Auth, write behavior, current-tab behavior, and known limitations."
    ]
  }
}
```

## Validation Bar

Minimum:

```bash
rzn-browser workflow validate <path-or-id>
rzn-browser run <path-or-id> --param key="value"
```

If docs changed:

```bash
rzn-browser list <system> <workflow>
```

That command should show sane params, examples, returns, notes, and source path.

## Contribution Shape

When contributing back to the repo:

- workflow JSON: `workflows/<system>/<workflow>.json`
- pack README: `docs/workflows/<system>/README.md`
- per-workflow doc when the flow is non-trivial
- focused test, smoke run, or validation note
- explicit statement of read-only, draft-only, or real write behavior
