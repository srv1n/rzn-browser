---
title: "Overview"
type: "note"
created: "2026-05-15"
updated: "2026-05-15"
tags: ["tusker-generated"]
---

# Project overview

<!-- tusker:overview:begin -->

`rzn-browser` uses a Tusker V6 vault. Current truth lives under `tusker/domains/**`; task files under `tusker/epics/**` are proof and implementation history.

Start with `tusker/SKILL.md`, then route through the narrowest domain `INDEX.md` and `CANON.md` before opening task history.

<!-- tusker:overview:end -->

---

# Epic roster

_Auto-generated 2026-05-15T01:27:51Z. This top-level roster intentionally shows epics only. Run `tusker list --type epic` for the live terminal view, then drill into one epic with `tusker list --epic <ACR> --type task --open`._

Agents: use this page only to choose the right epic. Do not read every task file. Pick the epic whose summary best matches; if nothing fits and the work will outlive one task, propose a new epic with `tusker new epic --acronym <ACR> --title "<name>" --summary "..."`.

## Active

### [[BRR]] — Bridge Reliability Hardening

**Summary:** Make the supervisor-native-host-extension readiness path recover from MV3 service-worker suspension, stale native-host handles, and stale extension bundles without routine user reloads.

**Counts:** 8 tasks, 3 bug tasks, 0 docs (open: 3, done: 5)

**Drill down:** `tusker list --epic BRR --type task --open`.

### [[BWR]] — Browser Worker Retirement

**Summary:** Hard-remove the legacy rzn-browser-worker runtime and broker_endpoint_v1 compatibility surface so the supervisor is the sole browser automation owner for CLI, MCP, app, cloud, and native-host bridge paths.

**Counts:** 6 tasks, 0 bug tasks, 0 docs (open: 6, done: 0)

**Drill down:** `tusker list --epic BWR --type task --open`.
