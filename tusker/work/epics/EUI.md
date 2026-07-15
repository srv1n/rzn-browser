---
schema: "tusker.epic/v7"
kind: "epic"
id: "EUI"
project: "rzn-browser"
title: "Extension UI and device observability"
status: "ready"
owner: "human:sarav"
priority: "p2"
domains: []
next_task_number: 1
next_gate_number: 1
next_decision_number: 1
created_at: "2026-07-11T07:06:59Z"
updated_at: "2026-07-13T05:42:38Z"
state_rev: "sha256:d886926db32904d35a640f6a19f9bc2fd6a9f94821344b6bf718fef73de68930"
capsule:
  skip_when: "Skip when you need a specific task contract, proof row, gate, or attempt."
  use_when: "Use to triage this workstream's scope, active tasks, and durable direction."
  what: "EUI epic: Extension UI and device observability."
---

# EUI · Extension UI and device observability

## Thesis

Rebuild the extension surface (popup + tabbed dashboard) as a thin client over new supervisor RPCs: run history store, workflow health flags with failure fingerprinting, master pause, fleet enrollment UX, logs/diagnostics.

## Success criteria

- [ ] Daily visibility never requires a terminal: popup shows health strip, now-running with Stop, master pause, and recent runs from a single `status.snapshot` RPC.
- [ ] Every run on the device (local CLI, MCP, fleet) lands in one queryable history with origin, and failed runs carry bounded failure context (console tail + screenshot/DOM excerpt).
- [ ] Repeated failures surface as degraded/broken flags with a dominant fingerprint sentence ("failed 4× at step 6, selector_not_found, since Jul 9") in the Workflows tab.
- [ ] Enroll, reconnect guidance, and unenroll all work from the Fleet tab; the device token is never displayed, logged, or exported.
- [ ] One-click diagnostics export produces a zip with tokens and raw params redacted.
- [ ] Fleet-dispatched runs notify the device owner and execute in a separate unfocused window; local runs are untouched.

## Current decision

One UI with progressive disclosure (no separate fleet mode); extension is a thin client over supervisor RPCs and owns zero state; vanilla TS (no framework) matching the repo; single master pause switch in v1. Task order: 0001→0002→0003 (supervisor), then 0004 (shell) unlocks 0005–0009. Cross-repo canon with backend FLD: `failure_summary {error_class, failing_step_index, fingerprint, message}` on RunResultV2.

## Open gates

<!-- tusker:generated open-gates -->

| Gate | Owner | Blocks | Action |
|---|---|---|---|
| _None._ |  |  |  |

## Active work

<!-- tusker:generated active-work -->

| Task | Status | Next owner | Next action |
|---|---|---|---|
| [[EUI-T-0001]] | review | reviewer | Review evidence and close or return to rework. |
| [[EUI-T-0002]] | review | reviewer | Review evidence and close or return to rework. |
| [[EUI-T-0003]] | review | reviewer | Review evidence and close or return to rework. |
| [[EUI-T-0004]] | review | reviewer | Review evidence and close or return to rework. |
| [[EUI-T-0005]] | review | reviewer | Review evidence and close or return to rework. |
| [[EUI-T-0006]] | review | reviewer | Review evidence and close or return to rework. |
| [[EUI-T-0007]] | review | reviewer | Review evidence and close or return to rework. |
| [[EUI-T-0008]] | ready | agent | Execute the task contract and satisfy proof mode. |
| [[EUI-T-0009]] | review | reviewer | Review evidence and close or return to rework. |

## Recently completed

<!-- tusker:generated recently-completed -->

| Task | Accepted by | Closed at |
|---|---|---|
| _None._ |  | |
