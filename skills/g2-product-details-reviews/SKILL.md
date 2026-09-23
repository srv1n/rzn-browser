---
name: g2-product-details-reviews
description: Extract a G2 product profile and a bounded sample of reviews using RZN Browser.
---

# G2 Product and Reviews

Requires the installed `rzn-browser` CLI and connected Chrome extension/native
host. Use the user's existing browser profile.

```bash
rzn-browser run g2/product_details_reviews --param product_url="https://www.g2.com/products/notion/reviews" --param max_pages=1
```

Returns `product`, `reviews`, and `pages_walked`. `max_pages` accepts 1–5; report pages actually walked and reviews returned. Use the canonical product URL from G2 search.

Check the actual output and source URLs before summarizing. If extraction fails
or returns incomplete data, report the limitation. Use `rzn-browser workflow list g2/product_details_reviews`
for the installed workflow's parameters and result contract.
