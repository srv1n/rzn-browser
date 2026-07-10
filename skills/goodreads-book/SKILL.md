---
name: "goodreads-book"
description: "Run the Goodreads book details + reviews workflow through the local CLI route."
---

# Goodreads Book Skill

Fetch one book's metadata, rating histogram, genres, first page of reviews, and
similar-book URLs:

```bash
./skills/goodreads-book/scripts/run.sh --book-url "https://www.goodreads.com/book/show/29993569-raising-a-secure-child"
```

Output is a normalized JSON envelope with `success`, `row_count`, and `data`.
`data` holds `{ book{...}, rating_breakdown[], genres[], reviews[], similar_urls[] }`.
For the full critical-vs-positive review spread use goodreads-reviews; for a titled
recommendation list use goodreads-similar.
