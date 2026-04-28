---
name: "capterra-product-details-reviews"
description: "Run Capterra product details + reviews extraction through the local CLI route."
---

# Capterra Product Details Reviews Skill

Run Capterra product details and reviews with no environment setup:

```bash
./skills/capterra-product-details-reviews/scripts/run.sh --product-url "https://www.capterra.com/p/12345/Example-Product/"
```

Output is a normalized JSON envelope with `success`, `row_count`, and `data`.

