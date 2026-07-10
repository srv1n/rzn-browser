# Goodreads Workflows

Read-only browser workflows for discovering books, reading their reviews, and
pulling recommendations from [Goodreads](https://www.goodreads.com).

All four are pure reads. None sign in, post, shelve, rate, or mutate any
account state.

## Workflows

| Ref | What it does | Key output |
|---|---|---|
| `goodreads/shelf` | Popularity-ranked top books on a shelf/genre, paginated. The discovery entry point for "top books in a field". | `books: [{ title, author, book_url, avg_rating, ratings_count, published }]` |
| `goodreads/search` | Drives the real navbar search box and reads the results table. | `results: [{ title, book_url, author, avg_rating, ratings_count }]` |
| `goodreads/book` | Book metadata from the schema.org `ld+json` block + DOM, the 5-star histogram, genres, the first page of reviews, and similar-book URLs. | `book{…}, rating_breakdown[], genres[], reviews[], similar_urls[]` |
| `goodreads/reviews` | The dedicated `<book>/reviews` page; collects reviews. `coverage=by_rating` walks every star bucket (5..1) to capture the full opinion spectrum incl. critical reviews; `max_clicks` paginates deeper when signed in. | `reviews: [{ author, rating, date, likes, body, review_url }], facets[]` |
| `goodreads/similar` | The full `Readers also enjoyed` recommendation list seeded from a book. | `recommendations: [{ title, author, avg_rating, ratings_count, book_url }]` |

## Discovery → opinion pipeline

These are the **data primitives** an agent composes to go from a topic to a
ranked, opinion-bearing reading list:

1. `goodreads/shelf` (or `goodreads/search`) → top books in the field.
2. `goodreads/book` per candidate → metadata + rating distribution + genres.
3. `goodreads/reviews --param coverage=by_rating` per candidate → praise **and**
   criticism (e.g. "grounded in debunked left-brain/right-brain science").
4. The agent synthesizes an opinion, shortlists, and emits a table/spreadsheet.

Steps 1–3 are these workflows; step 4 is agent reasoning over their JSON output.
Acquiring book files (EPUB) is **not** a Goodreads capability — Goodreads only
links out to retailers/libraries — so it is out of scope here.

## Typical flow

```bash
# 1. Find the book and grab its canonical URL.
rzn-browser run goodreads search --param search_query="raising a secure child"

# 2. Details + first page of reviews + similar URLs.
rzn-browser run goodreads book \
  --param book_url="https://www.goodreads.com/book/show/29993569-raising-a-secure-child"

# 3. Recommendations seeded from that book.
rzn-browser run goodreads similar \
  --param book_url="https://www.goodreads.com/book/show/29993569-raising-a-secure-child"

# 4. Collect reviews (first page logged out; deeper when signed in).
rzn-browser run goodreads reviews \
  --param book_url="https://www.goodreads.com/book/show/29993569-raising-a-secure-child" \
  --param max_clicks="5"
```

`book_url` is always the canonical `/book/show/<id>-slug` URL that
`goodreads/search` returns. Query strings and `#anchors` are stripped
automatically.

## Login caveat (important)

Goodreads gates **deep review browsing behind a sign-in wall**. Logged out:

- A book page and the `/reviews` page each server-render ~10–30 reviews.
- The first `Show more reviews` click opens a "Discover & Read More" sign-up
  modal and loads **no further reviews**, so `goodreads/reviews` returns the
  first page regardless of `max_clicks`.

When the operator's Chrome profile is **already signed in to Goodreads**, each
`Show more reviews` click loads ~30 more, so raising `max_clicks` walks deeper.
`total_count` always reports Goodreads' full text-review count for reference.

Personalized account recommendations (the `/recommendations` page) likewise
require login and are out of scope. `goodreads/similar` is the read-only,
logged-out-safe recommendation surface (recommendations seeded from a book).

## Implementation notes

- Metadata prefers the schema.org `ld+json` `Book` block, falling back to
  `data-testid` DOM nodes.
- The book/review pages hydrate review cards client-side; the extractors poll
  and scroll briefly so reviews are present before reading.
- `goodreads/similar` resolves the `/book/similar/<work_id>` link from the book
  page and fetches that server-rendered page same-origin — the `work_id`
  differs from the book id, so it can't be derived from the URL alone.
- `rating` is `N/A` on review cards where the reviewer wrote text but left no
  star rating.
- No CDP required; all JS runs in the page's main world.
```
