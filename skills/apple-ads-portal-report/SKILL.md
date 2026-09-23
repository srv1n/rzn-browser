---
name: apple-ads-portal-report
description: List Apple Ads campaigns or ad groups from the signed-in portal. This workflow does not return performance metrics.
---

# Apple Ads Portal Listings

Requires `rzn-browser`, its connected Chrome extension/native host, and the
user's authenticated Apple Ads profile.

```bash
rzn-browser run apple_ads/portal_report --param org_id="<org-id>"
# Add --param campaign_id="<campaign-id>" to list that campaign's ad groups.
```

Use the intended organization's ID; omission uses whichever organization the
portal opens. Returns `name`, `href`, and campaign/ad-group IDs. Deduplicate by
entity ID because the portal repeats links for metric cells; retain the row with
the entity name.

The legacy skill name is retained, but the workflow lists entities. It accepts
neither report-type nor date-range parameters and does not return spend,
impressions, taps, or installs. If metrics were requested, explain that coverage
gap and identify an appropriate reporting route before proceeding.
