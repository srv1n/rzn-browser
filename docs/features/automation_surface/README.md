**Overview**
- Goal: Offer a first-class "act / observe / extract" surface on top of our Rust CLI + Chrome extension so we can mirror the capabilities shown in the public reference implementations without inheriting their branding. Target drop-in parity for common examples while staying local-first.
- Constraints: MV3 extension + native messaging; keep actions deterministic and auditable; respect unified logging; avoid breaking existing workflows.

**Flow Diagrams**
- End-to-end
`User API (Node/CLI) → rzn-browser → rzn_plan (LLM/planner) → rzn_broker → Extension (SW) → Content Script → Page`
`                                                    ← telemetry/results ←                                       `

- Internal flows
`act(instruction) → single-step LLM toolcall → StepKind → execute_step`
`extract(schema) → AX/DOM inventory → LLM ID-first selection → map id→url/text → JSON`
`observe(prompt) → processDom/detectAutoList → summarize actionable elements → return candidates`
`agent.execute(goal) → LLMAutonomousPlanner loop → steps + (optional) extract → finish`

**Decision Record**
- Ship the CLI surface first (new subcommands) for the fastest path to parity; add SDKs later for ergonomics.
- Prefer AX/DOM ID-first extraction (already implemented) to replicate schema extraction without hard selectors.
- Keep everything local-first through our broker; defer any hosted/cloud dependencies.

**Architecture**
- Modules:
  - `crates/rzn_plan/llm_autonomous.rs`: ID-first extraction helpers, autonomous planner.
  - `crates/rzn_plan/orchestrator.rs`: step execution and tool adapters.
  - `crates/rzn_core`: `StepKind` for click/fill/press/wait/scroll/extract.
  - `crates/rzn_browser`: add subcommands: `act`, `extract-schema`, `observe`, `agent`.
- Data contracts:
  - `act`: `{ instruction: string, url?: string } → { success, step, reasoning? }`
  - `extract-schema`: `{ fields: [{ name, kind?, optional? }], limit?, scopeSelector? } → { items: [...] }`
  - `observe`: `{ prompt?: string, limit? } → { actions: [{ id, selector?, role?, text?, action }...] }`
  - `agent`: `{ goal: string, maxSteps?, provider? } → { success, steps, result? }`

**Implementation Notes**
- Entry points (CLI):
  - `rzn-browser act "Click the Sign In button" --url https://example.com`
  - `rzn-browser extract-schema --fields '[{"name":"title"},{"name":"url","kind":"url"}]' --limit 5`
  - `rzn-browser observe --limit 30`
  - `rzn-browser agent "Find hotels in SF and list top 3" --max-steps 12`
- Key calls:
  - Use `LLMAutonomousPlanner` for `agent`, `parse_browser_step` single toolcall for `act`.
  - For `extract-schema`, reuse `extract_schema_id_first` (AX/DOM inventory → LLM JSON). Support URL/text mapping.
  - For `observe`, wrap `process_dom` + `detect_auto_list` and summarize to a compact action list.
- Error handling & retries:
  - `act`: on selector miss, try `observe` to propose alternatives; retry once.
  - `extract-schema`: if inventory empty, fallback to AX text-only pass.

**Tasks & Status**
- [x] CLI: add `act`, `extract-schema`, `observe` wrappers (agent routes through `llm-auto`).
- [x] Prompt: tighten `act` single-step tool schema.
- [x] Observe: summarize `processDom/detectAutoList` to candidate actions.
- [ ] Extract: accept field kinds (text/url/image/price) and optionality; map to ID-first. _(attributes wired; optional flags still todo)_
- [ ] Node SDK (optional v2): thin wrapper exposing `page.act/extract/observe` to mirror the public reference surface.
- [ ] Tests: Playwright e2e parity cases (act, extract top results, observe buttons).

**What Works (Do Not Change)**
- Unified logging (`~/rzn_build.log`) and broker wiring.
- Core `StepKind` semantics and extension bridge: `window.__rznExecuteStep(...)`.

**Tried & Didn’t Work**
- Full Browserbase emulation: unnecessary for local parity and adds complexity.
