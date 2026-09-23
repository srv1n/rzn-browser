---
name: goodreads-shelf
description: Discover popularity-ranked books on a Goodreads shelf or genre using RZN Browser.
---

# Goodreads Shelf

Requires `rzn-browser` and its connected Chrome extension/native host.

```bash
rzn-browser run goodreads/shelf --param shelf="parenting" --param max_pages=1 --param start_page=1
```

Use the slug from `/shelf/show/<slug>`. The result includes `books`,
`count`, and `pages_walked`; each book has title, author, URL, rating, ratings
count, and publication information. Shelf popularity is not a quality ranking.

For more than fifty books, request one page per call with `max_pages=1` and
increment `start_page` (1–25). A single multi-page run may lose rows to the
bridge's roughly fifty-item array cap. Deduplicate by book URL and report the
pages and rows actually returned.
