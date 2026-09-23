---
name: appstore-details
description: Extract App Store reviews for an app ID or storefront URL using RZN Browser.
---

# App Store Details

Requires the installed `rzn-browser` CLI and connected Chrome extension/native
host. Use the user's existing browser profile.

```bash
rzn-browser run appstore/app_details --param app_id="1232780281"
```

Pass either `app_id` (US storefront) or a full `app_url` for another storefront. The declared final result is review rows; ratings and screenshots observed in other steps are not guaranteed fields in that result.

Check the actual output and source URLs before summarizing. If extraction fails
or returns incomplete data, report the limitation. Use `rzn-browser workflow list appstore app_details`
for the installed workflow's parameters and result contract.
