---
name: goodreads-reviews
description: Collect a Goodreads book's reader reviews, including samples across star ratings, using RZN Browser.
---

# Goodreads Reviews

Requires `rzn-browser` and its connected Chrome extension/native host.

```bash
rzn-browser run goodreads/reviews --param book_url="<book-url>" --param coverage=by_rating
```

Use `coverage=page` for only the initial review page. `by_rating` visits each
star bucket and deduplicates; it supports public pages without signing in.
Add `--param max_clicks=10` only for deeper loading in a signed-in session.

The result includes `reviews`, `facets`, `reviews_loaded`, and
`reviews_returned`. The workflow bounds its returned sample to about ten reviews
per star to fit the bridge's array limit. Report actual bucket counts and missing
coverage; loaded reviews and returned reviews are different measures.

Keep review URLs when attributing praise or criticism. This balanced sample is
not a population distribution; use the book's rating histogram for percentages.
