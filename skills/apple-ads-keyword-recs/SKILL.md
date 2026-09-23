---
name: apple-ads-keyword-recs
description: Read Apple Ads recommendation cards or request keyword suggestions in a signed-in portal session.
---

# Apple Ads Recommendations

Requires `rzn-browser`, its connected Chrome extension/native host, and the
user's authenticated `app-ads.apple.com` profile. Choose the requested surface:

```bash
# Category summary cards for the signed-in organization
rzn-browser run apple_ads/keyword_recommendations

# Actual keyword suggestions in an app and ad-group context
rzn-browser run apple_ads/keyword_suggest --param org_id="<org-id>" --param campaign_id="<campaign-id>" --param adgroup_id="<adgroup-id>" --param adam_id="<app-id>" --param query="budget planner"
```

Recommendation cards contain category summaries, not individual keywords.
Their optional app, ad-group, query, and storefront parameters do not filter the
portal UI. For keyword suggestions, use verified IDs from the intended account;
the result normally contains `recommendations` with keyword and popularity data.

Both routes read data; they do not apply recommendations or change bids. Report
the selected organization/context and actual returned fields. An auth failure
requires reauthentication, not repeated retries or guessed IDs.
