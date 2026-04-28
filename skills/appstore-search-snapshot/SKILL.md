---
name: "appstore-search-snapshot"
description: "Run one-off App Store web snapshot workflow (top fold + screenshot) and output a normalized JSON envelope."
---

# App Store Search Snapshot Skill

Run one-off App Store search snapshot extraction:

```bash
./skills/appstore-search-snapshot/scripts/run.sh --term "budget app" --country "us"
```

Output is a normalized JSON envelope with `success`, `row_count`, `params`, and `data`.
