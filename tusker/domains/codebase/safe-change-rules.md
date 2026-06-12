---
schema: "tusker.knowledge/v6"
node: "codebase/safe-change-rules"
title: "Safe change rules"
domain: "codebase"
kind: "reference"
audience: "developer"
agent_layer: "capsule"
canonical_status: "draft"
summary: "Repo-specific safety rules for edits, git, browser automation, workflows, docs, and generated files."
aliases:
  - "safe change rules"
source_of_truth:
  - "AGENTS.md"
  - "tusker/SKILL.md"
stale_when:
  paths:
    - "AGENTS.md"
    - "tusker/SKILL.md"
publish:
  include_in_llms: true
  lane: "internal"
  path: "codebase/safe-change-rules"
created_at: "2026-05-12"
updated_at: "2026-05-13"
---

# Safe change rules

## Read this when

Read this before making edits that could affect repo behavior, workflow packs, browser automation, docs, or git state.

## Do not read this when

Do not use this to override direct human instructions or deeper `AGENTS.md` files.

## Rules

- Work on `main` unless the human asks for a branch.
- Do not run mutating git operations unless the human asks.
- Do not revert changes you did not make.
- Check for nested `AGENTS.md` before editing scoped directories.
- For browser automation, use the existing Chrome extension/native-host path unless the task is explicitly a Playwright test.
- Do not add site-specific selectors to generic runtime code.
- For workflows, probe the live DOM and run end-to-end before declaring success.
- Do not edit `tusker/_system/generated/**`.
- Do not author in generated docs-site output.
- Keep feature scratchpads current under `docs/features/<feature>/README.md`.

## Source of truth

- `AGENTS.md`
- `tusker/SKILL.md`

## Related

- [[codebase/CANON]]
- [[workflow/CANON]]
- [[runtime/CANON]]

## Recent changes

<!-- tusker:backrefs:begin -->
_No task proof recorded yet._
<!-- tusker:backrefs:end -->
