**Overview**
- Goal: Interactive, site-agnostic CLI to define deterministic workflows without hard-coded, site-specific logic. The builder asks for a name, required/optional parameters, an optional root domain, and desired outcome (single vs. list + fields), then emits a runnable workflow JSON and a companion params schema file.
- Constraints: No site-specific selectors. Use only generic actions from StepKind. Output must validate against the existing workflow schema. Keep prompts minimal; avoid LLM calls during the builder phase.

**Flow Diagrams**
- End-to-end
CLI (workflow new) -> collect inputs -> generate Workflow JSON -> write ~/.rzn/workflows/<slug>.json
                                               \-> write ~/.rzn/workflows/<slug>.params.json

- Run-time (deterministic)
workflow run -> Orchestrator -> Broker -> Extension -> Page

**Decision Record**
- Chosen: Minimal deterministic builder now; optional simulate-and-bind phase later to “learn” repeated lists via DOM inventory and expand steps.
- Alternatives: Template-driven (_rejected_) – felt too Google-specific and constrained. LLM-in-the-loop builder (_deferred_) – higher latency/cost; keep as opt-in.

**Architecture**
- Modules
  - crates/rzn_browser/src/main.rs: handle_workflow_new(), generate_generic_workflow()
  - crates/rzn_plan/src/workflow_manager.rs: loads .json/.yaml workflows; caches and indexes by description keywords
- Data contracts
  - rzn_core::dsl::{Workflow, BrowserAutomation, Sequence, Variable, Step}
  - Step uses rzn_core::StepKind (flattened). No raw/custom types.

**Implementation Notes**
- Builder prompts for: name → required params → optional params → goal → root domain(s) → defaults → outcome (list? fields?)
- Output files:
  - Workflow JSON: ~/.rzn/workflows/<slug>.json
  - Params meta: ~/.rzn/workflows/<slug>.params.json (required/optional/defaults, outcome)
- Current workflow seed: optional NavigateToUrl step to the first root domain. Subsequent steps are expected to be added by a “simulate and bind” pass (planned).

**Tasks & Status**
- [x] Generic CLI builder replacing template menu
- [x] WorkflowManager loads .json as well as .yaml
- [ ] Add simulate_and_bind: probe DOM, learn repeated list itemSelector, append ExtractStructuredData
- [ ] Param substitution preview and validation
- [ ] Unit test: capture builder I/O transcript

**What Works (Do Not Change)**
- No site-specific selectors in generator; only generic StepKinds
- Keep builder non-LLM by default

**Tried & Didn’t Work**
- Static template menu (Google-only): blocked generalization; removed
