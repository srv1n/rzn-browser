---
title: Tusker V6 vault
type: note
created: '2026-05-12'
updated: '2026-05-12'
tags:
- v6
- knowledge-graph
---

# Project overview

<!-- tusker:overview:begin -->

`rzn-browser` uses a Tusker V6 vault. Current truth lives under `tusker/domains/**`; task files under `tusker/epics/**` are proof and implementation history.

Start with `tusker/SKILL.md`, then route through the narrowest domain `INDEX.md` and `CANON.md` before opening task history.

<!-- tusker:overview:end -->

---

# Domain Roster

| Domain | Canon | Summary |
|---|---|---|
| [[adoption/INDEX]] | [[adoption/CANON]] | Install, migration, rollout, and consumer repo adoption. |
| [[cli/INDEX]] | [[cli/CANON]] | Command surface, flags, help text, routing, and user-visible terminal behavior. |
| [[codebase/INDEX]] | [[codebase/CANON]] | Repository layout, implementation anchors, testing, source authority, and safe change rules. |
| [[docs/INDEX]] | [[docs/CANON]] | Current durable truth for docs. |
| [[docs-system/INDEX]] | [[docs-system/CANON]] | Current durable truth for docs system. |
| [[obsidian/INDEX]] | [[obsidian/CANON]] | Vault layout, wikilinks, managed blocks, Bases views, and graph navigation. |
| [[runtime/INDEX]] | [[runtime/CANON]] | Current local browser runtime model: supervisor, native host, extension, legacy worker fallback, sessions, heal, and logs. |
| [[schema/INDEX]] | [[schema/CANON]] | Tusker V6 note schema, workflow manifest v2, run result envelopes, action schemas, templates, validation, and generated boundaries. |
| [[skill/INDEX]] | [[skill/CANON]] | Project knowledge routing, repo agent guidance, Tusker workflow, bundled skills, workflow-builder guidance, and skill installer behavior. |
| [[workflow/INDEX]] | [[workflow/CANON]] | Workflow contract platform, manifest-shaped workflow files, capability routing, validation, side effects, and smoke proof. |

# Epic Roster

| Epic | Status | Primary domains | Summary |
|---|---|---|---|
| [[LRT]] | active | runtime | Unify browser automation runtime ownership around a durable rzn-browser supervisor so CLI, MCP, Reason app, and cloud jobs share one stable local contract while the Chrome extension and native host remain thin browser transport pieces. |
| [[OPS]] | draft | skill | Repo-local agent workflow, contributor guidance, and tracking hygiene for rzn-browser. |
| [[WAT]] | draft | workflow | Systematic end-to-end validation of every workflow in workflows/<system>/. Run each via 'rzn-browser run', record pass/fail, and flag broken selectors or auth-gated flows. For post/submit workflows, verify scaffolding through the last pre-submit step but do not actually post. |
| [[WCP]] | draft | workflow | Define and enforce the core workflow/capability ABI before public launch: typed manifests, stable run envelopes, capability routing, side-effect policy, catalog lifecycle, contract tests, and downstream workflow-team enablement. |
