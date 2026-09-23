---
name: etsy-listing-details-reviews
description: Extract reviews for a specific Etsy listing using RZN Browser.
---

# Etsy Listing Reviews

Requires the installed `rzn-browser` CLI and connected Chrome extension/native
host. Use the user's existing browser profile.

```bash
rzn-browser run etsy/listing_details_reviews --param listing_url="<listing-url>"
```

Use a listing URL supplied by the user or returned by Etsy search. The declared final result is review rows with body, author, date, and rating; it is not a combined listing-details object.

Check the actual output and source URLs before summarizing. If extraction fails
or returns incomplete data, report the limitation. Use `rzn-browser workflow list etsy listing_details_reviews`
for the installed workflow's parameters and result contract.
