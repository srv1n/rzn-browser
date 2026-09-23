# Workflow authoring

Use this only when creating, editing, or promoting workflow JSON. Routine workflow
execution does not need an authoring loop.

Inspect the nearest workflow and its callable contract. Extend it when the
outcome and side-effect class match. Keep site-specific selectors and extraction
in workflow data, rather than shared engine code.

## Manifest essentials

- One production file: `workflows/<system>/<workflow>.json`, with
  `schema_version: "rzn.workflow_manifest"`; no manifest sidecars.
- Declare typed inputs in `params.properties`, executable `steps`, honest
  `side_effects`, and the result selector/schema in `result`.
- Keep `help` consistent with actual parameters, output, auth, and write behavior.
- Prefer dedicated tabs in the user's existing profile. Declare
  `runtime.requires_existing_session: true` only for exact existing page state.
- For externally mutating flows, preserve required confirmation and draft modes.
  Existing user authorization covers only the specified action and target.

## Check the changed workflow

```bash
rzn-browser workflow validate /path/to/workflow.json --strict --json
rzn-browser workflow inspect /path/to/workflow.json --json
rzn-browser run /path/to/workflow.json --param key="value"
```

Validation and inspection are offline; run only with the necessary browser access
and authorization. Fix failures introduced by the change and rerun affected
checks. Validate the full catalog when capability routing or shared manifest
behavior changes, not for every local edit.

For uncertain page paths, bounded `llm-auto` can support authorized exploration
with a configured provider. The CLI does not export workflow JSON; author it from
observations and validate it independently. Dummy runs are not discovery evidence.

A finished workflow has a usable inspected contract and evidence for its changed
behavior. Report any untested live path explicitly. Update pack documentation
when usage changes; a separate document per workflow is not required.
