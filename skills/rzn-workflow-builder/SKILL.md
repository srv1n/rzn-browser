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
- Move like a person. The extension already turns a same-site
  `navigate_to_url` into an in-page navigation and fails a page that shows a bot
  check with `site_challenge`. Click links when the flow follows one, scroll and
  pause before extracting, and never fetch extra pages in the background. The
  CLI queues runs per site and reuses one tab.
- Keep timers out of page JavaScript; background tabs throttle them. Use
  `wait_for_timeout` steps for pauses and a `MutationObserver` for waits.
- Describe write behavior accurately. Test within the user's authorization;
  otherwise validate locally and exercise the draft path up to its approval gate.
- Source edits and offline validation do not require installing the runtime.
  Read [troubleshooting](references/troubleshooting.md) for runtime failures.
  Install only when repair or setup is part of the requested task.

## Repairing a workflow

A fix to an installed workflow is a repair candidate for the fleet catalog, not a
private override. A local copy silently outranks the catalog forever and later
fixes never reach it.

1. Keep the edit as a user fork: copy the installed manifest into
   `~/Library/Application Support/RZN/workflows/user/<system>/` (same `id`) and
   edit only the copy.
2. Record what broke from the failed run's `failure_summary`: `error_class`,
   `fingerprint`, `failing_step_index` and `message` (in `rzn-browser run … --json`).
3. Run the contract lint, `rzn-browser workflow validate <fork> --strict --json`,
   then a live run of the fork, `rzn-browser run <fork> --param …`. An item
   workflow's last run must succeed: its output is checked against the contract.
4. Submit it: `rzn-browser fleet propose <id> --reason "<error_class> at step
   <failing_step_index> (<fingerprint>): <what broke>; <what changed>"`. It sends
   the fork, the hash it was made from, the reason and the last run; offline it
   queues and the supervisor sends it after its next check-in.

Once the catalog releases it, the next catalog pull moves the fork to
`workflows/trash/`. `rzn-browser fleet forks` lists every remaining fork with
its diff and propose command.

## Completion

Validate the changed file, inspect its contract, and run an authorized browser
check when runtime behavior changed. Fix failures caused by the change and rerun
affected checks. Finish when the requested outcome and checks pass, or report the
specific missing runtime, access, or approval; do not call an unrun flow verified.

Read [contribution shape](references/contribution-shape.md) when preparing a pack
for review. Add documentation or tests where the changed behavior needs them.
