---
name: "etsy-listing-details-reviews"
description: "Run Etsy listing details + reviews extraction through the local CLI route."
---

# Etsy Listing Details Reviews Skill

Run Etsy listing details and reviews with no environment setup:

```bash
./skills/etsy-listing-details-reviews/scripts/run.sh --listing-url "https://www.etsy.com/listing/123456789/example-listing"
```

Output is a normalized JSON envelope with `success`, `row_count`, and `data`.

