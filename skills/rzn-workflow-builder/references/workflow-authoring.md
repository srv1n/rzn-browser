# Workflow authoring

## Contract

Production workflows live in `workflows/<system>/<workflow>.json`. Keep one file
per capability and declare `schema_version: "rzn.workflow_manifest"` in its body.
Use the closest maintained workflow as the starting point, not a copied schema
skeleton or a new sidecar.

```bash
rzn-browser workflow list google
rzn-browser workflow inspect google/search --json
```

The inspected contract must describe inputs, output, effects, and runtime without
requiring callers to read step internals.

| Field | Requirement |
| --- | --- |
| `id`, `system`, `capability` | Identify the intended catalog route. |
| `params.properties` | Declare required/optional inputs, types, defaults, enums, and sensitivity as applicable. |
| `side_effects` | Declare actual browser, network, auth, file, and external write behavior. |
| `steps` | Executable actions with site-specific logic kept here. |
| `result` | Select the actual output step/path and declare its schema. |
| `help` | Accurate parameters, runnable examples, returns, and limitations. |

Use strings for IDs, integers for counts, booleans for toggles, and objects/arrays
for structured values. The CLI normalizes array params from JSON, comma-separated
text, or a single value.

## Browser state and writes

Prefer a dedicated tab in the existing profile. Use
`runtime.requires_existing_session: true` only when the flow needs exact existing
page state. Do not add legacy active-tab fields to production JSON.

For a real write, make the draft and final action distinguishable, verify the
target/control before acting, and preserve required `request_user_intervention`
gates. Exercise the final action only when the user has authorized it. If the
outcome is ambiguous, inspect state before retrying.

## Discovery when needed

```bash
rzn-browser llm-auto "Inspect the target page and describe the extraction path; do not submit forms" --url "https://example.com" --max-steps 12 --json
```

Use a configured provider within the task's scope. The current CLI does not
support `--save-workflow`; observations must be translated into deterministic
JSON. Dummy mode is for local smoke checks, not real site discovery.

## Validation and completion

```bash
rzn-browser workflow validate /path/to/workflow.json --strict --json
rzn-browser workflow inspect /path/to/workflow.json --json
rzn-browser run /path/to/workflow.json --param key="value"
```

Validate and inspect before a browser run. A source-only change can proceed
without installation or provider calls. Compare actual output to the declared
schema and requested outcome; strict validation alone does not establish it.

Fix failures caused by the change and rerun the affected checks. Run
`rzn-browser workflow validate-catalog --strict --json` when catalog routing or
shared contract behavior changes. Use focused tests for changed parsing or
control flow; do not rerun unrelated suites for a documentation edit.

Promote a draft when its inputs and effects are explicit, its steps are reusable,
and its changed behavior has an authorized runtime check. If access or approval
is missing, finish the offline work and report the precise unverified path.
