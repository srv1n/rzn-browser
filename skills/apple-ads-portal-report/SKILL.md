---
name: "apple-ads-portal-report"
description: "Run Apple Ads portal report fallback workflow and output a normalized JSON envelope."
---

# Apple Ads Portal Report Skill

Run Apple Ads fallback report extraction (requires a logged-in Apple Ads portal session):

```bash
./skills/apple-ads-portal-report/scripts/run.sh --report-type "campaigns" --start-date "2026-02-01" --end-date "2026-02-15" --organization-id "11111111"
```

Output is a normalized JSON envelope with `success`, `row_count`, `params`, and `data`.
