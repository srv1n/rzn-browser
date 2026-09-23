---
name: amazon-search
description: Search Amazon product listings by keyword using RZN Browser.
---

# Amazon Search

Requires the installed `rzn-browser` CLI and connected Chrome extension/native
host. Use the user's existing browser profile.

```bash
rzn-browser run amazon/search --param search_query="wireless mouse"
```

Returns listing rows with title, product URL, ASIN, displayed price, rating, and review count. Use a returned product URL for product review extraction; search results are not a complete market inventory.

Check the actual output and source URLs before summarizing. If extraction fails
or returns incomplete data, report the limitation. Use `rzn-browser workflow list amazon search`
for the installed workflow's parameters and result contract.
