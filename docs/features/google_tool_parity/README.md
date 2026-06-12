# Google Tool Parity Workflows

## Overview
Goal: provide workflow coverage for the Google-focused tool surface exposed by the `noapi-google-search-mcp` project (web properties like Search/News/Maps/Finance), implemented as **RZN workflow JSONs** under `workflows/google/` so the repo has one-to-one “tool” parity at the workflow layer.

Constraints: these are stealth-first browser workflows executed via the extension + native host + broker. Google properties are heavily client-rendered and sometimes gated by consent/captcha flows, so many workflows are intentionally “best-effort” and prefer robust waits + light extraction over brittle, deeply nested selectors.

## Flow Diagrams
- End-to-end flow
```
rzn-browser → rzn_plan Orchestrator → rzn_broker → Extension (SW) → Content Script → Google page
       ← results (JSON)       ←           ←               ←               ←
```

- Internal flow (run-workflow)
```
Load workflow JSON
  → validate required params (with aliases like query↔search_query)
  → execute steps sequentially via broker
  → merge ExtractStructuredData payloads
  → print formatted output (or JSON fallback)
```

## Decision Record
- Why this approach: workflows are the fastest path to parity without adding new product surface area or taking on external dependencies. They also remain easy to iterate on with `make run`.
- Tradeoffs: workflows can’t express conditional logic (e.g., optional parameters) and Google UIs change frequently; some tools (e.g., “Lens detect” with local OpenCV) cannot be replicated exactly with browser-only steps.
- Alternatives considered:
  - Add a dedicated “Google tool layer” in Rust: more robust long-term, but higher lift and risks adding domain-tuned code paths.
  - Use LLM auto-healing for everything: increases cost and reduces determinism; these workflows are designed to run with `--no-auto-heal` first.

## Architecture
- Modules:
  - `workflows/google/*.json`: the parity surface (data).
  - `crates/rzn_plan/src/orchestrator.rs`: parameter aliasing + step execution (runtime).
  - `crates/rzn_plan/tests/*`: lightweight workflow parse/shape checks.
- Data contracts:
  - Workflow JSON uses v1 `browser_automation.sequences[].steps[]` with step objects shaped like `rzn_core::dsl::Step`.
  - Extract outputs are JSON arrays of objects (best-effort, page-dependent).

## Implementation Notes
- Entry points:
  - `rzn-browser run workflows/google/<name>.json --param ...`
  - `make run W=workflows/google/<name>.json PARAMS='--param ...'`
- Parameter aliasing:
  - Common search parameter name mismatches are normalized (`search_query` ↔ `query` ↔ `q`) before validation/execution.
- Error handling & retries:
  - Workflows prefer explicit `wait_for_element`/`wait_for_timeout` and `dismiss_popups`.
  - For unstable surfaces, workflows fall back to `get_element_text` on `main`/`body` rather than complex structured extraction.

## Tasks & Status
- [x] Add workflow JSONs for missing Google tools (shopping/flights/hotels/translate/maps/directions/weather/finance/scholar/books/trends/lens/lens-detect)
- [x] Document workflow inventory and parameter aliasing
- [x] Add a minimal parse test to keep workflows from drifting into invalid step shapes
- [x] Iterate on selectors based on live runs (only when needed)
- [x] Stabilize workflow runs (reduce extension response size + add retries for content-script message races)

## What Works (Do Not Change)
- Runner-level parameter aliases (`search_query`/`query`/`q`) should remain generic and non-domain-specific.
- Workflows should avoid hard-coding site-specific logic into Rust/TS code paths; keep domain tuning inside workflow data.

## Tried & Didn’t Work
- Full fidelity parity for `google_lens_detect` (OpenCV object detection + per-object Lens calls): not achievable with current workflow step surface without adding local vision tooling and conditional branching.
