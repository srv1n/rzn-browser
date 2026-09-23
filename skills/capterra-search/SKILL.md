---
name: capterra-search
description: Find Capterra software products by keyword or name using RZN Browser.
---

# Capterra Search

Requires the installed `rzn-browser` CLI and connected Chrome extension/native
host. Use the user's existing browser profile.

```bash
rzn-browser run capterra/search --param search_query="crm"
```

Returns `query`, `count`, and `results` with product IDs, names, canonical review URLs, ratings, and review counts. Carry the returned URL into the product-details workflow.

Check the actual output and source URLs before summarizing. If extraction fails
or returns incomplete data, report the limitation. Use `rzn-browser workflow list capterra search`
for the installed workflow's parameters and result contract.
