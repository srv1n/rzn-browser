---
title: "Documentation guide"
subject: documentation
keywords: [docs, writing, ASD-STE100]
part_of: overview
read_when: "You need to write or update project documentation."
skip_when: "You need product behavior. Read the owning source page."
---

# Documentation guide

Keep one owner for each rule:

- `README.md` is the product entry point.
- `CONTRIBUTING.md` is for source changes and review.
- `workflows/README.md` is the workflow catalog contract.
- `docs/system/` contains current system rules.
- `workflows/<system>/README.md` contains pack-specific guidance.

Do not create a second page that repeats a rule. Update the owning page and
link to it.

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

Use headings, tables, and small code blocks. Do not copy large source blocks.

## Change process

Search the current docs before you add a page. Check the source caller and
composition root for every behavior claim. Keep generated indexes and graphs as
generated outputs. Do not edit `docs/system/INDEX.md` or `docs/system/graph.json`
by hand.

When a code change changes a contract, update the owning page in the same
change. Remove a stale page instead of keeping two competing instructions.
