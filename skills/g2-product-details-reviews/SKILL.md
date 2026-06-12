---
name: "g2-product-details-reviews"
description: "Run G2 product details + reviews extraction through the local CLI route."
---

# G2 Product Details Reviews Skill

Run G2 product details and reviews with no environment setup:

```bash
./skills/g2-product-details-reviews/scripts/run.sh --product-url "https://www.g2.com/products/notion/reviews"
```

Output is a normalized JSON envelope with `success`, `row_count`, and `data`.

