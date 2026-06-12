---
title: App Store Search Snapshot
slug: /workflows/aso/appstore-search-snapshot/
sidebar:
  label: App Store Snapshot
---

# appstore-search-snapshot

## JSON
- File: `workflows/generated/aso/appstore-search-snapshot.json`
- Workflow id: `appstore_search_snapshot_v1`

## Input Parameters
- `term`: App Store search term.
- `country`: Storefront country code (`us`, `gb`, `ca`, etc).

## Behavior
- Opens App Store iPhone web search for term/country (`/{country}/iphone/search?term=...`).
- Waits for hydration and extracts first-view app rows.
- Parses app IDs from app URLs (`id123...`).
- Captures top-fold screenshot artifact.

## Output Shape
- `appstore_snapshot[]` rows include:
  - `app_id`
  - `app_name`
  - `app_url`
  - `developer`

## Caution
- This workflow is intended for one-off observational sampling, not high-scale crawling.

## Run
- `./skills/appstore-search-snapshot/scripts/run.sh --term "budget app" --country "us"`
