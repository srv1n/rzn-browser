---
title: Apple Ads Portal Report
slug: /workflows/aso/apple-ads-portal-report/
sidebar:
  label: Apple Ads Portal Report
---

# apple-ads-portal-report

## JSON
- File: `workflows/generated/aso/apple-ads-portal-report.json`
- Workflow id: `apple_ads_portal_report_v1`

## Input Parameters
- `report_type`: Report family (`campaigns`, `adgroups`, `keywords`, `searchterms`, etc).
- `start_date`: Start date (`YYYY-MM-DD`).
- `end_date`: End date (`YYYY-MM-DD`).
- `organization_id`: Optional org filter.
- `campaign_id`: Optional campaign filter.

## Behavior
- Fallback workflow for UI/portal-only report pulls.
- Navigates to Apple Ads portal and opens the campaigns report tab.
- Extracts visible report-table rows when present (`article [role='row']`, `article tr`).
- Always extracts report summary context (`.table-toolbar__header`, `.disclosure`) so output is non-empty even when no row data is visible.

## Output Shape
- Envelope (`skill-run`) includes:
  - `success`, `params`, `row_count`, `data`.
- `data` is an array of extracted entries with fields:
  - `class_name`
  - `section_title`
  - `text`

## Notes
- This is a fallback path and should not replace official APIs by default.
- Requires active authenticated portal session.
- Date and filter parameters are retained in wrapper inputs for downstream compatibility, but this workflow extracts currently visible UI data rather than calling backend report APIs.

## Run
- `./skills/apple-ads-portal-report/scripts/run.sh --report-type "campaigns" --start-date "2026-02-01" --end-date "2026-02-15" --organization-id "123"`
