---
schema: "tusker.knowledge/v6"
node: "{{domain}}/canon"
title: "{{title}} canon"
domain: "{{domain}}"
kind: "canon"
audience: "developer"
agent_layer: "capsule"
canonical_status: "draft"
summary: "{{summary}}"
source_of_truth:
  - "tusker/SKILL.md"
stale_when:
  paths:
    - "tusker/SKILL.md"
publish:
  lane: "internal"
  path: "{{domain}}/canon"
  include_in_llms: true
created_at: "{{date}}"
updated_at: "{{date}}"
---

# {{title}} canon

## Read this when

Read this for the current {{title}} model.

## Do not read this when

Do not use this as task proof.

## Current model

Current model goes here.

## Invariants

- Keep current truth here.

## Current defaults

- Defaults go here.

## Deprecated behavior

- Deprecated behavior goes here.

## Source of truth

- `tusker/SKILL.md`

## Open questions

- None yet.

## Related

- [[{{domain}}/INDEX]]

## Recent changes

<!-- tusker:backrefs:begin -->
_Run `tusker reindex` to refresh recent task proof._
<!-- tusker:backrefs:end -->
