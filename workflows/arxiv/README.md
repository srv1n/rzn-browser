# arXiv Workflow Documentation

This directory contains workflows for extracting machine-readable HTML versions and metadata from arXiv papers.

## Workflows

### 1. arxiv-html.json
Extracts metadata and HTML link from a single arXiv paper.

**Parameters:**
- `arxiv_id`: The arXiv paper ID (e.g., "2301.00234")

**Extracted Data:**
- Title
- Authors
- Abstract
- arXiv ID
- Categories
- PDF link
- HTML link (if available)
- Source link

**Example:**
```bash
./test-arxiv.sh 2301.00234
```

### 2. arxiv-search.json
Searches arXiv and extracts results with metadata.

**Parameters:**
- `search_query`: Search terms for arXiv

**Extracted Data:**
- Title
- Authors
- Abstract
- arXiv ID
- Paper URL
- PDF link
- Categories

**Example:**
```bash
./test-arxiv.sh "machine learning"
```

## Important Notes

1. **HTML Availability**: Not all arXiv papers have HTML versions. The workflow gracefully handles papers that only have PDF versions by marking the `html_link` field as optional.

2. **HTML Versions**: arXiv provides HTML versions through their LaTeXML conversion system. Papers with HTML versions will have a link like `https://arxiv.org/html/XXXX.XXXXXvX`.

3. **ar5iv Alternative**: For papers without official HTML versions, you can try ar5iv by replacing "arxiv.org" with "ar5iv.labs.arxiv.org" in the URL.

## Testing

Use the provided test script:

```bash
# Test single paper extraction
./test-arxiv.sh 2301.00234

# Test search functionality
./test-arxiv.sh "quantum computing"
```

## Common arXiv IDs for Testing

Papers likely to have HTML versions:
- `2301.00234` - A Survey on In-context Learning
- `2402.08954` - HTML papers on arXiv (meta paper about HTML support)
- Recent CS papers (2023-2025) are more likely to have HTML versions