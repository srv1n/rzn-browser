---
schema: tusker.task/v7
kind: task
id: "{{id}}"
title: "{{title}}"
epic: "{{epic}}"
task_kind: bug
status: ready
readiness: ready
priority: p1
risk: medium
size: s
domains: []
proof_mode: inline
proof_status: pending
proof_required:
  - focused_test
evidence_budget: 0
raw_artifacts_allowed: false
next_owner: agent
next_source: task
next_ref: "{{id}}"
next_action: "Reproduce or isolate the bug, fix the smallest cause, verify the acceptance item, and request review."
agent_action: continue
state_rev: 1
created_at: "{{date}}"
updated_at: "{{date}}"
---

# {{id}} · {{title}}

## Agent capsule

- Essence: {{title}}.
- Next action: reproduce/isolate, fix the smallest cause, satisfy proof mode, and request review.
- Read next: this note, then the named code anchors.
- Avoid: broad debugging, raw logs in the task body, and unrelated cleanup.

## Intent

## Symptom

-

## Reproduction

Steps:
1.

Expected:
-

Observed:
-

## Acceptance contract

| ID | Outcome | Proof required | Owner |
|---|---|---|---|
| A1 | The bug no longer reproduces under the stated conditions. | focused_test or focused manual check | agent |

## Scope

In:
-

Out:
-

## Verification

| Acceptance | Check | Result | Notes |
|---|---|---|---|

## Evidence

_No evidence yet. Add evidence only when proof mode requires a durable card or artifact._
