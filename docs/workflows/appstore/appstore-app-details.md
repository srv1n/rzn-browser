---
title: App Store App Details
slug: /workflows/appstore/appstore-app-details/
sidebar:
  label: App Store App Details
---

# appstore-app-details

## JSON
- File: `workflows/appstore/appstore-app-details.json`
- Workflow id: `appstore_app_details`

## Input Parameters
- `app_id`: Numeric App Store app id.

## Behavior
- Opens app details page from app id.
- Extracts:
  - Core app metadata
  - Ratings and review summary
  - Screenshots
  - Review rows from inline and full review sections

## Run
- `./skills/appstore-details/scripts/run.sh --app-id "1232780281"`
