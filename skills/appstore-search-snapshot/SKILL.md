---
name: appstore-search-snapshot
description: Handle App Store search snapshot requests, distinguishing current listing data from screenshot evidence.
---

# App Store Search Snapshot

The former snapshot workflow is no longer shipped. Current listing extraction
uses `appstore/search` with the installed `rzn-browser` CLI and its connected
Chrome extension/native host:

```bash
rzn-browser run appstore/search --param term="budget app" --param country="us"
```

Returns app names, URLs, IDs, developer/subtitle, and card text. For ordinary
listing searches, use the appstore-search skill.

If the user requested a screenshot, listing JSON alone is incomplete. Use an
available browser capture capability and verify the resulting image, or report
that capture is unavailable. Do not label DOM data or a successful extraction
as screenshot evidence.
