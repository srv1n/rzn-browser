---
title: ASO Workflows
slug: /workflows/aso/
sidebar:
  label: ASO
---

# ASO Workflows

## Scope
- Systems:
  - `app-ads.apple.com` (Apple Ads portal, authenticated)
  - `apps.apple.com` (App Store web snapshot)
- Workflow JSON root: `workflows/generated/aso/`

## Workflows
- [`apple-ads-keyword-recommendations`](./apple-ads-keyword-recommendations.md)
- [`apple-ads-portal-report`](./apple-ads-portal-report.md)
- [`appstore-search-snapshot`](./appstore-search-snapshot.md)

## Prerequisites
- Apple Ads workflows require an already-authenticated portal session in Chrome.
- Never log secrets/cookies/tokens.

## Commands
- Keyword recs:
  - `./skills/apple-ads-keyword-recs/scripts/run.sh --adam-id "123" --adgroup-id "456" --query "budget app" --storefront "us"`
- Portal report fallback:
  - `./skills/apple-ads-portal-report/scripts/run.sh --report-type "campaigns" --start-date "2026-02-01" --end-date "2026-02-15" --organization-id "123"`
- App Store snapshot:
  - `./skills/appstore-search-snapshot/scripts/run.sh --term "budget app" --country "us"`
