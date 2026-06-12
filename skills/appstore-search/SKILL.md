---
name: "appstore-search"
description: "Run App Store keyword search extraction workflow through the local CLI route."
---

# App Store Search Skill

Run App Store search with no environment setup:

```bash
./skills/appstore-search/scripts/run.sh --query "notion"
```

Output is a normalized JSON envelope with `success`, `row_count`, and `data`.

