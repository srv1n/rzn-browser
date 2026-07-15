---
schema: "tusker.epic/v7"
kind: "epic"
id: "FLA"
project: "rzn-browser"
title: "Fleet laptop agent"
status: "ready"
owner: "human:sarav"
priority: "p2"
domains: []
next_task_number: 1
next_gate_number: 1
next_decision_number: 1
created_at: "2026-07-11T02:01:17Z"
updated_at: "2026-07-11T03:57:29Z"
state_rev: "sha256:72204990f222785451bcf8762e14dae86618755906166414297da0feaf5cd6d4"
---

# FLA · Fleet laptop agent

## Thesis

Turn the supervisor daemon into a fleet agent for the backend control plane: fleet wire contracts in rzn_contracts, run-loop extraction from native_runner so the supervisor can execute whole workflows, jittered simple-poll loop with disk journal and dedupe, workflow cache by content hash with GC, and CLI enrollment/status commands. Extension and native host unchanged; local CLI runner stays fully functional.

## Success criteria

- [ ] Define success criteria.

## Current decision

TBD.

## Open gates

<!-- tusker:generated open-gates -->

| Gate | Owner | Blocks | Action |
|---|---|---|---|
| _None._ |  |  |  |

## Active work

<!-- tusker:generated active-work -->

| Task | Status | Next owner | Next action |
|---|---|---|---|
| [[FLA-T-0001]] | review | reviewer | Review evidence and close or return to rework. |
| [[FLA-T-0002]] | review | reviewer | Review evidence and close or return to rework. |
| [[FLA-T-0003]] | review | reviewer | Review evidence and close or return to rework. |
| [[FLA-T-0004]] | review | reviewer | Review evidence and close or return to rework. |
| [[FLA-T-0005]] | review | reviewer | Review evidence and close or return to rework. |
| [[FLA-T-0006]] | ready | agent | Execute the task contract and satisfy proof mode. |

## Recently completed

<!-- tusker:generated recently-completed -->

| Task | Accepted by | Closed at |
|---|---|---|
| _None._ |  | |
