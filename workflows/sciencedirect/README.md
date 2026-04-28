# ScienceDirect Workflows

Browser automation workflows for ScienceDirect academic search and article metadata.

## Workflows

### `sciencedirect-search.json`
Search ScienceDirect and extract first-page result metadata.

- **Parameter:** `search_query` — free-text query (aliases: `query`, `q`).
- **Returns (per item):** `title`, `url` (relative PII path), `authors`, `journal`, `date`, `article_type`, `access_type` (absent on paywalled items), `doi`.
- **Note:** Abstracts are not rendered on the SERP. Use `sciencedirect-paper-access` for full abstract and PDF link.

```bash
./target/release/rzn-browser run workflows/sciencedirect/sciencedirect-search.json \
  --param search_query="machine learning"
```

### `sciencedirect-paper-access.json`
Extract metadata, abstract, PDF link, and authors from a specific article page.

- **Parameter:** `paper_url` — absolute ScienceDirect article URL (e.g. `https://www.sciencedirect.com/science/article/pii/<PII>`). Pre-resolve DOIs to a ScienceDirect URL.
- **Returns:** `{ title, journal, citation, doi, abstract, pdf_url }` plus an authors array.
- **Note:** `pdf_url` points to the `/pdfft` endpoint; on paywalled articles this redirects to an entitlement page rather than a PDF.

```bash
./target/release/rzn-browser run workflows/sciencedirect/sciencedirect-paper-access.json \
  --param paper_url="https://www.sciencedirect.com/science/article/pii/S2666389920301896"
```

## Notes

- The cookie banner that appears at page bottom is a z-indexed overlay; it does **not** block DOM extraction because the result/article elements are present in the DOM beneath it.
- Neither workflow handles institutional login. Paywalled content returns metadata only.
- Run `rzn-browser workflow validate <path>` before committing changes — both workflows must pass with zero errors.
