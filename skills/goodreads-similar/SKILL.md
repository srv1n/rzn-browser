---
name: "goodreads-similar"
description: "Run the Goodreads similar-books (recommendations) workflow through the local CLI route."
---

# Goodreads Similar Skill

Get the full "Readers also enjoyed" recommendation list seeded from a book:

```bash
./skills/goodreads-similar/scripts/run.sh --book-url "https://www.goodreads.com/book/show/29993569-raising-a-secure-child"
```

Output is a normalized JSON envelope with `success`, `row_count`, and `data`.
`data.recommendations` holds `{ title, author, avg_rating, ratings_count, book_url }`.
This is the read-only, logged-out-safe recommendation surface (account-personalized
recommendations require login and are out of scope).
