# Contribution Shape

Use this reference when preparing a workflow or pack for the repo.

## Required Shape

Put files here:

- workflow JSON: `workflows/<system>/`
- system docs: `docs/workflows/<system>/`

Keep the contribution small and obvious:

- one system
- one concrete user outcome
- deterministic steps
- explicit params

## What To Include

Good workflow submissions usually include:

- one canonical workflow JSON filename
- a system `README.md` under `docs/workflows/<system>/`
- one markdown file per workflow
- a runnable example command
- parameter notes
- whether the flow is read-only, draft-only, or a real write

## What Not To Do

- Do not add site-specific hacks to shared engine code.
- Do not hide real write behavior.
- Do not submit generated debug probes as product workflows.
- Do not mix multiple unrelated user outcomes into one giant workflow.

## Submission Heuristic

A workflow pack is ready when a reviewer can answer these questions quickly:

1. What does it do?
2. What params does it need?
3. Does it write or only read?
4. How do I run it?
5. Why does this belong in this pack?
