---
name: amazon-product
description: Extract Amazon reviews for a specific product URL using RZN Browser.
---

# Amazon Product

Requires the installed `rzn-browser` CLI and connected Chrome extension/native
host. Use the user's existing browser profile.

```bash
rzn-browser run amazon/product_key_facts_reviews --param product_url="https://www.amazon.com/dp/B07FZ8S74R"
```

The declared final result is review rows (title, rating, date, body, verified-purchase label, helpful votes). Product facts may appear in intermediate steps; do not promise a combined product-facts object or complete review coverage.

Check the actual output and source URLs before summarizing. If extraction fails
or returns incomplete data, report the limitation. Use `rzn-browser workflow list amazon product_key_facts_reviews`
for the installed workflow's parameters and result contract.
