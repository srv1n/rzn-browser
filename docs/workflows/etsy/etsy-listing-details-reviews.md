---
title: Etsy Listing Details Reviews
slug: /workflows/etsy/etsy-listing-details-reviews/
sidebar:
  label: Etsy Listing Details Reviews
---

# etsy-listing-details-reviews

## JSON
- File: `workflows/etsy/etsy-listing-details-reviews.json`
- Workflow id: `etsy_listing_details_reviews`

## Input Parameters
- `listing_url`: Full Etsy listing URL.

## Behavior
- Opens listing page.
- Extracts listing-level fields (title, price, shop, rating, counts, description).
- Extracts listing images (`src`/`srcset`/`alt`).
- Attempts multi-pass review extraction with scroll and next-page click.

## Run
- `./skills/etsy-listing-details-reviews/scripts/run.sh --listing-url "https://www.etsy.com/in-en/listing/4371662429/custom-genuine-leather-wallet"`
