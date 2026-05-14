---
schema: tusker.project-skill/v6
name: project-knowledge
description: Understand, modify, explain, or verify rzn-browser using domain canon, codebase map, task proof, and the Tusker V6 knowledge graph.
---

# Project knowledge skill

Use this file to route through this repository's Tusker knowledge graph.
Use the Tusker operator skill for task mechanics and CLI workflow.

## Routing rule

Start with the narrowest domain INDEX. Read CANON before task history.
Read task files only for proof, evidence, or implementation history.

## Answering rules

1. Prefer domain CANON.md over task history.
2. Prefer source code or API schemas over prose when exact behavior conflicts.
3. When code and canon disagree, trust code, mark canon stale, and report the conflict.
4. Do not read generated output by default.
5. Do not load full files when a capsule or section read is enough.
6. When suggesting a code change, include verification.
7. When production impact is possible, include rollback or safe-change checks.

## Domains

<!-- tusker:domains:begin -->
| Intent | Read first | Canon | Notes |
|---|---|---|---|
| Install, migrate, or roll out Tusker | [[adoption/INDEX]] | [[adoption/CANON]] | Install, migration, rollout, and consumer repo adoption. |
| Change or inspect CLI behavior | [[cli/INDEX]] | [[cli/CANON]] | Command surface, flags, help text, routing, and user-visible terminal behavior. |
| Change repository code safely | [[codebase/INDEX]] | [[codebase/CANON]] | Repository layout, implementation anchors, testing, source authority, and safe change rules. |
| Understand docs | [[docs/INDEX]] | [[docs/CANON]] | Current durable truth for docs. |
| Understand docs system | [[docs-system/INDEX]] | [[docs-system/CANON]] | Current durable truth for docs system. |
| Change vault navigation or Obsidian views | [[obsidian/INDEX]] | [[obsidian/CANON]] | Vault layout, wikilinks, managed blocks, Bases views, and graph navigation. |
| Understand supervisor, native host, extension bridge, or worker fallback | [[runtime/INDEX]] | [[runtime/CANON]] | Current local browser runtime model: supervisor, native host, extension, legacy worker fallback, sessions, heal, and logs. |
| Change frontmatter, manifests, validation, templates, or schemas | [[schema/INDEX]] | [[schema/CANON]] | Tusker V6 note schema, workflow manifest v2, run result envelopes, action schemas, templates, validation, and generated boundaries. |
| Change operator or project skill guidance | [[skill/INDEX]] | [[skill/CANON]] | Project knowledge routing, repo agent guidance, Tusker workflow, bundled skills, workflow-builder guidance, and skill installer behavior. |
| Change workflow manifests, capability routing, effects, or validation | [[workflow/INDEX]] | [[workflow/CANON]] | Workflow contract platform, manifest-shaped workflow files, capability routing, validation, side effects, and smoke proof. |
<!-- tusker:domains:end -->
