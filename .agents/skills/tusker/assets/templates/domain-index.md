---
schema: tusker.domain/v7
kind: domain
id: "{{domain}}"
project: "{{project}}"
title: "{{title}}"
status: draft
summary: "{{summary}}"
capsule:
  what: ""
  use_when: ""
  skip_when: ""
source_of_truth:
  - tusker/knowledge/domains/{{domain}}/CANON.md
canonical_files:
  - INDEX.md
  - CANON.md
created_at: "{{date}}"
updated_at: "{{date}}"
state_rev: 1
---

# {{title}}

## Read this when

Use this domain index to route work for {{title}}.

## Do not read this when

Skip this when another domain is narrower.

## Current canon

- [[CANON]]

## Main knowledge nodes

- `CANON.md` — current model, invariants, defaults, deprecated behavior.
- `runbooks/` — operational procedures.
- `decisions/` — accepted or pending durable decisions.
- `interfaces/` — contracts and boundaries.
- `invariants/` — rules that should not drift.
- `sources/` — raw or external source material.
- `glossary.md` — terms of art.

## Current work

<!-- tusker:current-work:begin -->
_Run `tusker reindex` to refresh current work._
<!-- tusker:current-work:end -->
