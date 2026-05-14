---
schema: tusker.knowledge/v6
title: Tusker v5 adoption guide
node: adoption/spec/v5-adoption
audience: developer
agent_layer: none
kind: how-to
domains:
- adoption
source_of_truth:
- tusker/_config/docs-map.v5-legacy.yaml
canonical_status: draft
created: '2026-05-08'
updated: '2026-05-08'
domain: adoption
stale_when:
  paths:
  - tusker/_config/docs-map.v5-legacy.yaml
publish:
  lane: internal
  path: spec/v5-adoption
  include_in_llms: true
summary: Tusker v5 adoption guide
---

# Tusker v5 adoption guide

## Goal

Move an existing Tusker vault onto V5 without hand-renaming notes or leaving old story/bug concepts behind.

## Existing repo repair

Run this from the repo root after installing or rebuilding the current Tusker binary:

```bash
tusker init --migrate-v5 --dry-run --vault ./tusker
tusker init --migrate-v5 --yes --vault-only --no-mount --vault ./tusker
tusker validate --vault ./tusker
tusker publish llms --vault ./tusker
tusker publish site --vault ./tusker
tusker update --repo . --repo-only --no-bin
```

Use `--vault-only` when the repo already has its own `AGENTS.md`, `CLAUDE.md`, or project contract files and the goal is only to repair the Tusker vault.

## What migration changes

| Legacy shape | V5 shape |
|---|---|
| `type: story` | `type: task`, `kind: feature` |
| `type: bug` | `type: task`, `kind: bug` |
| `ABC-S-0001.md` | `ABC-T-0001.md` |
| `ABC-B-0001.md` | next non-conflicting `ABC-T-NNNN.md` |
| `epics/ABC/index.md` | `epics/ABC/ABC.md` |
| old wikilinks | rewritten to the new task IDs |
| missing docs map entries | added for published docs |

## Acceptance

- `tusker validate --vault ./tusker` exits with zero errors.
- No `*-S-NNNN.md`, `*-B-NNNN.md`, `Stories.base`, `Bugs.base`, or `story.md` files remain in the vault.
- `tusker list --vault ./tusker --type epic` shows the expected epic roster.
- `tusker publish site --vault ./tusker` completes.

Warnings about missing V5 sections in old notes are migration debt, not a broken repo. Fix them when touching the note for real work.

## Rollback

By default migration creates `tusker.backup-v5-YYYYMMDD-HHMMSS`. If a repo has a watcher that restores or deletes sidecar backups, rerun with `--no-backup` only after making your own git or filesystem checkpoint.

## Read this when

Read this when you need current Tusker v5 adoption guide knowledge.

## Do not read this when

Do not use this page as task proof; open linked tasks only for implementation history or evidence.

## Source of truth

- See frontmatter `source_of_truth`.

## Related

- [[adoption/CANON]]
