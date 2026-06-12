---
schema: "tusker.epic/v7"
kind: "epic"
id: "SEC"
project: "rzn-browser"
title: "Security and reliability hardening (2026-06-11 full-repo review)"
status: "ready"
owner: "sarav"
priority: "p2"
domains: []
next_task_number: 1
next_gate_number: 1
next_decision_number: 1
created_at: "2026-06-11T13:28:14Z"
updated_at: "2026-06-11T16:11:00Z"
state_rev: "sha256:d8e6ee44923fdf0fe2bfd06f9df0206490cda730cf4c46f38134fdec936e14d6"
---

# SEC · Security and reliability hardening (2026-06-11 full-repo review)

## Thesis

Fix critical security gaps and reliability bugs found in the 2026-06-11 five-agent full-repo audit: unauthenticated page bridges, workflow param injection, unverified installers, world-readable secrets, broker desync, missing command allowlists, prompt-injection hardening, CI gates.

## Success criteria

- [ ] No unauthenticated page-reachable privileged surface in the extension (SEC-T-0001, SEC-T-0008).
- [ ] Workflow params cannot inject into embedded JS; all workflows use the args channel (SEC-T-0002).
- [ ] No secret/credential file written world-readable; no sensitive debug logs in /tmp or $HOME (SEC-T-0004, SEC-T-0005).
- [ ] Installers verify artifact checksums before executing anything (SEC-T-0006).
- [ ] Supervisor wire protocol has one implementation, one frame limit, and survives timeouts without desync (SEC-T-0007).
- [ ] Autonomous agent wraps all page-derived text as untrusted (SEC-T-0009).
- [ ] CI fails on clippy warnings, new RUSTSEC advisories, and invalid workflow JSON (SEC-T-0013).
- [ ] Remaining medium correctness checklist closed (SEC-T-0003, SEC-T-0010, SEC-T-0011, SEC-T-0012, SEC-T-0014).

## Execution order and file-ownership notes

Origin: five-agent audit on 2026-06-11; full findings digest in `tusker/feedback/agents/2026-06-11-full-repo-security-review.md`. Line anchors in task contexts are from that audit — agents must re-verify anchors with grep before editing.

Independent, start anytime (disjoint files): SEC-T-0001, SEC-T-0002, SEC-T-0003, SEC-T-0006, SEC-T-0010, SEC-T-0011, SEC-T-0013.

Ordering constraints (shared files):
- SEC-T-0005 before SEC-T-0004 preferred (T4 reuses the rzn_core secure-write helper; otherwise T4 inlines and T5 consolidates).
- SEC-T-0008 (background.ts registry refactor) before SEC-T-0012 (cloud popup/https path edits background.ts) — avoid conflicting large diffs.
- SEC-T-0007 (broker/supervisor framing) before SEC-T-0014 items 7-10 (same files).
- SEC-T-0001 and SEC-T-0008 both introduce a prod/dev build flag in extension build scripts — whichever lands second reuses the first one's flag.

## Current decision

Land p0 tasks (0001, 0002, 0003, 0005, 0006) first; they are independently shippable. p1/p2 follow the ordering constraints above.

## Open gates

<!-- tusker:generated open-gates -->

| Gate | Owner | Blocks | Action |
|---|---|---|---|
| [[SEC-G-0001]] | human:sarav | [[SEC-T-0001]] | Review SEC-T-0001 bridge-auth diff and accepted evidence, then satisfy or return to rework. |
| [[SEC-G-0002]] | human:sarav | [[SEC-T-0002]] | Review SEC-T-0002 workflow-param escaping diff and accepted evidence, then satisfy or return to rework. |

## Active work

<!-- tusker:generated active-work -->

| Task | Status | Next owner | Next action |
|---|---|---|---|
| [[SEC-T-0001]] | review | human:sarav | Accept, waive, or return rework for SEC-G-0001. |
| [[SEC-T-0002]] | review | human:sarav | Accept, waive, or return rework for SEC-G-0002. |
| [[SEC-T-0003]] | review | reviewer | Review evidence and close or return to rework. |
| [[SEC-T-0005]] | review | reviewer | Review evidence and close or return to rework. |
| [[SEC-T-0006]] | review | reviewer | Review evidence and close or return to rework. |
| [[SEC-T-0007]] | review | reviewer | Review evidence and close or return to rework. |
| [[SEC-T-0008]] | review | reviewer | Review evidence and close or return to rework. |

## Recently completed

<!-- tusker:generated recently-completed -->

| Task | Accepted by | Closed at |
|---|---|---|
| [[SEC-T-0004]] | reviewer:sec-review-a | 2026-06-11T16:10:49Z |
| [[SEC-T-0009]] | reviewer:sec-review-b | 2026-06-11T16:10:49Z |
| [[SEC-T-0010]] | reviewer:sec-review-b | 2026-06-11T16:10:49Z |
| [[SEC-T-0011]] | reviewer:sec-review-c | 2026-06-11T16:10:49Z |
| [[SEC-T-0012]] | reviewer:sec-review-c | 2026-06-11T16:10:49Z |
| [[SEC-T-0013]] | reviewer:sec-review-c | 2026-06-11T16:10:49Z |
| [[SEC-T-0014]] | reviewer:sec-review-c | 2026-06-11T16:10:49Z |
