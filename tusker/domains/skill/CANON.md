---
schema: "tusker.knowledge/v6"
node: "skill/canon"
title: "Skill canon"
domain: "skill"
kind: "canon"
audience: "developer"
agent_layer: "capsule"
canonical_status: "draft"
summary: "Agent skill and repo guidance canon: project knowledge routing, Tusker workflow, bundled RZN skills, workflow-builder rules, and stale triggers."
aliases:
  - "skill canon"
  - "skill"
  - "agent skill"
source_of_truth:
  - "AGENTS.md"
  - "CLAUDE.md"
  - "tusker/SKILL.md"
  - "tusker/README.md"
  - "skills/rzn-browser/SKILL.md"
  - "skills/rzn-workflow-builder/SKILL.md"
  - "scripts/install_rzn_workflow_builder_skill.sh"
  - "crates/rzn_browser/src/skill_installer.rs"
  - "docs/workflows/AGENT_PLAYBOOK.md"
stale_when:
  paths:
    - "AGENTS.md"
    - "CLAUDE.md"
    - "tusker/SKILL.md"
    - "tusker/README.md"
    - "skills/**/SKILL.md"
    - "scripts/install_*skill*.sh"
    - "crates/rzn_browser/src/skill_installer.rs"
    - "docs/workflows/AGENT_PLAYBOOK.md"
publish:
  include_in_llms: true
  lane: "internal"
  path: "skill/canon"
created_at: "2026-05-12"
updated_at: "2026-05-13"
tags:
  - "skill"
---

# Skill canon

## Read this when

Read this before changing agent guidance, project knowledge routing, bundled skills, workflow-builder instructions, or Tusker task/closeout expectations.

## Do not read this when

Do not use this as product runtime truth; read [[runtime/CANON]]. Do not use this as workflow contract truth; read [[workflow/CANON]].

## Current model

There are three guidance layers:

| Layer | Source | Purpose |
|---|---|---|
| Repo instructions | `AGENTS.md`, `CLAUDE.md` | Mandatory contributor/agent rules for this repo: main-only workflow, no mutating git unless asked, browser automation guardrails, feature scratchpads, testing expectations. |
| Tusker project skill | `tusker/SKILL.md` | Repo-local knowledge graph router. Start with the narrowest domain `INDEX.md`, then `CANON.md`, then task history only for proof. |
| Product skills | `skills/**/SKILL.md` plus CLI installer | Reusable agent skills for using RZN Browser or authoring workflows. |

The key rule is: durable truth lives in domain canon, task records prove work, and source code wins conflicts.

## Current defaults

- Route repo-local work through `tusker/SKILL.md`, then the narrowest domain index/canon.
- Keep durable instructions in canon or source files; keep task files as evidence and closeout proof.
- Use the bundled product skills only when they match the user request or are explicitly named.

## Project knowledge routing

When answering or changing this repo:

1. Pick the narrowest domain index.
2. Read that domain's `CANON.md`.
3. Prefer canon over task history.
4. Prefer source code/API schemas over prose when exact behavior conflicts.
5. If canon and code disagree, update canon and record the conflict as a knowledge change.
6. Read task files only for proof, evidence, or implementation history.

The current high-value domains are:

| Domain | Use for |
|---|---|
| [[runtime/INDEX]] | Supervisor, native host, extension bridge, worker fallback, sessions, heal. |
| [[workflow/INDEX]] | Workflow manifests, capability routing, validation, side-effect policy, smoke proof. |
| [[codebase/INDEX]] | Repo layout, change safety, testing, source anchors. |
| [[schema/INDEX]] | Tusker V6 schema, manifest schema, action schemas, generated boundaries. |
| [[skill/INDEX]] | Agent instructions, bundled skills, project knowledge router. |

`docs`, `docs-system`, `adoption`, and `obsidian` are lower-priority here unless the work explicitly touches publication, vault mechanics, rollout, or Obsidian navigation.

## Tusker operating contract

The normal workflow is:

- find the vault
- read `tusker/README.md`
- pick the matching epic
- create/update a task
- record evidence, docs impact, verification, and close state
- run `tusker validate`

If the `tusker` CLI is unavailable, manually preserve the same contract in markdown and say validation is blocked by missing CLI. Do not fake validation.

For docs work:

- choose audience and Diataxis mode before writing
- synthesize from source-of-truth material
- do not paste task records as human docs
- resolve knowledge/docs impact with applied/noop/waived status
- keep generated files untouched

## Bundled skill surface

`rzn-browser skill install` is implemented by `crates/rzn_browser/src/skill_installer.rs`. The README documents:

- `rzn-browser skill install --global`
- `rzn-browser skill install --project`
- `rzn-browser skill update`
- `rzn-browser skill remove`

The broad product skill is `skills/rzn-browser/SKILL.md`. The narrower workflow authoring skill is `skills/rzn-workflow-builder/SKILL.md` and can also be installed through `scripts/install_rzn_workflow_builder_skill.sh`.

Skill edits should be treated like API edits: the instruction text changes agent behavior. Validate against the actual CLI/workflow/runtime surfaces before claiming they are current.

## Agent workflow rules that matter most

- Work on `main` unless the human asks for a branch.
- Do not run mutating git operations unless asked.
- Use the existing Chrome extension/native-host path for browser automation work.
- Do not launch isolated browser instances for routine workflow execution.
- Use Playwright only for repo-owned tests or explicit Playwright work.
- Every feature must have a current scratchpad under `docs/features/<feature>/README.md`.
- Workflow work must follow the tool-design rules in `AGENTS.md` and the traps in `docs/workflows/AGENT_PLAYBOOK.md`.

## Invariants

- Agent guidance should be terse and enforceable. If it cannot be checked or followed, it probably does not belong in root guidance.
- Product skills should teach agents to call the stable public surface, not private implementation shortcuts.
- Skill install/update behavior must match README examples and actual CLI code.
- Do not let Tusker task bodies become the project knowledge graph. They are proof.

## Deprecated behavior

- Beads/`bd` is no longer the repo-local tracker.
- Structural V6 migration alone is not knowledge migration.
- Generated domain pages are not acceptable final documentation unless they are explicitly marked as scaffolds.

## Source of truth

- Repo agent contract: `AGENTS.md`, `CLAUDE.md`
- Project knowledge router: `tusker/SKILL.md`, `tusker/README.md`
- Skill installer: `crates/rzn_browser/src/skill_installer.rs`
- Product skills: `skills/rzn-browser/SKILL.md`, `skills/rzn-workflow-builder/SKILL.md`
- Workflow authoring traps: `docs/workflows/AGENT_PLAYBOOK.md`

## Open questions

- Whether lower-priority generated domains should be deleted, marked scaffold, or rewritten after the five priority domains settle.
- Whether a repo-local `tusker validate` wrapper should be committed so agents are not dependent on a global binary.
- How much product-skill content should be generated from manifest/catalog inspection versus maintained by hand.

## Related

- [[skill/INDEX]]
- [[workflow/CANON]]
- [[codebase/CANON]]

## Recent changes

<!-- tusker:backrefs:begin -->
- [[OPS-T-0002]] - Replaced generated scaffold with source-backed skill canon.
<!-- tusker:backrefs:end -->
