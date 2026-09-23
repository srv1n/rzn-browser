---
name: rzn-browser
description: Run RZN Browser workflows in the user's existing Chrome session. Use for browser tasks that need rendered pages or signed-in state.
---

# RZN Browser

Complete the requested browser task through the installed `rzn-browser` CLI,
extension, and native host. Prefer an available API for tasks that do not need
browser state.

## Choose a route

| Task | Route |
| --- | --- |
| Run a known workflow | Inspect its help, then run it with the requested parameters. |
| Find a workflow | `rzn-browser workflow list <system>`; list everything only if the system is unknown. |
| No suitable workflow | Consider bounded `llm-auto` with a configured provider. |
| Create or repair workflow JSON | Read [workflow authoring](references/workflow-authoring.md). |
| Connection or installation failure | Read [runtime troubleshooting](references/runtime-troubleshooting.md). |

```bash
rzn-browser workflow list google search
rzn-browser run google search --param search_query="browser automation"
```

Use [CLI reference](references/cli-cheatsheet.md) for output files, tab selection,
catalog imports, or autonomous-mode flags. Read only what the task needs.

## Execution boundaries

- Reuse the user's Chrome profile. Dedicated workflow tabs preserve its login;
  use an existing tab when the task needs that exact page state.
- A request to send, submit, purchase, or delete authorizes only the specified
  action and target. Prepare and inspect the result first; ask only for missing
  scope or approval not already supplied. Honor workflow gates.
- `llm-auto` can act on the page and call a paid provider. Include the user's
  constraints in its instruction and bound the steps. Dummy mode is a local
  smoke check, not proof of real discovery or provider behavior.
- Inspect an ambiguous write before retrying; it may already have taken effect.
  Diagnose a broken bridge without silently switching profiles.

## Done

Check returned data or the visible postcondition against the user's request.
Report the useful result, source URLs, and missing coverage. A zero exit code
alone does not establish successful extraction or submission. If blocked, give
the exact failure and next action; distinguish local validation from live results.
