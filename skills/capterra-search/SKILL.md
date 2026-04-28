---
name: "capterra-search"
description: "Run Capterra search extraction workflow through the local CLI route."
---

# Capterra Search Skill

Run Capterra search with no environment setup:

```bash
./skills/capterra-search/scripts/run.sh --query "crm"
```

Output is a normalized JSON envelope with `success`, `row_count`, and `data`.

