---
title: "Documentation guide"
subject: documentation
keywords: [docs, writing, ASD-STE100]
part_of: overview
kind: doc
lifecycle: current
code_conformance: unverified
read_when: "You need to write or update project documentation."
skip_when: "You need product behavior. Read the owning source page."
---

# Documentation guide

Keep one owner for each rule:

| Page | Owns |
| --- | --- |
| `README.md` | Product entry point. |
| `CONTRIBUTING.md` | Source changes and review. |
| `workflows/README.md` | Workflow catalog contract. |
| `docs/system/` | Current system rules. |
| `workflows/<system>/README.md` | Pack-specific guidance. |

Do not create a second page that repeats a rule. Update the owning page and
link to it.

## Page format

Each `docs/system/` page carries frontmatter that agents use for routing:

| Field | Use |
| --- | --- |
| `title`, `subject` | Page identity. `subject` is the stable lookup key. |
| `keywords` | Discovery terms for `tusker docs find`. |
| `part_of` | Parent subject; forms the docs graph. |
| `read_when` / `skip_when` | Route a reader in or out of the page. |
| `kind` | `doc`, `proposal`, or `decision`. |
| `lifecycle` | `current` or `superseded` for docs. |
| `code_conformance` | `unverified`, `matches`, `drift`, or `not_applicable`. Mark `matches` only after checking behavior, with `last_verified` and a commit. |

Keep metadata out of the reading prose.

## ASD-STE100 style

- Use one idea per sentence.
- Use short sentences and active voice.
- Use one term for one thing.
- Define an abbreviation before you use it.
- Give a command when a command is required.
- State limits and failure cases.
- Avoid marketing claims and vague words.
- Do not claim source, install, runtime, or human proof from another proof.
- Put the source path near a claim that can change.

Use headings, tables, Mermaid diagrams, and small code blocks. Do not copy
large source blocks.

## Change process

Search the current docs before you add a page. Check the source caller and
composition root for every behavior claim. Keep generated indexes and graphs as
generated outputs. Do not edit `docs/system/INDEX.md` or
`docs/system/graph.json` by hand. Regenerate them with `tusker docs map` on a
checkout that has the Tusker vault, and check edits with
`tusker docs check <subject-or-path>`.

When a code change changes a contract, update the owning page in the same
change. Remove a stale page instead of keeping two competing instructions.
