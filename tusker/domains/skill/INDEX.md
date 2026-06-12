---
schema: "tusker.domain/v6"
id: "skill"
title: "Skill"
status: "current"
owner: "sarav"
summary: "Project knowledge routing, repo agent guidance, Tusker workflow, bundled skills, workflow-builder guidance, and skill installer behavior."
required: false
knowledge_nodes:
  - "skill/canon"
source_of_truth:
  - "AGENTS.md"
  - "CLAUDE.md"
  - "tusker/SKILL.md"
  - "skills/**/SKILL.md"
  - "crates/rzn_browser/src/skill_installer.rs"
tags:
  - "skill"
---

# Skill

## Read this when

Read this when work touches project knowledge routing, agent instructions, Tusker task workflow, bundled skills, or skill install/update behavior.

## Do not read this when

Do not read this for unrelated domains or task proof history unless this index routes you there.

## Current canon

- [[skill/CANON]]

## Start here

Read [[skill/CANON]] first, then the specific skill file or installer code being changed.

## Main knowledge nodes

- [[skill/CANON]]
- [[skill/skill]]

## Source of truth

- `AGENTS.md`
- `CLAUDE.md`
- `tusker/SKILL.md`
- `skills/**/SKILL.md`
- `crates/rzn_browser/src/skill_installer.rs`

## Related domains

- [[codebase/INDEX]]

## Current work

<!-- tusker:current-work:begin -->
- [[OPS-T-0002]] - Replace generated V6 domain canon with repo-specific documentation (review)
- [[WCP-T-0031]] - Publish workflow authoring gate for downstream agents (review)
<!-- tusker:current-work:end -->
