# ASO Workflows Pack v1

## Overview
This feature adds a small ASO-oriented workflow pack for downstream automation: Apple Ads keyword recommendations, Apple Ads fallback report pulls, and an optional App Store web search snapshot. Constraints are fast iteration, no code-path site special-casing, no secret logging, and compatibility with existing `make run`/`make skill-run` flows.

## Flow Diagrams
- End-to-end flow
```
make run / make skill-run
  -> start_app.sh
  -> rzn-browser run-workflow
  -> extension + broker
  -> target web app
  -> extracted JSON + screenshots
```

- Skill wrapper flow
```
skills/*/run.sh
  -> shared run_workflow.sh
  -> workflow execution
  -> parse final JSON block
  -> normalized envelope {success, params, row_count, data}
```

## Decision Record
- Keep new workflows under `workflows/generated/aso/` for quick delivery and clear ownership.
- Reuse shared runner with command extensions instead of adding one-off wrapper logic per workflow.
- Add docs under `docs/workflows/aso/` plus feature scratchpads under `docs/features/` to preserve onboarding context.

## Architecture
- Workflows
  - `workflows/generated/aso/apple-ads-keyword-recommendations.json`
  - `workflows/generated/aso/apple-ads-portal-report.json`
  - `workflows/generated/aso/appstore-search-snapshot.json`
- Wrappers
  - `skills/apple-ads-keyword-recs/scripts/run.sh`
  - `skills/apple-ads-portal-report/scripts/run.sh`
  - `skills/appstore-search-snapshot/scripts/run.sh`
  - shared: `skills/amazon-appstore-workflows/scripts/run_workflow.sh`
- Docs
  - `docs/workflows/aso/*`
  - `docs/features/aso_apple_ads_portal_reports/README.md`

## Implementation Notes
- Apple Ads workflows assume authenticated portal session is already active in Chrome.
- Apple Ads workflows use DOM extraction (`extract_structured_data`) rather than async API fetch scripts in `execute_javascript`.
- Shared runner now handles object-shaped outputs and multi-parameter commands.
- App Store snapshot extracts app IDs from URLs using post-processing regex.

## Tasks & Status
- [x] Add Apple Ads keyword recommendation workflow JSON.
- [x] Add Apple Ads portal report fallback workflow JSON.
- [x] Add optional App Store search snapshot workflow JSON.
- [x] Add wrapper skills for all three workflows.
- [x] Extend Makefile skill commands and convenience targets.
- [x] Add workflow docs and feature scratchpads.
- [x] Run authenticated end-to-end validation for Apple Ads portal workflows.

## What Works (Do Not Change)
- Shared skill envelope contract and `--show-log` stderr-only behavior.
- Existing workflow command compatibility for Amazon/App Store/G2/Capterra/Etsy commands.
- `workflows/generated/` placement for quick iteration workflows.

## Tried & Didn’t Work
- New dedicated runner per ASO workflow: duplicated logic and output parsing risk.
- Embedding workflow-specific envelope shaping inside each wrapper: harder to keep stable as workflows evolve.
