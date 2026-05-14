---
schema: "tusker.knowledge/v6"
node: "docs-system/canon"
title: "Docs System canon"
domain: "docs-system"
kind: "canon"
audience: "developer"
agent_layer: "capsule"
canonical_status: "draft"
summary: "Current durable truth for docs system."
aliases:
  - "docs-system canon"
  - "docs system"
source_of_truth:
  - "tusker/SKILL.md"
stale_when:
  paths:
    - "tusker/SKILL.md"
publish:
  include_in_llms: true
  lane: "internal"
  path: "docs-system/canon"
created_at: "2026-05-12"
updated_at: "2026-05-12"
tags:
  - "docs-system"
---

# Docs System canon

## Read this when

Read this for the current model, invariants, defaults, and boundaries for docs system.

## Do not read this when

Do not use this as task proof; open linked tasks only when implementation history or evidence matters.

## Current model

This domain records current durable truth for docs system.

## Invariants

- Keep current truth in domain knowledge pages.
- Keep task proof in `tusker/epics/**`.
- Prefer source code over prose when exact behavior conflicts.

## Current defaults

- New knowledge starts as draft canon until verified.
- Route through this canon before opening historical tasks.

## Deprecated behavior

- Do not treat task files as canonical documentation.

## Source of truth

- `tusker/SKILL.md`

## Open questions

- Add domain-specific open questions here as the implementation matures.

## Related

- [[docs-system/INDEX]]

## Recent changes

<!-- tusker:backrefs:begin -->
_No task proof recorded yet._
<!-- tusker:backrefs:end -->
