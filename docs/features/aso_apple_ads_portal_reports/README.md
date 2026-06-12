# ASO Apple Ads Portal Reports

## Overview
This feature defines a fallback, authenticated browser workflow path for Apple Ads report-like pulls when direct API fetch is not viable in the current runtime. The workflow runs through the extension/native-host stack, avoids cookie/token logging, preserves parameterized wrapper inputs for date and filters, and returns structured DOM-extracted rows plus summary context.

## Flow Diagrams
- End-to-end flow
```
Caller (make/skill-run)
  -> start_app.sh
  -> rzn-browser run-workflow
  -> broker/native host
  -> extension background/content script
  -> app-ads.apple.com (authenticated tab)
  -> DOM extraction (report rows + summary)
  <- normalized envelope
```

- Internal report flow
```
navigate -> wait -> open report tab
        -> extract_structured_data(selectors for rows + summary)
        -> envelope(row_count, params, data)
```

## Decision Record
- Chosen approach: use workflow-level DOM extraction from report pages (`extract_structured_data`) rather than API fetch calls.
- Rationale: this repo build’s `execute_javascript` path is sandboxed and does not execute arbitrary async fetch scripts, so DOM extraction is the stable path.
- Alternative rejected: relying on backend endpoint fetch via workflow JS in the current runtime.
- Alternative deferred: direct backend API integration in core ASO service (preferred long-term).

## Architecture
- Modules
  - `workflows/generated/aso/apple-ads-portal-report.json`: fallback report workflow.
  - `skills/apple-ads-portal-report/scripts/run.sh`: wrapper entrypoint.
  - `skills/amazon-appstore-workflows/scripts/run_workflow.sh`: normalized envelope runner.
- Data contracts
  - Inputs: `report_type`, `start_date`, `end_date`, optional `organization_id`, optional `campaign_id`.
  - Output: `[{ class_name, section_title?, text }]` and envelope metadata (`success`, `params`, `row_count`).

## Implementation Notes
- Selector strategy:
  - Row-like selectors: `article [role='row']`, `article tr`
  - Summary fallback selectors: `.table-toolbar__header`, `.disclosure`
- Safe-to-log fields:
  - report type/date parameters (wrapper), row count, extracted text snippets.
- Do not log:
  - cookies, authorization headers, csrf/session tokens, full raw HTML containing session state.
- Retry behavior:
  - rely on existing runner/native-host reconnect behavior and deterministic DOM extraction.

## Tasks & Status
- [x] Create report workflow under `workflows/generated/aso/`.
- [x] Add skill wrapper and normalized envelope output.
- [x] Add docs with safety notes and run examples.
- [x] Validate authenticated end-to-end DOM extraction in a live portal session.

## What Works (Do Not Change)
- `make run` + `make skill-run` as the primary local workflow execution loop.
- Normalized skill envelope fields: `success`, `params`, `row_count`, `data`.
- DOM-based extraction selectors for report rows and summary (`article [role='row']`, `article tr`, `.table-toolbar__header`, `.disclosure`).

## Tried & Didn’t Work
- Async `execute_javascript` API fetch scripts in this runtime: sandboxed execution returns empty/limited data.
- UI-click report export download flow: fragile selectors and poor parameterization for automated downstream ingestion.
