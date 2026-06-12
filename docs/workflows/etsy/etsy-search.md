---
title: Etsy Search
slug: /workflows/etsy/etsy-search/
sidebar:
  label: Etsy Search
---

# etsy-search

## JSON
- File: `workflows/etsy/etsy-search.json`
- Workflow id: `etsy_search`

## Input Parameters
- `search_query`: Etsy search text.

## Behavior
- Opens search results.
- Extracts listing cards with listing URL, title, price, shop, and rating text.

## Run
- `./skills/etsy-search/scripts/run.sh --query "leather wallet"`
