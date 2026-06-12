---
schema: "tusker.knowledge/v6"
node: "{{node}}"
title: "{{title}}"
domain: "{{domain}}"
kind: "reference"
audience: "developer"
agent_layer: "capsule"
canonical_status: "draft"
summary: "{{summary}}"
aliases: []
source_of_truth:
  - "tusker/SKILL.md"
stale_when:
  paths:
    - "tusker/SKILL.md"
related_nodes: []
related_epics: []
publish:
  lane: "internal"
  path: "{{node}}"
  include_in_llms: true
created_at: "{{date}}"
updated_at: "{{date}}"
---

# {{title}}

## Read this when

Read this when this exact knowledge node answers the user's intent.

## Do not read this when

Do not read this for unrelated domains or historical proof.

## Source of truth

- `tusker/SKILL.md`

## Related

- [[{{domain}}/CANON]]

## Recent changes

<!-- tusker:backrefs:begin -->
_Run `tusker reindex` to refresh recent task proof._
<!-- tusker:backrefs:end -->
