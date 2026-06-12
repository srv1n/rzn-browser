# Workflow Docs Structure

## Overview
- Goal: move workflow documentation to a system-first folder structure so each system has a clear root path and per-workflow pages suitable for Astro routing.
- Constraint: do not mix unrelated systems in one README.

## Flow Diagrams
```
docs/workflows/
  <system>/
    README.md
    <workflow>.md
```

## Decision Record
- Chosen: `docs/workflows/<system>/` as canonical docs route for workflow docs.
  Rationale: simple system-level routing and easier ownership.
- Chosen: one markdown file per workflow.
  Rationale: keeps command/params/status isolated and scannable.

## Architecture
- Root index: `docs/workflows/README.md`
- System roots:
  - `docs/workflows/amazon/`
  - `docs/workflows/appstore/`
  - `docs/workflows/g2/`
  - `docs/workflows/capterra/`
  - `docs/workflows/etsy/`

## Implementation Notes
- Existing mixed docs are split into system docs.
- Existing workflow JSON filenames remain unchanged.

## Tasks & Status
- [x] Create system-first docs tree.
- [x] Add system-level README pages.
- [x] Add workflow-level markdown pages.
- [x] Deprecate mixed-system README pages.

## What Works (Do Not Change)
- One canonical JSON filename per workflow.
- System-first doc routing under `docs/workflows/`.

## Tried & Didn’t Work
- Keeping mixed-system docs for convenience.
  - Rejected because it does not scale and makes ownership unclear.
