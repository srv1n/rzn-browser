---
name: "goodreads-shelf"
description: "Run the Goodreads shelf/genre top-books workflow through the local CLI route."
---

# Goodreads Shelf Skill

List the popularity-ranked top books on a Goodreads shelf/genre:

```bash
./skills/goodreads-shelf/scripts/run.sh --shelf "parenting"
./skills/goodreads-shelf/scripts/run.sh --shelf "self-help" --max-pages "3"
```

`--shelf` is the slug from `/shelf/show/<slug>` (e.g. parenting, self-help,
philosophy, psychology). `--max-pages` (1-5, default 1) collects 50 books/page.

Output is a normalized JSON envelope with `success`, `row_count`, and `data`.
`data.books` holds `{ title, author, book_url, avg_rating, ratings_count, published }`,
ordered by shelf popularity. This is the discovery entry point for "top books in a
field"; feed `book_url` into goodreads-book / goodreads-reviews / goodreads-similar.
