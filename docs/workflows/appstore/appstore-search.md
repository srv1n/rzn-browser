---
title: App Store Search
slug: /workflows/appstore/appstore-search/
sidebar:
  label: App Store Search
---

# appstore-search

## JSON
- File: `workflows/appstore/appstore-search.json`
- Workflow id: `appstore_search`

## Input Parameters
- `app_query`: Search text.

## Behavior
- Navigates to iPhone App Store search URL:
  - `https://apps.apple.com/us/iphone/search?term={app_query}`
- Extracts app candidates and app URLs.

## Run
- `./skills/appstore-search/scripts/run.sh --query "notion"`
