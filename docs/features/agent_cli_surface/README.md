**Overview**
- Goal: Mirror agent patterns shown in public demos using our CLI/LLM planner and extension, so developers can run goal-directed browsing tasks with similar ergonomics (task string → agent loop → result), locally and without Playwright.
- Constraints: Keep our Rust + MV3 architecture; keep the CLI as the primary surface; prioritize CLI compatibility and JSON I/O.

**Flow Diagrams**
`CLI → rzn-browser llm-auto → rzn_plan (planner/tools) → rzn_broker → Extension → Page`
`                                            ← logs/results ←                         `

**Decision Record**
- Emulate the public agent `run(task)` pattern via `rzn-browser llm-auto "task"` first; add a thin language binding later only if it earns its keep.
- Reuse our existing `tools` (navigate/click/fill/press/scroll/wait/extract) and allow user-defined tools via a simple JSON-RPC hook.

**Architecture**
- Modules:
  - `crates/rzn_browser` subcommand `llm-auto` (exists) + flags for provider, max steps, and JSON output.
  - `crates/rzn_plan/llm_autonomous.rs`: planner loop, tool calls, extraction helpers.
  - Optional: `bindings/` thin wrapper that shells out to `rzn-browser` and streams logs.
- Data contracts:
  - Input: `{ goal: string, maxSteps?, provider?, memory? }`
  - Output: `{ success, steps: [...], result?, error? }`

**Implementation Notes**
- Add `--json` to `llm-auto` for machine-readable parity.
- Provide example tasks to match reference README patterns: search, fill forms, paginate, extract lists.
- Introduce `--tool-hook http://localhost:PORT` to let users register custom tools (parity with the reference ecosystem, minimal viable).

**Tasks & Status**
- [ ] `llm-auto --json` output and structured telemetry.
- [ ] Example scripts: `examples/agent_surface/quickstart.sh` mapping to README task.
- [ ] Optional thin binding under `bindings/` with `Agent.run()` calling CLI.
- [ ] Tool hook: JSON-RPC over HTTP for custom actions.
- [ ] Tests: reproduce top N results extraction scenario.

**What Works (Do Not Change)**
- Planner’s existing tool taxonomy and failure recovery.
- Dummy provider option (`LLM_PROVIDER=dummy`) for deterministic demos.

**Tried & Didn’t Work**
- Full Playwright parity: unnecessary since extension already handles DOM routing; keep local.
