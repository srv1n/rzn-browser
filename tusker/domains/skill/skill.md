---
schema: tusker.knowledge/v6
title: Skill and AGENTS guidance
node: skill/skill
audience: developer
agent_layer: capsule
kind: reference
domains:
- skill
source_of_truth:
- AGENTS.md
- CLAUDE.md
- skills/rzn-browser/SKILL.md
- skills/rzn-workflow-builder/SKILL.md
- crates/rzn_browser/src/skill_installer.rs
canonical_status: draft
created: '2026-05-08'
updated: '2026-05-13'
domain: skill
stale_when:
  paths:
  - AGENTS.md
  - CLAUDE.md
  - skills/**/SKILL.md
  - crates/rzn_browser/src/skill_installer.rs
publish:
  lane: internal
  path: skill/skill
  include_in_llms: true
summary: Concrete source map for repo guidance and bundled agent skills.
---

# Skill and AGENTS guidance

## Read this when

Read this when changing root agent instructions, project knowledge routing, bundled skill files, or skill installer behavior.

## Do not read this when

Do not use this for runtime or workflow ABI details; follow the related canon pages.

## Source map

| Source | Owns |
|---|---|
| `AGENTS.md` | Repo-wide agent/contributor rules, browser guardrails, feature scratchpad requirements, build/test commands. |
| `CLAUDE.md` | Claude-facing repo guidance. Keep tracker guidance aligned with `AGENTS.md`. |
| `tusker/SKILL.md` | Project knowledge router and Tusker workflow reminders. |
| `skills/rzn-browser/SKILL.md` | Broad product skill for agents using RZN Browser. |
| `skills/rzn-workflow-builder/SKILL.md` | Narrow workflow-authoring skill. |
| `crates/rzn_browser/src/skill_installer.rs` | CLI install/update/remove/link implementation. |

## Editing rules

- Update docs and skill text from actual CLI/code behavior.
- Keep install examples aligned with `rzn-browser skill` commands.
- Do not duplicate long workflow trap lists in multiple places; point to `docs/workflows/AGENT_PLAYBOOK.md`.
- Treat skill changes as behavior changes for future agents.

## Source of truth

- `AGENTS.md`
- `CLAUDE.md`
- `skills/rzn-browser/SKILL.md`
- `skills/rzn-workflow-builder/SKILL.md`
- `crates/rzn_browser/src/skill_installer.rs`

## Related

- [[skill/CANON]]
- [[workflow/CANON]]
- [[runtime/CANON]]

## Recent changes

<!-- tusker:backrefs:begin -->
_No task proof recorded yet._
<!-- tusker:backrefs:end -->
