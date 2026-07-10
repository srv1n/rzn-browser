---
name: "goodreads-reviews"
description: "Run the Goodreads review-collection workflow (by-rating coverage) through the local CLI route."
---

# Goodreads Reviews Skill

Collect a book's community reviews. `--coverage by_rating` walks every star bucket
(1-5) and dedupes, capturing both praise and criticism — the recommended mode for
opinion synthesis:

```bash
# First page only (~30 reviews)
./skills/goodreads-reviews/scripts/run.sh --book-url "https://www.goodreads.com/book/show/29993569-raising-a-secure-child"

# Full opinion spectrum incl. critical reviews (login-free)
./skills/goodreads-reviews/scripts/run.sh --book-url "<url>" --coverage "by_rating"

# Signed in to Goodreads: walk deeper within each bucket
./skills/goodreads-reviews/scripts/run.sh --book-url "<url>" --coverage "by_rating" --max-clicks "10"
```

Output is a normalized JSON envelope with `success`, `row_count`, and `data`.
`data.reviews` holds `{ author, rating, date, likes, body, review_url }`.

Notes:
- `by_rating` works logged out (clicks histogram star buckets; no sign-up wall).
- The result bridge caps a returned array at ~50, so `by_rating` returns a balanced
  sample of up to 10 reviews per star (critical first). `data.reviews_loaded` reports
  the true count seen; `data.reviews_returned` the count in the array.
- `--max-clicks` ("Show more reviews") only loads more when the operator's Chrome is
  signed in to Goodreads; it is a no-op logged out.
