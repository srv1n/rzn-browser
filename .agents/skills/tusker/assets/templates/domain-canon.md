---
schema: tusker.domain-canon/v7
kind: domain_canon
id: "{{domain}}/canon"
project: "{{project}}"
domain: "{{domain}}"
title: "{{title}} canon"
status: draft
summary: "{{summary}}"
capsule:
  what: ""
  use_when: ""
  skip_when: ""
source_of_truth:
  - tusker/knowledge/domains/{{domain}}/INDEX.md
created_at: "{{date}}"
updated_at: "{{date}}"
state_rev: 1
---

# {{title}} canon

## Read this when

Read this for the current {{title}} model.

## Do not read this when

Do not use this as task proof. Task proof lives in tasks, gates, evidence, attempts, and review packets.

## Current model

Current model goes here.

## Invariants

- Keep current truth here.

## Current defaults

- Defaults go here.

## Deprecated behavior

- Deprecated behavior goes here.

## Source of truth

- `tusker/knowledge/domains/{{domain}}/INDEX.md`

## Open questions

- None yet.

## Related

- [[INDEX]]

## Recent changes

<!-- tusker:backrefs:begin -->
_Run `tusker reindex` to refresh recent task proof._
<!-- tusker:backrefs:end -->
