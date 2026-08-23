---
name: tusker
description: Track work and navigate repo knowledge through the Tusker CLI. Use when a repository contains .tusker, a Tusker ID is named, work must be recorded, or a repo-knowledge question needs a canonical answer.
---

# Tusker

`tusker capabilities --json` is executable truth; trust it over any doc, including this one. Mutate tracker state only through the CLI — lifecycle, proof, gate, and generated fields are CLI-owned, and a hand edit corrupts compare-and-swap for every other writer. If a command is missing or refuses, report that plainly: a tracker failure is not a source-code failure and never expands the coding request.

An interactive session implements the requested work itself — no daemon enablement, no run claim; only the resident daemon dispatches background runs. When `TUSKER_ATTEMPT_ID` is set you are a dispatched worker: work only the claimed task.

## Route once

Read exactly one guide:

| Request | Read |
|---|---|
| Create, update, or close tracked work | `references/TRACK.md` |
| Answer from or write repo knowledge | `references/KNOWLEDGE.md` |
| Read or update documentation/spec contracts | `references/SPECS.md` |
| Run a task, resolve gates, watch runs | `references/RUN.md` |
| Tracker diagnosis or stuck task state | `references/OPERATE.md` |
| Existing-repo onboarding | `references/REPO_ONBOARDING.md` |
| Xcode generated build-state failure | `references/XCODE_BUILD_STATE.md` |

For a read-only answer, stay here: `tusker show <ID> --capsule`, `tusker list`, `tusker search <term>`. Task history, attempts, events, `_generated`, and raw logs open only when the request names them.

## Reset a stale project

When repo-local tracker state no longer matches the current Tusker API, preview
then apply the disposable-project reset:

```sh
tusker reset --dry-run
tusker reset --yes
# or: tusker relaunch --yes
```

`reset` deletes known Tusker state (tickets, epics, proof, scratch, and generated
state), preserves `.tusker/specs/**`, leaves source and `docs/specs/**` alone,
and initializes a clean V7 vault. Use `--repo <path>` for another checkout.
It is destructive and requires `--yes`; use `tusker init --yes --purge-state
--preserve-specs` when composing the lower-level operation directly.

## Hard stop

`agent_action: stop_until_human_response` or `readiness: waiting_on_human` ends tracker mutation. Inspect with `tusker proof status <TASK-ID>` and `tusker closeout status <TASK-ID> --json`, then report the exact human action and task/gate ID. Proof is recorded, never manufactured; a human-owned gate is resolved by its owner.
