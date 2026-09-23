---
name: goodreads-similar
description: Find Goodreads Readers also enjoyed recommendations for a specific book using RZN Browser.
---

# Goodreads Similar Books

Requires the installed `rzn-browser` CLI and connected Chrome extension/native
host. Use the user's existing browser profile.

```bash
rzn-browser run goodreads/similar --param book_url="<book-url>"
```

Returns `recommendations` with title, author, rating, ratings count, and book URL. This public, book-seeded surface differs from personalized account recommendations; report only the entries actually returned.

Check the actual output and source URLs before summarizing. If extraction fails
or returns incomplete data, report the limitation. Use `rzn-browser workflow list goodreads similar`
for the installed workflow's parameters and result contract.
