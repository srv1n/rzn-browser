---
name: "goodreads-search"
description: "Run the Goodreads book-search workflow through the local CLI route."
---

# Goodreads Search Skill

Search Goodreads by title/author/keyword with no environment setup:

```bash
./skills/goodreads-search/scripts/run.sh --query "raising a secure child"
```

Output is a normalized JSON envelope with `success`, `row_count`, and `data`.
`data.results` holds `{ title, book_url, author, avg_rating, ratings_count }`.
Feed `book_url` into goodreads-book, goodreads-reviews, or goodreads-similar.
