---
name: "apple-ads-keyword-recs"
description: "Run Apple Ads portal keyword recommendations workflow and output a normalized JSON envelope."
---

# Apple Ads Keyword Recs Skill

Run Apple Ads keyword recommendations (requires a logged-in Apple Ads portal session):

```bash
./skills/apple-ads-keyword-recs/scripts/run.sh --adam-id "123456789" --adgroup-id "987654321" --query "budget planner" --storefront "us"
```

Output is a normalized JSON envelope with `success`, `row_count`, `params`, and `data`.
