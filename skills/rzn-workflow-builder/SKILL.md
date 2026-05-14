---
name: rzn-workflow-builder
description: Build, debug, validate, and contribute RZN Browser workflow JSONs using the local rzn-browser runtime, extension, native host, and llm-auto workflow-factory loop. Use when Codex or another coding agent needs to bootstrap or repair local RZN setup, create or refine workflows under workflows/system-name/ or workflows/generated/, discover a workflow with llm-auto and turn it into deterministic JSON, validate flows against a live Chrome session, or prepare workflow docs for submission.
---

# RZN Workflow Builder

Start every workflow task by making sure the local runtime is actually usable:

```bash
./skills/rzn-workflow-builder/scripts/ensure-runtime.sh
```

If the runtime is already healthy, the script exits quickly. If not, it runs the repo install flow and then the doctor check.

## Core Workflow

1. Confirm the target site, desired outcome, and whether the flow is read-only, draft-only, or a real write.
2. Inspect nearby workflow JSONs under `workflows/<system>/` before inventing a new pattern.
3. Choose one build path:
   - Deterministic first: clone and edit an existing workflow JSON when the steps are already clear.
   - Discovery first: use `llm-auto --save-workflow` when the site path is fuzzy, then clean the saved JSON into a deterministic workflow.
4. Validate the workflow locally with `rzn-browser run ...` until it is stable.
5. Inspect and validate the manifest contract with `workflow inspect` and
   `workflow validate --strict`.
6. Keep site-specific selectors and DOM logic inside the workflow JSON or workflow-local docs, not in shared engine code.
7. If the workflow is worth keeping, add docs under `docs/workflows/<system>/` and keep the workflow filename canonical.

Read [references/workflow-authoring.md](references/workflow-authoring.md) before editing or generating a workflow.

## Build Paths

### Path A: Edit an Existing Workflow

Use this when there is already a nearby pack for the same site or user outcome.

- Copy the closest workflow in `workflows/<system>/`.
- Keep the workflow in the same system folder if it belongs to the existing pack.
- Run it through `rzn-browser run <system> <workflow> --param ...` or by explicit file path while iterating.
- Make the smallest possible JSON change between runs.

### Path B: Discover With `llm-auto`

Use this when the target site path is not yet deterministic.

```bash
./skills/rzn-workflow-builder/scripts/discover-workflow.sh "Search Google for browser automation and extract the top results"
```

The discovery helper writes saved flows into `workflows/generated/` by default. After it produces something usable:

1. open the generated JSON
2. remove junk or unstable steps
3. replace vague steps with deterministic ones where possible
4. rerun the cleaned workflow directly
5. move or rename it into the right pack only after it is stable

## Workflow Rules

- Prefer built-in workflow ids like `google search` once the flow is good enough to reuse.
- Keep the normal workflow filename and put the manifest contract in the JSON body.
- Use `runtime.requires_existing_session: true` only when the flow truly needs an already-open browser session.
- Keep risky write flows explicit. For send/post/reply/submit actions, prefer `request_user_intervention` with a real approval gate.
- Keep shared runtime code generic. Site-specific hacks belong in workflow data and workflow docs.
- Do not hide real write behavior. Call out whether the flow mutates state.

Read [references/workflow-authoring.md](references/workflow-authoring.md) for concrete patterns including manifest params, `request_user_intervention`, and local validation commands.

## Contribution Shape

When the user wants a workflow contributed back to the repo:

1. Add the workflow JSON under `workflows/<system>/`.
2. Add or update `docs/workflows/<system>/README.md`.
3. Add one markdown doc per workflow under `docs/workflows/<system>/`.
4. Include required params, output shape, and one runnable example.
5. Add a parse test, smoke flow, or validation note when the change is non-trivial.

Read [references/contribution-shape.md](references/contribution-shape.md) when preparing a workflow pack for upstream use.

## Troubleshooting

If anything smells off, read [references/troubleshooting.md](references/troubleshooting.md).

Use that reference when:

- `rzn-browser` is missing
- the native host is not connected
- Chrome is open but the extension is stale
- `llm-auto` should avoid real provider billing
- the workflow should use a dedicated tab or requires an existing session
- the runtime attaches but the flow still fails
