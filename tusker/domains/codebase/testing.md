---
schema: "tusker.knowledge/v6"
node: "codebase/testing"
title: "Testing"
domain: "codebase"
kind: "reference"
audience: "developer"
agent_layer: "capsule"
canonical_status: "draft"
summary: "Testing and verification map for Rust, extension, workflow, runtime, and docs changes."
aliases:
  - "testing"
source_of_truth:
  - "AGENTS.md"
  - "README.md"
  - "Makefile"
  - "extension/package.json"
  - "docs/workflows/AGENT_PLAYBOOK.md"
stale_when:
  paths:
    - "AGENTS.md"
    - "README.md"
    - "Makefile"
    - "extension/package.json"
    - "docs/workflows/AGENT_PLAYBOOK.md"
publish:
  include_in_llms: true
  lane: "internal"
  path: "codebase/testing"
created_at: "2026-05-12"
updated_at: "2026-05-13"
---

# Testing

## Read this when

Read this when picking the right verification command for a change.

## Do not read this when

Do not use this as proof that a command was run. Check the task verification log.

## Verification map

| Change | Verification |
|---|---|
| Rust crate behavior | `cargo test -p <crate>` or focused test target. |
| Whole Rust workspace | `cargo test` or `make test`. |
| Full build | `make build`. |
| Extension source | `cd extension && bun run build && bun x vitest`. |
| Extension e2e | `cd extension && bun x playwright test` for repo-owned e2e only. |
| Workflow manifest | `rzn-browser workflow validate <path-or-ref> --strict --json`; inspect; catalog validate. |
| Workflow live behavior | `rzn-browser run <system> <workflow> --param ...` in the existing Chrome profile. |
| Tusker docs | `tusker validate` when CLI is available; otherwise record the blocker and run structural text checks. |

Structural validation is necessary but not enough for workflow work. Live smoke is the bar unless the remaining risk is explicitly tracked.

## Source of truth

- `AGENTS.md`
- `README.md`
- `Makefile`
- `extension/package.json`
- `docs/workflows/AGENT_PLAYBOOK.md`

## Related

- [[codebase/CANON]]
- [[workflow/CANON]]

## Recent changes

<!-- tusker:backrefs:begin -->
- [[OPS-T-0002]] - Replaced generated scaffold with testing reference.
<!-- tusker:backrefs:end -->
