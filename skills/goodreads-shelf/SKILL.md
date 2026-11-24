---
name: goodreads-shelf
description: Discover popularity-ranked books on a Goodreads shelf or genre using RZN Browser.
---

# Goodreads Shelf

Requires `rzn-browser` and its connected Chrome extension/native host.

```bash
rzn-browser run goodreads/shelf --param shelf="parenting" --param start_page=1
```

Use the slug from `/shelf/show/<slug>`. The result includes `books`,
`count`, and `page`; each book has title, author, URL, rating, ratings
count, and publication information. Shelf popularity is not a quality ranking.

Each call reads one page of fifty books. For more, increment `start_page`
(1–25), one call per page. Deduplicate by book URL and report the pages and
rows actually returned.
