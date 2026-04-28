# Porkbun Domain Availability Workflow

## Overview
This feature adds a workflow JSON that opens Porkbun domain search results for a requested `domain_name`, waits for rows to resolve from pending state, and extracts machine-readable fields for all visible result rows without hard-coding per-domain rules.

## Flow Diagrams
### End-to-End
`rzn-browser run-workflow` -> extension executes steps -> Porkbun search page loads -> async availability checks resolve -> extractor returns all visible domain rows.

### Internal Flow
Navigate -> wait input/button -> fill input -> click search button -> wait results container + row shell -> wait rows to leave `.pendingDomain` -> settle wait -> extract row metadata and visible text.

## Decision Record
- Chosen: homepage navigation + `fill_input_field` + `click_element` on `#domainSearchButton`.
  - Why: `submit_input` can type without reliably submitting on Porkbun; explicit button click is deterministic.
- Chosen: row readiness detection via `.searchResultRowDomain:not(.pendingDomain)`.
  - Why: Porkbun renders row shells as pending first, then applies resolved classes/text asynchronously.
- Rejected: static HTML-only checks.
  - Why: result rows are populated asynchronously by page JS and can remain pending in initial markup.

## Architecture
- Workflow file: `workflows/generated/porkbun-domain-availability.json`.
- Input contract:
  - `domain_name` (required): fully qualified domain string.
- Output contract:
  - Extracted row fields from exact/suggested/trending rows:
    - `#searchResultsDomainContainer .searchResultRow[data-result-type='exact'|'suggested'|'trending']`
    - `domain`
    - `domain_css_class`
    - `price_text`
    - `actions_text`
    - `result_type`
    - `in_cart`
    - `row_id`

## Implementation Notes
- Uses `wait_for_element` for:
  - search input and button readiness,
  - results container and row shell presence,
  - resolved row state via `.searchResultRowDomain:not(.pendingDomain)`.
- Uses `fill_input_field` followed by `click_element` to trigger search from homepage context.
- Uses one short `wait_for_timeout` to reduce racing in options/price cells after status resolution.
- Uses `extract_structured_data` with `selector: "*"` for row-level attributes and scoped selectors for child content.

## Tasks & Status
- [x] Created generated workflow for Porkbun availability checks.
- [x] Added required variable and deterministic URL navigation.
- [x] Added dynamic status wait and extraction mapping.
- [ ] Validate live run in a local Chrome session with extension connected.

## What Works (Do Not Change)
- Primary targeting: exact/suggested/trending rows in `#searchResultsDomainContainer`.
- Status readiness detection: `.searchResultRowDomain:not(.pendingDomain)`.
- Homepage search input selector: `#domainSearchInput`.
- Homepage search button selector: `#domainSearchButton`.

## Tried & Didn’t Work
- Static source parsing for availability text.
  - Did not work because initial HTML commonly shows pending placeholders and loads final state asynchronously.
