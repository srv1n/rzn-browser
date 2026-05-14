---
schema: tusker.knowledge/v6
title: Template contract
node: schema/templates
audience: developer
agent_layer: capsule
kind: reference
domains:
- schema
source_of_truth:
- tusker/_system/templates
- tusker/_config/knowledge-policy.yaml
canonical_status: draft
created: '2026-05-08'
updated: '2026-05-13'
domain: schema
stale_when:
  paths:
  - tusker/_system/templates/**
  - tusker/_config/knowledge-policy.yaml
publish:
  lane: internal
  path: schema/templates
  include_in_llms: true
summary: Tusker V6 template files and required section contract.
---

# Template contract

## Read this when

Read this before adding or changing Tusker templates.

## Do not read this when

Do not use this as exact YAML schema; open the template file and policy file.

## Contract

Templates under `tusker/_system/templates/**` define the shape new tasks, epics, domains, and knowledge pages should start from. The policy file defines which sections validation expects.

Required knowledge sections:

- `Read this when`
- `Do not read this when`
- `Source of truth`
- `Related`

Required domain index sections:

- `Read this when`
- `Do not read this when`
- `Current canon`
- `Start here`
- `Main knowledge nodes`
- `Source of truth`
- `Related domains`
- `Current work`

Template edits should preserve human-readable markdown first. Do not add fields agents cannot keep current.

## Source of truth

- `tusker/_system/templates/**`
- `tusker/_config/knowledge-policy.yaml`

## Related

- [[schema/CANON]]

## Recent changes

<!-- tusker:backrefs:begin -->
- [[OPS-T-0002]] - Replaced generated scaffold with template contract reference.
<!-- tusker:backrefs:end -->
