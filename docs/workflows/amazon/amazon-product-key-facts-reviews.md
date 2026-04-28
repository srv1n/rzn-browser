---
title: Amazon Product Key Facts Reviews
slug: /workflows/amazon/amazon-product-key-facts-reviews/
sidebar:
  label: Amazon Product Key Facts Reviews
---

# amazon-product-key-facts-reviews

## JSON
- File: `workflows/amazon/amazon-product-key-facts-reviews.json`
- Workflow id: `amazon_product_key_facts_reviews_v1`

## Input Parameters
- `product_url`: Full Amazon product URL.

## Behavior
- Opens product page.
- Extracts product key facts from PDP.
- Opens full reviews page and extracts multiple review pages.

## Run
- `./skills/amazon-product/scripts/run.sh --product-url "https://www.amazon.com/dp/B07FZ8S74R"`
