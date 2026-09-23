---
name: capterra-product-details-reviews
description: Extract a Capterra product profile and visible reviews using RZN Browser.
---

# Capterra Product and Reviews

Requires the installed `rzn-browser` CLI and connected Chrome extension/native
host. Use the user's existing browser profile.

```bash
rzn-browser run capterra/product_details_reviews --param product_url="<product-url>"
```

Use a canonical product URL from Capterra search. Returns `product`, `reviews`, and `reviews_returned`; distinguish the collected sample from the product's total review count.

Check the actual output and source URLs before summarizing. If extraction fails
or returns incomplete data, report the limitation. Use `rzn-browser workflow list capterra product_details_reviews`
for the installed workflow's parameters and result contract.
