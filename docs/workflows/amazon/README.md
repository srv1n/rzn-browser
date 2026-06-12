---
title: Amazon Workflows
slug: /workflows/amazon/
sidebar:
  label: Amazon
---

# Amazon Workflows

## Scope
- System: `amazon.com`
- Workflow JSON root: `workflows/amazon/`

## Workflows
- [`amazon-search`](./amazon-search.md)
- [`amazon-product-key-facts-reviews`](./amazon-product-key-facts-reviews.md)

## Commands
- Search:
  - `./skills/amazon-search/scripts/run.sh --query "wireless mouse"`
- Product details + reviews:
  - `./skills/amazon-product/scripts/run.sh --product-url "https://www.amazon.com/dp/B07FZ8S74R"`
