---
title: "Tusker Dashboard"
type: note
created_at: "{{date}}"
updated_at: "{{date}}"
tags: [dashboard]
---

# Tusker Dashboard

Plain markdown dashboard seed. Generated sections may be refreshed by `tusker reindex`.

## Workflow status

| Surface | Current setting |
|---|---|
| Agent-runnable statuses | `ready`, `rework` |
| Agent-runnable readiness | `ready` |
| Review checkpoint | `review` |
| Human wait | `readiness: held` with `next_owner: human:*` + `agent_action: stop_until_human_response` |
| Runtime state | leases/runtime store, not task status |

## Ready for agent

<!-- tusker:agent-ready:begin -->
_Run `tusker reindex` to refresh._
<!-- tusker:agent-ready:end -->

## Waiting on review

<!-- tusker:review:begin -->
_Run `tusker reindex` to refresh._
<!-- tusker:review:end -->

## Waiting on human/external

<!-- tusker:human-wait:begin -->
_Run `tusker reindex` to refresh._
<!-- tusker:human-wait:end -->

## Live runs

<!-- tusker:live-runs:begin -->
_Run `tusker reindex` to refresh._
<!-- tusker:live-runs:end -->

## Backlog

<!-- tusker:backlog:begin -->
_Run `tusker reindex` to refresh._
<!-- tusker:backlog:end -->
