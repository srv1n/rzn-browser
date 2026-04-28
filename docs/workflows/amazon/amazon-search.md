---
title: Amazon Search
slug: /workflows/amazon/amazon-search/
sidebar:
  label: Amazon Search
---

# amazon-search

## JSON
- File: `workflows/amazon/amazon-search.json`
- Workflow id: `amazon_search`

## Input Parameters
- `search_query`: Amazon search text.

## Output Shape
- Extracted rows from search result cards, including product link candidates.

## Run
- `./skills/amazon-search/scripts/run.sh --query "wireless mouse"`
