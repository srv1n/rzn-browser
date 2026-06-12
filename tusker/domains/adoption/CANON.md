---
schema: "tusker.knowledge/v6"
node: "adoption/canon"
title: "Adoption canon"
domain: "adoption"
kind: "canon"
audience: "developer"
agent_layer: "capsule"
canonical_status: "draft"
summary: "Install, migration, rollout, and consumer repo adoption."
aliases:
  - "adoption canon"
  - "adoption"
source_of_truth:
  - "tusker/SKILL.md"
stale_when:
  paths:
    - "tusker/SKILL.md"
publish:
  include_in_llms: true
  lane: "internal"
  path: "adoption/canon"
created_at: "2026-05-12"
updated_at: "2026-05-12"
tags:
  - "adoption"
---

# Adoption canon

## Read this when

Read this for the current model, invariants, defaults, and boundaries for adoption.

## Do not read this when

Do not use this as task proof; open linked tasks only when implementation history or evidence matters.

## Current model

This domain records current durable truth for adoption.

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

- [[adoption/INDEX]]

## Recent changes

<!-- tusker:backrefs:begin -->
_No task proof recorded yet._
<!-- tusker:backrefs:end -->
