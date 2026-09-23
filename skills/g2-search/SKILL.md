---
name: g2-search
description: Find G2 software products by keyword or product name using RZN Browser.
---

# G2 Search

Requires the installed `rzn-browser` CLI and connected Chrome extension/native
host. Use the user's existing browser profile.

```bash
rzn-browser run g2/search --param search_query="project management"
```

Returns `results` with product names, URLs, ratings, and review counts. Broad category terms can redirect to a category page; inspect the returned surface before treating an empty result as no matching products.

Check the actual output and source URLs before summarizing. If extraction fails
or returns incomplete data, report the limitation. Use `rzn-browser workflow list g2/search`
for the installed workflow's parameters and result contract.
