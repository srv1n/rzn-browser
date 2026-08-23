---
schema: "tusker.project-skill/v7"
kind: "project_skill"
name: "project-knowledge"
project: "rzn-browser"
status: "current"
description: "Route agents through this repository's V7 domain canon without publishing task proof or runtime state."
capsule:
  skip_when: []
  use_when: []
  what: ""
operator_skill: "tusker"
source_of_truth:
  - "knowledge/domains"
canonical_files:
  - "SKILL.md"
  - "knowledge/domains/*/INDEX.md"
  - "knowledge/domains/*/CANON.md"
created_at: "2026-08-23T10:56:19Z"
updated_at: "2026-08-23T10:56:19Z"
state_rev: "sha256:766805f35e6d26981ca6e1bc7c94174726fe2b5bac3f10ffbbe1cf0b83d914b4"
---

# Project Knowledge Skill

This is the project knowledge skill for this repository. Use it after the Tusker operator skill when repository-specific canon is needed.

## Read This When

- You need durable repository-specific canon before implementing a task.
- A task packet routes you to one or more project domains.
- You are updating project knowledge after behavior, policy, or interfaces changed.

## Do Not Read This When

- You only need Tusker task lifecycle, proof, gates, closeout, or CLI semantics; use the Tusker operator skill.
- You are looking for raw proof logs, task history, attempts, events, generated packets, or local runtime state.

## First Action

Task agents must run `tusker packet <TASK-ID> --for agent`, then read only the routed domains from that packet unless the task contract names a narrower file.

## Routing Algorithm

1. Read this `SKILL.md`.
2. Use the task packet or intent to choose the narrowest matching domain.
3. Read that domain `INDEX.md`.
4. Read that domain `CANON.md`.
5. Open deeper runbooks, decisions, interfaces, invariants, sources, or glossary entries only when the domain files route you there.

## Domains

| Domain | Read when | Read first | Canon |
|---|---|---|---|
| Project | Durable project knowledge. | `knowledge/domains/project/INDEX.md` | `knowledge/domains/project/CANON.md` |

## Repo Command Policy

- Put repository-specific command rules here or in routed runbooks: validation commands, build-lock/status commands, token/noise wrappers, and forbidden expensive probes.
- Keep root `AGENTS.md` and `CLAUDE.md` as managed Tusker bootstrap pointers; do not copy Tusker workflow mechanics there.
- Agents should prefer path-scoped status/search, lock/status commands over process-table probes, redirected validation logs, and command + PASS/FAIL summaries.

## Updating Canon

- Update the narrowest owning domain `CANON.md` when durable truth changes.
- Create or update a leaf node only when the canon needs a stable runbook, interface, invariant, decision, glossary entry, or source attribution.
- Run `tusker validate --json` after changing project knowledge.
- Do not put proof logs, task history, attempts, event streams, generated packets, or raw terminal output in canon.

## Forbidden Source Truth

- Do not publish task records, evidence logs, attempts, event files, generated output, runtime state, or raw logs as project skill source.
- Forbidden paths include `work/**`, `epics/**`, `evidence/**`, `attempts/**`, `events/**`, `_generated/**`, `_system/**`, `dashboards/**`, packet caches, `.tusker-*`, raw logs, and local absolute paths.
- Raw external input belongs in `knowledge/domains/<domain>/sources/`.
- Root `docs/` may contain optional repository engineering guardrails; it is not the V7 canonical knowledge source.

## Validation

- `tusker skill doctor --strict --json` checks project skill routes and package hygiene.
- `tusker validate --json` checks V7 domain layout and task-domain coverage.
