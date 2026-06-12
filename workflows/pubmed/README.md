# PubMed Workflows

This directory contains workflows for automating PubMed searches and paper information extraction.

## Available Workflows

### 1. PubMed Search (`pubmed-search.json`)

Searches PubMed for papers and extracts metadata from search results.

**Parameters:**
- `search_query` - The search term (e.g., "COVID-19 vaccines", "machine learning medicine")

**Extracted Data:**
- Paper title
- Authors
- Journal name
- Publication date
- PMID (PubMed ID)
- Abstract snippet
- URL to full paper details
- Total number of search results

**Example:**
```bash
rzn-browser run workflows/pubmed/pubmed-search.json --param search_query="CRISPR gene therapy"
```

### 2. PubMed Extract (`pubmed-extract.json`)

Extracts comprehensive information from a specific PubMed paper.

**Parameters:**
- `pmid` - The PubMed ID (e.g., "35216673") or full URL

**Extracted Data:**
- Full title
- Authors with affiliations
- Complete abstract
- Keywords/MeSH terms
- Journal, volume, pages
- Publication date
- DOI
- PMC ID (if available)
- Full text links (PMC, publisher)
- Citation count

**Example:**
```bash
# Using PMID
rzn-browser run workflows/pubmed/pubmed-extract.json --param pmid="35216673"

# Using full URL
rzn-browser run workflows/pubmed/pubmed-extract.json --param pmid="https://pubmed.ncbi.nlm.nih.gov/35216673"
```

## Testing Examples

### Medical Search Terms
- "COVID-19 vaccines clinical trials"
- "CRISPR Cas9 therapeutic applications"
- "machine learning radiology diagnosis"
- "alzheimer disease prevention"
- "cancer immunotherapy checkpoint"

### Scientific Search Terms
- "quantum computing applications"
- "microbiome gut brain axis"
- "climate change health impacts"
- "artificial intelligence drug discovery"
- "stem cell regenerative medicine"

## Notes

1. **Rate Limiting**: PubMed generally doesn't have strict rate limits for metadata access, but be respectful with request frequency.

2. **Full Text Access**: While PubMed provides metadata freely, full text access depends on:
   - PMC (PubMed Central) - Free full text for open access papers
   - Publisher sites - May require institutional access or payment

3. **Selector Updates**: PubMed occasionally updates their HTML structure. If extraction fails, check the selectors using browser DevTools.

4. **Large Result Sets**: The search workflow extracts papers visible on the first page. For comprehensive searches, consider:
   - Using more specific search queries
   - Implementing pagination support
   - Using PubMed's advanced search filters

## Debugging

If workflows fail to extract data:

1. Check browser console for errors:
   ```bash
   RUST_LOG=debug rzn-browser run workflows/pubmed/pubmed-search.json --param search_query="test"
   ```

2. Verify selectors are still valid by inspecting PubMed's HTML

3. Check logs:
   ```bash
   tail -f ~/rzn_build.log | jq .
   ```