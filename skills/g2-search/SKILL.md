---
name: "g2-search"
description: "Run G2 search extraction workflow through the local CLI route."
---

# G2 Search Skill

Run G2 search with no environment setup:

```bash
./skills/g2-search/scripts/run.sh --query "project management"
```

Output is a normalized JSON envelope with `success`, `row_count`, and `data`.

