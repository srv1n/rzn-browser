---
name: goodreads-search
description: Find Goodreads books by title, author, ISBN, or keyword using RZN Browser.
---

# Goodreads Search

Requires the installed `rzn-browser` CLI and connected Chrome extension/native
host. Use the user's existing browser profile.

```bash
rzn-browser run goodreads/search --param search_query="raising a secure child"
```

Returns `results` with title, author, book URL, rating, and ratings count. Verify the intended title/author/edition before using a returned URL for book details, reviews, or similar books.

Check the actual output and source URLs before summarizing. If extraction fails
or returns incomplete data, report the limitation. Use `rzn-browser workflow list goodreads search`
for the installed workflow's parameters and result contract.
