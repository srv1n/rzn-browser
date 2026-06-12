Overview
The goal is to enable natural language tasks like “First comment of top 5 posts of hackernews” to run fully autonomously via `rzn-browser llm-auto` without site-specific code. The feature adds a generic list-iteration macro that: (1) detects a repeated list on a landing page, (2) picks a same-origin “discussion/comments” link per item using generic anchor scoring, and (3) on the thread page extracts the first top-level comment’s author, time text, and body using structure discovered from the DOM inventory.

Flow Diagrams
- End-to-end
  1) Planner receives instruction → matches “first comment of top N …” macro
  2) Navigate (if site is named) → process_dom(detectAutoList)
  3) Build queue from autoList.itemIds
  4) For each item: pick best internal anchor (comments/discussion) → Navigate
  5) process_dom(detectAutoList) on thread → pick first itemId → extract author/time/body → append result
  6) Return JSON list of N results

- Key internal flows
  - List Detection: contentScript `process_dom` computes `autoList` (containerSelector, itemSelector, itemIds) + inventory (elements, idToXPaths, idToUrl)
  - Anchor Selection: Rust macro scores anchors inside each list item using text/url heuristics (no site-specific selectors)
  - Comment Extraction: ID-first descendant filtering picks short link as author, “ago/Month/Year” text as time, and first long text block as body

Decision Record
- Chosen: ID-first inventory (process_dom + autoList.itemIds) over brittle CSS rules to keep site-agnostic
- Chosen: Anchor scoring by semantics (“comments/discuss/replies/thread”) + same-origin preference
- Rejected: Hard-coded per-domain patterns (e.g., `item?id=...`) to comply with guardrails

Architecture
- Modules
  - rzn_plan::llm_autonomous::try_first_comment_top_n: macro entry and orchestration
  - extension/contentScript.ts: process_dom + autoList detection (existing)
- Data contracts
  - process_dom result: { elements[], idToXPaths{}, idToUrl{}, autoList{ itemIds[], containerSelector, itemSelector } }
  - Output: [ { title, discussion_url, first_comment: { author, time, text } } ]

Implementation Notes
- Anchor scoring (generic):
  - +40 same-origin; -10 external
  - +50 if text matches /(comment|comments|discuss|discussion|reply|replies|thread|threads)/i
  - +20 if also numeric count present
  - -30 for non-discussion actions like hide/share/save/report/next/previous
  - +8 if url includes #comments/#discussion or path contains /comments|/discussion|/thread
- First comment fields:
  - author: first short (<20 chars, no spaces/@) descendant link
  - time: first descendant text matching “X minutes/hours/days ago”, Month name, or year
  - text: first long-ish (>=40 chars) descendant paragraph-like element
- Retries/backoffs: rely on existing wait + DOM capture paths; avoids oscillating scrolls

Tasks & Status
- [x] Macro orchestration in Rust (ID-first, no site-specific code)
- [x] Anchor scoring and first-comment extraction heuristics
- [ ] Add unit tests for anchor scoring (JSON fixture of process_dom output)
- [ ] E2E synthetic page test in extension/tests/e2e for repeated-list → first-comment flow

What Works (Do Not Change)
- Do not add site/domain-specific selectors or URL checks in code paths
- Keep selection driven by DOM inventory (CDP AX tree / process_dom) and repeated-list detection

Tried & Didn’t Work
- Using only “first anchor inside item” as discussion link: on many sites this is navigation (title) or utility links (hide/share), not the comments page
