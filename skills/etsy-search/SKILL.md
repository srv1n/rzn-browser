---
name: "etsy-search"
description: "Run Etsy listing search extraction workflow through the local CLI route."
---

# Etsy Search Skill

Run Etsy listing search with no environment setup:

```bash
./skills/etsy-search/scripts/run.sh --query "leather wallet"
```

Output is a normalized JSON envelope with `success`, `row_count`, and `data`.

