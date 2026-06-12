---
name: "appstore-details"
description: "Run App Store app details extraction (ratings, screenshots, reviews) through the local CLI route."
---

# App Store Details Skill

Run App Store app details with no environment setup:

```bash
./skills/appstore-details/scripts/run.sh --app-id "1232780281"
```

Output is a normalized JSON envelope with `success`, `row_count`, and `data`.

