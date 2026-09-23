---
name: goodreads-book
description: Look up one Goodreads book's metadata, rating histogram, and initial reviews using RZN Browser.
---

# Goodreads Book

Requires the installed `rzn-browser` CLI and connected Chrome extension/native
host. Use the user's existing browser profile.

```bash
rzn-browser run goodreads/book --param book_url="<book-url>"
```

Returns `book`, `rating_breakdown`, `genres`, initial `reviews`, and `similar_urls`. Use `goodreads/reviews` with `coverage=by_rating` for a spread of opinions; the initial page is not representative of all readers.

Check the actual output and source URLs before summarizing. If extraction fails
or returns incomplete data, report the limitation. Use `rzn-browser workflow list goodreads book`
for the installed workflow's parameters and result contract.
