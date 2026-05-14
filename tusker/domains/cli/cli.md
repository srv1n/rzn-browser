---
schema: tusker.knowledge/v6
title: Tusker v5 CLI surface
node: cli/cli
audience: developer
agent_layer: capsule
kind: reference
domains:
- cli
source_of_truth:
- tusker/_config/docs-map.v5-legacy.yaml
canonical_status: draft
created: '2026-05-08'
updated: '2026-05-08'
domain: cli
stale_when:
  paths:
  - tusker/_config/docs-map.v5-legacy.yaml
publish:
  lane: internal
  path: reference/cli
  include_in_llms: true
summary: Tusker v5 CLI surface
---

# Tusker v5 CLI surface

## Shape

The public CLI is small on purpose:

| Area | Commands |
|---|---|
| Setup | `init`, `update` |
| Work items | `new`, `list`, `next`, `claim`, `status`, `evidence`, `verify`, `close` |
| Docs | `docs model`, `docs map`, `docs catalog`, `docs freshness`, `docs check`, `docs apply`, `docs noop`, `docs waive`, `docs export`, `docs dev`, `docs build` |
| Shared vaults | `vault set`, `vault status`, `vault mount`, `vault unmount`, `vault repair`, `vault move` |
| Health | `validate`, `reindex` |

## Existing repo migration

```bash
tusker init --migrate-v5 --dry-run --vault ./tusker
tusker init --migrate-v5 --yes --vault-only --no-mount --vault ./tusker
```

`--migrate-v5` converts old stories and bugs into tasks, updates IDs and wikilinks, renames epic index files, installs V5 templates/views, and adds missing docs-map nodes for published docs.

## Work pickup

```bash
tusker next --vault ./tusker
tusker claim APP-T-0001 --as codex --vault ./tusker
tusker next --claim --as codex --vault ./tusker
```

`next` returns only pickable work: `ready` or `rework` tasks with no unresolved `blocked_by` dependencies. `claim` assigns the task and moves it to `active`. `draft` and `backlog` are intentionally not pickable.

## Repo-local skill refresh

```bash
tusker update --repo . --repo-only --no-bin
```

Use this after pulling or rebuilding Tusker when the repository should carry the current agent skill bundle under `.agents/skills/tusker` and `.claude/skills/tusker`.

## Docs pipeline

```bash
tusker knowledge map --json
tusker knowledge freshness --stale
tusker publish llms --vault ./tusker
tusker publish site --vault ./tusker
```

The site output is generated. Author source docs in `tusker/domains/**` or registered repo docs, not in `site/src/content/docs/**`.

## Shared Obsidian vault

```bash
tusker vault set --path /path/to/shared-obsidian-vault
tusker vault mount --repo /path/to/repo --vault /path/to/repo/tusker --name repo-name
tusker vault status
```

`vault mount` creates a symlink at `<shared-vault>/<name>` that points to the repo-local Tusker tracker. Use this when one Obsidian workspace should monitor multiple project trackers.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | user input or command error |
| 2 | validation failure |
| 3 | filesystem or I/O failure |

## Read this when

Read this when you need current Tusker v5 CLI surface knowledge.

## Do not read this when

Do not use this page as task proof; open linked tasks only for implementation history or evidence.

## Source of truth

- See frontmatter `source_of_truth`.

## Related

- [[cli/CANON]]
