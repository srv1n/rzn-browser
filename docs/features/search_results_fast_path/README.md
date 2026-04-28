**Overview**
- Goal: Make common “search → extract top N results” tasks fast and low-latency by avoiding multiple LLM turns once results are visible.
- Constraints: CSP-safe; no site-specific selectors. Prefer browser-side inventory and enhanced extractors. Keep logic generic and reusable across engines and sites.

**Flow Diagrams**
- Results Short-Circuit
LLMAutonomousPlanner (mode=Results) -> try AX-based extraction (Google) -> enhanced extract_structured_data (search_results) -> legacy observe+extract fallback -> return

**Decision Record**
- Short-circuit inside Results mode to eliminate extra “think” turns once the results page is confirmed.
- Prefer structured extractor first; scroll loops only as a last resort.

**Architecture**
- Modules
  - crates/rzn_plan/src/llm_autonomous.rs
    - ax_extract_google_results(top)
    - fast_search_extract(url, query, top)
    - Results-mode short-circuit in execute_autonomous()
  - crates/rzn_plan/src/broker_client.rs
    - execute_raw_step for enhanced extraction types
- Data contracts
  - Raw step payloads for enhanced extract_structured_data with extraction_type="search_results"

**Implementation Notes**
- If URL indicates a Google results page, try AX-based extraction (id/url map) with no selectors.
- Otherwise call enhanced extractor with extraction_type=search_results, then legacy observe→extract if empty.
- On submit, use submit_text_query raw action (press Enter, requestSubmit) and then wait for results selectors.

**Tasks & Status**
- [x] Results-mode short-circuit
- [x] Enhanced extractor path + fallbacks
- [x] Submit helper (raw) wired
- [ ] DOM hash de-dupe to skip redundant snapshots
- [ ] Tests for short-circuit and fallback order

**What Works (Do Not Change)**
- No site hard-coding beyond generic profile signal (extraction_type)
- Use broker raw step routing only for enhanced extractors/submit

**Tried & Didn’t Work**
- Pure scroll+legacy extract first: oscillation and latency on dynamic SERPs

