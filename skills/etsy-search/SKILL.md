---
name: etsy-search
description: Search Etsy listings by keyword using RZN Browser.
---

# Etsy Search

Requires the installed `rzn-browser` CLI and connected Chrome extension/native
host. Use the user's existing browser profile.

```bash
rzn-browser run etsy/search --param search_query="leather wallet"
```

Returns listing title, URL, displayed price, shop name, and rating summary. Use the returned listing URL for reviews. Search extraction does not purchase or contact sellers.

Check the actual output and source URLs before summarizing. If extraction fails
or returns incomplete data, report the limitation. Use `rzn-browser workflow list etsy search`
for the installed workflow's parameters and result contract.
