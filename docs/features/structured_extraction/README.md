#+ Structured Extraction

## Overview
- Goal: Extract structured information (lists, fields) from arbitrary pages into compact JSON using a hybrid strategy: code-first via DOM, LLM for semantics, and LLM-guided selector discovery to minimize tokens and maximize speed/robustness.
- Constraints: MV3 extension, CSP-safe defaults, minimal permissions; cross-origin frames via CDP only when needed; unified logging; low token budgets; avoid shipping full HTML to LLM.

## Flow Diagrams
- End-to-end
```
CLI / Planner → Broker → Extension (SW) → Content Script → Page DOM
              ←        ←                 ←               ← JSON result
```

- Code-First (Selectors → JSON)
```
step(type=extract_structured_data, item_selector, fields)
  ↓ querySelectorAll(item_selector)
  ↓ per-item field selectors/attributes
  ↓ JSON [{...}, ...]
```

- LLM-Assisted Semantic (Scoped → JSON)
```
get_pruned_dom / get_dom_snapshot (scoped)
  ↓ LLM(prompt: instruction + schema + scope)
  ↓ JSON (validated/constrained)
```

- Hybrid Observe → Extract (LLM-guided selectors)
```
captureEnhancedDOMSnapshot (outline)
  ↓ LLM(prompt: “locate items/containers/selectors”)
  ↓ selectors/XPath/TargetSpec[]
  ↓ extract_structured_data (code) → JSON
  ↓ optional LLM post-formatting/summarization
```

## Decision Record
- Why hybrid: Deterministic DOM extraction is fastest and free (tokens), but brittle for unknown layouts. LLM brings semantic resilience. Guiding code with LLM-selected scopes/selectors minimizes context and cost.
- Alternatives considered: (a) Always-LLM parsing of full HTML (too slow/expensive, brittle to noise); (b) Pure scraping (brittle across sites). Hybrid provides speed with adaptability.
- Tradeoffs: Adds selector verification and small model calls; requires careful prompt design and caching to keep cost negligible.

## Architecture
- Modules
  - `extension/src/content/actions-enhanced.ts`: enhanced `extract_structured_data` (fields, optional `extraction_type`); code-first extraction.
  - `extension/src/contentScript.ts`: message router; handlers for `get_pruned_dom`, `get_dom_snapshot`, legacy/ enhanced extraction; Playwright test bridge.
  - `extension/src/content/dom-capture.ts`: DOM outline/hash for compact snapshots; used by `captureEnhancedDOMSnapshot`.
  - `extension/src/types/extractionPlan.ts`: validated `ExtractionPlan` schema (Zod) for deterministic extraction without arbitrary JS.
  - `schema/extraction-plan-v1.json`: JSON schema for non-TS callers/tests.
  - `extension/src/content/intelligent-dom.ts`: minimal, semantic page context and pattern detection to reduce LLM context.
  - `crates/rzn_plan/*`: planner/orchestrator selects strategy (code/LLM/hybrid) and composes steps.
  - `rzn_broker/*`: transport of length‑prefixed JSON between CLI⇄extension.

- Data contracts (existing)
  - `execute_step` with `step.type` in `rzn_core::StepKind` and extension handlers.
  - `get_pruned_dom` | `get_dom_snapshot`: returns `{ dom_snapshot, dom_hash, element_map }` where `dom_snapshot` is a compact outline.
  - `extract_structured_data` (legacy): `{ item_selector, fields:[{name, selector, attribute?}], extraction_type? }` → `Array<Object>`.
  - `extract_structured_data_enhanced` (preferred): `{ target_spec?, fields:[{ name, selector? | target_spec?, attribute? }], extraction_type? }` → `Object | Array<Object>`.
  - `execute_extraction_plan` (preferred for desktop): `{ plan }` → `JSON + provenance` (plan version, rung used, dom_hash).

- Data contracts (observe)
  - Request: `{ cmd: 'observe', instruction: string, max_items?: number, scope_selector?: string, dom_opts?: { maxElements?: number } }`
  - Response: `{ candidates: Array<{ selector?: string, xpath?: string, target_spec?: any, kind: 'item'|'list'|'value'|'table', score: number, reason: string, sample_text?: string }> }`
  - Use: Planner calls observe; then issues code extraction using best candidate(s). If empty/low score, planner falls back to semantic extraction.

## Implementation Notes
- Entry points
  - Background: `onMessage { cmd: 'execute_step' | 'observe' | 'execute_extraction_plan' | 'get_pruned_dom' | 'get_dom_snapshot' }` → content script.
  - Content script: `captureEnhancedDOMSnapshot(opts)`, `observe`, `execute_extraction_plan`, `extract_structured_data[_enhanced]`.
  - Page bridge: `window.__rznExecuteStep(step)` and `window.captureEnhancedDOMSnapshot(opts)` for tests/e2e.

- Strategy selection (planner policy)
  - Try deterministic: If the workflow/prompt supplies a validated extraction plan, run `execute_extraction_plan`.
  - Try code-first generic: If the workflow/prompt supplies selectors/item schema, run `extract_structured_data` over DOM.
  - Try hybrid: Call observe (heuristic baseline today; LLM-guided later) to get list/container selectors → run code extraction.
  - Semantic fallback: If selectors fail or page is highly unstructured, call LLM with snapshot text and schema for direct JSON.

- DOM outline/snapshot
  - Use `captureEnhancedDOMSnapshot({ maxElements })` for compact, hierarchical text with `elements`, `prompt`, `hash`.
  - Prefer scoping (limit to main content or suspected container) to keep token usage small and accuracy high.

- Heuristics for lists (code)
  - Repeated sibling patterns: common classes, similar tag+class, consistent subtree shapes.
  - Anchor/title/snippet triads for search results; table row detection for tabular data.
  - If unknown, observe returns container selector, then iterate in code (O(N) DOM, O(1) tokens).

- Prompt patterns
  - Observe (selectors):
    - System: “You get a compact DOM outline. Return only selectors/XPaths for elements containing: <task>. Prefer the smallest container that contains all items. Output JSON {candidates:[...]}. No prose.”
    - User: “Find the container and item selectors for ‘search results’.” + snapshot excerpt
  - Semantic extract (scoped):
    - System: “Extract JSON matching this schema. If not found, return empty arrays/fields. No extra text.”
    - User: “Extract ALL articles with { title: string, url: string, date?: string } from the main list.” + scoped outline

- Token efficiency
  - Use outline/prompt strings, not full HTML. Limit to top-K relevant elements via `intelligent-dom` when possible.
  - Prefer small model for observe; larger model only for ambiguous pages. Cache good selectors per domain.

- Caching
  - Key by `{hostname, task_kind}` → selectors/container paths; invalidate on `dom_hash` change or selector miss.
  - Store last successful extraction schema per site/action for reuse.

- Error handling & retries
  - Verify selectors exist before extraction; if zero results, widen scope or fallback to LLM semantic.
  - Use `dom_hash` to detect page mutation; re-run observe if hash changed significantly.
  - Return provenance with each field: `{ selector/xpath, text_sample, source: 'profile'|'observe'|'manual'|'llm' }`.

## Tasks & Status
- [x] Code-first extractors via `extract_structured_data[_enhanced]`
- [x] Compact DOM outline via `captureEnhancedDOMSnapshot` and message handlers
- [x] Validated extraction plan DSL (`execute_extraction_plan`) with provenance
- [x] Add observe API (heuristic selector discovery) in background/content
- [ ] Add semantic extract path (scoped prompt + schema validation) with dummy provider option
- [ ] Cache selectors per host and invalidate on `dom_hash` drift
- [x] E2E coverage on local fixtures (list + table + act)
- [ ] E2E coverage: dynamic feeds / infinite scroll
- [ ] CLI helpers: `rzn-browser extract --schema … [--selector … | --observe …]`

## What Works (Do Not Change)
- Step schemas and enhanced action dispatch wiring in `contentScript.ts`
- DOM capture contract returned by `get_pruned_dom`/`get_dom_snapshot` (elements, prompt, hash)
- Validated extraction plans (`execute_extraction_plan`) stay CSP-safe and deterministic
- Minimal-permission posture; CDP only when required (e.g., cross-origin text)

## Tried & Didn’t Work
- Sending full HTML to LLM: too verbose, brittle to boilerplate noise; increases latency/cost.
- Always-semantic extraction: slow and O(N) tokens for long lists; code iteration is superior after locating scope.
- Hard-coding brittle selectors for unknown sites: breaks often; hybrid observe improves resilience while keeping cost low.

## Examples
- Search results (code-first)
```
step: {
  type: 'extract_structured_data',
  extraction_type: 'search_results', // gives profile a chance
  item_selector: '#search .g, .MjjYud, .tF2Cxc',
  fields: [
    { name: 'title', selector: 'h3' },
    { name: 'url', selector: 'a', attribute: 'href' },
    { name: 'snippet', selector: '.VwiC3b, [data-sncf], .yXK7lf' }
  ]
}
→ JSON: [{ title, url, snippet }]
```

- Stock price (hybrid)
```
observe(instruction: 'Locate the live stock quote for GOOG')
→ candidates[0].selector: '#kp-wp-tab-overview [jsname="vWLAgc"]'
→ extract_structured_data_enhanced(target_spec: { css: candidates[0].selector },
   fields: [ { name: 'price' }, { name: 'currency', selector: 'span:contains("USD")' } ])
→ { price: '134.56', currency: 'USD' }
```

- Table (semantic scoped)
```
get_pruned_dom(scope: '#main table.prices') → outline
LLM(schema: { rows: Array<{ name: string, value: number }> })
→ { rows: [ { name: 'Alpha', value: 12.3 }, … ] }
```

## Minimal API Sketches
- Observe request (planner → extension via broker)
```
{ cmd: 'observe', req_id, payload: {
    instruction: 'find news article items',
    scope_selector: '#main, article, [role="main"]',
    dom_opts: { maxElements: 180 }
} }
```

- Extract enhanced (already supported)
```
step: { type: 'extract_structured_data',
        fields: [{ name: 'title', selector: 'h2' }, { name: 'link', selector: 'a', attribute: 'href' }] }
```

## Test & Eval Plan
- Unit-like (content script):
  - Extraction plan runner returns expected shapes on known fixtures.
  - DOM snapshot size stays under target for typical pages (< 15KB prompt).
- E2E (Playwright):
  - Local fixtures: observe → extract → click/open → extract detail.
  - Local fixtures: nested table extraction plan list mode.
- Policy: Ensure no full-HTML prompts and selector caching reduces repeat cost.
- Recent Changes & Operational Notes
- Prefer deterministic extraction plans first; if unknown layout, observe proposes a container/item selector; if still empty, fall back to semantic extraction.
- CLI quick-extract prints pretty results; if empty, it also prints the raw extension response (debug) to aid troubleshooting.
- Planner uses DOM snapshot only for page state logging (no CDP attach churn). UTF‑8 safe truncation avoids logging panics on multi-byte characters.
- Robust submit is provided by `submit_text_query` (see Robust Input & Submit scratchpad). Autonomous Search mode uses this primitive before extraction.
