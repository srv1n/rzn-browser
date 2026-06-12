---
title: Apple Ads Keyword Recs
slug: /workflows/aso/apple-ads-keyword-recommendations/
sidebar:
  label: Apple Ads Keyword Recs
---

# apple-ads-keyword-recommendations

## JSON
- File: `workflows/generated/aso/apple-ads-keyword-recommendations.json`
- Workflow id: `apple_ads_keyword_recommendations_v1`

## Input Parameters
- `adam_id`: App adamId context.
- `adgroup_id`: Ad group context.
- `query`: Keyword seed text.
- `storefront`: Storefront code (`us`, `gb`, `ca`, etc).

## Behavior
- Navigates to `https://app-ads.apple.com/cm/app`.
- Opens the Recommendations view in the authenticated portal session.
- Extracts recommendation cards via DOM selectors (`.rc-card-component.row`).
- Returns either recommendation-like card rows or the explicit empty-state card (for example, `No Recommendations`).

## Output Shape
- Envelope (`skill-run`) includes:
  - `success`, `params`, `row_count`, `data`.
- `data` is an array of extracted card rows with fields:
  - `title`
  - `detail`
  - `raw_text`

## Notes
- This workflow currently relies on DOM extraction because the runtime sandbox in this repo build does not execute arbitrary async `execute_javascript` fetch code.

## Run
- `./skills/apple-ads-keyword-recs/scripts/run.sh --adam-id "123" --adgroup-id "456" --query "budget app" --storefront "us"`
