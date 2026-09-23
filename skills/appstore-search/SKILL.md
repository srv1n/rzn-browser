---
name: appstore-search
description: Search App Store listings by keyword and storefront using RZN Browser.
---

# App Store Search

Requires the installed `rzn-browser` CLI and connected Chrome extension/native
host. Use the user's existing browser profile.

```bash
rzn-browser run appstore/search --param term="notion" --param country="us"
```

Returns app name, URL, app ID, developer/subtitle, and card text. Use `term`, not the retired `app_query` parameter. For one app's reviews, use `appstore/app_details`.

Check the actual output and source URLs before summarizing. If extraction fails
or returns incomplete data, report the limitation. Use `rzn-browser workflow list appstore search`
for the installed workflow's parameters and result contract.
