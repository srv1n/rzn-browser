---
name: rzn-workflow-builder
description: Create, debug, or validate RZN Browser workflow JSON and its callable contract. Use for workflow authoring, not routine workflow execution.
---

# RZN Workflow Builder

Deliver a reusable workflow with discoverable inputs, declared side effects, and
an output contract that matches the observed result.

Inspect the nearest workflow in `workflows/<system>/`. Extend it when the outcome
and side-effect class match; create a separate workflow for a distinct capability.
Read [workflow authoring](references/workflow-authoring.md) for the manifest
contract and validation commands.

Edit deterministic steps directly when the path is known. Use bounded `llm-auto`
only when page exploration is needed and provider use is in scope. The current
CLI does not export a workflow; translate observations into JSON explicitly.

## Boundaries

- Keep site selectors and DOM logic in workflow files; shared runtime stays generic.
- Use dedicated tabs in the existing browser profile unless exact existing page
  state is needed. Declare that requirement in the manifest.
- Describe write behavior accurately. Test within the user's authorization;
  otherwise validate locally and exercise the draft path up to its approval gate.
- Source edits and offline validation do not require installing the runtime.
  Read [troubleshooting](references/troubleshooting.md) for runtime failures.
  Install only when repair or setup is part of the requested task.

## Completion

Validate the changed file, inspect its contract, and run an authorized browser
check when runtime behavior changed. Fix failures caused by the change and rerun
affected checks. Finish when the requested outcome and checks pass, or report the
specific missing runtime, access, or approval; do not call an unrun flow verified.

Read [contribution shape](references/contribution-shape.md) when preparing a pack
for review. Add documentation or tests where the changed behavior needs them.
