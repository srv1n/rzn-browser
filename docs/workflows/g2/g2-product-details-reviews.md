---
title: G2 Product Details Reviews
slug: /workflows/g2/g2-product-details-reviews/
sidebar:
  label: G2 Product Details Reviews
---

# g2-product-details-reviews

## JSON
- File: `workflows/g2/g2-product-details-reviews.json`
- Workflow id: `g2_product_details_reviews`

## Input Parameters
- `product_url`: Full G2 product URL.

## Behavior
- Opens product page.
- Extracts product-level fields (name, rating, counts, category, description metadata).
- Attempts review extraction with pagination pass.

## Run
- `./skills/g2-product-details-reviews/scripts/run.sh --product-url "https://www.g2.com/products/jira/reviews"`
