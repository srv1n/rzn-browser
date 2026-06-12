---
title: Capterra Product Details Reviews
slug: /workflows/capterra/capterra-product-details-reviews/
sidebar:
  label: Capterra Product Details Reviews
---

# capterra-product-details-reviews

## JSON
- File: `workflows/capterra/capterra-product-details-reviews.json`
- Workflow id: `capterra_product_details_reviews`

## Input Parameters
- `product_url`: Full Capterra product URL.

## Behavior
- Opens product/reviews page.
- Extracts product-level fields (name, rating, review count, category, description metadata).
- Attempts multi-pass review extraction (initial, scroll pass, next-page pass).

## Run
- `./skills/capterra-product-details-reviews/scripts/run.sh --product-url "https://www.capterra.com/p/155928/Zoho-CRM/reviews/"`
