---
name: "amazon-search"
description: "Run Amazon search extraction workflow through the local CLI->native-host->extension route."
---

# Amazon Search Skill

Run Amazon search with no environment setup:

```bash
./skills/amazon-search/scripts/run.sh --query "wireless mouse"
```

Output is a normalized JSON envelope with `success`, `row_count`, and `data`.
