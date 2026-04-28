---
name: "amazon-product"
description: "Run Amazon product key-facts + multi-page reviews extraction through the local CLI route."
---

# Amazon Product Skill

Run Amazon product extraction with no environment setup:

```bash
./skills/amazon-product/scripts/run.sh --product-url "https://www.amazon.com/dp/B07FZ8S74R"
```

Output is a normalized JSON envelope with `success`, `row_count`, and `data`.

