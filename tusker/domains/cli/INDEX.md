---
schema: "tusker.domain/v6"
id: "cli"
title: "Cli"
status: "current"
owner: "sarav"
summary: "Command surface, flags, help text, routing, and user-visible terminal behavior."
required: false
knowledge_nodes:
  - "cli/canon"
source_of_truth:
  - "tusker/SKILL.md"
tags:
  - "cli"
---

# Cli

## Read this when

Read this when work touches cli behavior, implementation, policy, or current product knowledge.

## Do not read this when

Do not read this for unrelated domains or task proof history unless this index routes you there.

## Current canon

- [[cli/CANON]]

## Start here

Read [[cli/CANON]] first, then the narrowest reference node.

## Main knowledge nodes

- [[cli/CANON]]

## Source of truth

- `tusker/SKILL.md`

## Related domains

- [[codebase/INDEX]]

## Current work

<!-- tusker:current-work:begin -->
- [[LRT-T-0010]] - Harden extension bridge timeouts and MV3 wakeup recovery (active)
- [[WCP-T-0023]] - Make manifest canonical workflow contract and inspection surface (review)
- [[WCP-T-0025]] - Make strict catalog validation a real rollout gate (review)
- [[WCP-T-0028]] - Enforce declared side effects end to end (review)
- [[WCP-T-0029]] - Make RunResultV2 the only host-visible run envelope (review)
<!-- tusker:current-work:end -->
