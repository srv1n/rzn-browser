# LLM Orchestrator (rzn_plan)

## Overview
- Goal: Plan and control browser automation steps using an LLM with broker-integrated execution. The orchestrator obtains page context (DOM snapshot or HTML), builds directive prompts, asks the LLM for the next atomic action, executes it via the broker, validates outcome, and iterates until the goal is complete or limits are reached.
- Constraints: MV3/extension boundaries; CSP-safe actions first; minimal permissions; cross‑origin handled via escalation (CDP); predictable transport with length‑prefixed JSON; prompt/response size limits; human‑like action pacing.

## Flow Diagrams
- End-to-end flow
```
CLI / API → Orchestrator (rzn_plan) → BrokerClient → Broker (Rust) → Extension (SW) → Content Script → Page
          ←                 Results ←            ←               ←                 ←
```

- Planning loop (3-tier snapshot pipeline)
```
while goal_not_achieved and steps < max:
  1) Context: broker.get_current_dom_snapshot()   // or get_page_source then DomProcessor.extract_dom_context
  2) TIER 1 (Planner): PromptBuilder.build_*planner*_prompt(dom, url, goal, history)
     → LLM returns planned_action {action, parameters, reasoning}
  3) TIER 2 (Navigator): PromptBuilder.build_navigator_prompt(snapshot, planned_action)
     → LLM returns validated action (selector/encoded_id, action_type)
  4) Execute: BrokerClient.execute_step[_and_get_dom](step)
  5) TIER 3 (Validator): PromptBuilder.build_validator_prompt(before_dom, after_dom, step)
     → LLM returns status (continue/complete/failed), feedback, suggestions
  6) Store history, update URL/DOM, loop
```

- DOM routing (extension)
```
Background (SW) → frameRouter.attachToTab(tab) → per-frame sessions
  execute_step → contentScript:
    - enhancedActionHandlers (preferred CSP-safe)
    - CDP escalation for cross-origin/trusted events
Back to SW → Broker → Orchestrator
```

## Decision Record
- BrokerClient abstraction: Encapsulates transport (TCP or named pipe), session state (tab id, current URL), compression for large messages, and DOM snapshot handling.
- Tiered prompts: Split planning into Planner (what to do), Navigator (how to select safely), and Validator (did it work). This keeps individual prompts small, focused, and robust to DOM changes.
- DOM source: Prefer extension DOM snapshots (compact, with encoded ids and hints). Fall back to raw HTML + DomProcessor.extract_dom_context(html, url) if needed.
- Safe behavior: Do not “hack” URLs for search/result pages; rely on human-like navigation + interaction. Explicit policy and sanitizer block unsafe categories (e.g., iframe interaction without escalation).
- Element targets: Prefer encoded identifiers from snapshots or robust selectors produced in Tier 2; never fabricate selectors.

## Architecture
- Modules
  - `crates/rzn_plan/src/orchestrator.rs`: Planning loop, snapshot pipeline, execution, telemetry.
  - `crates/rzn_plan/src/broker_client.rs`: BrokerClient (transport, session, snapshots, step/task execution).
  - `crates/rzn_plan/src/prompt_builder.rs`: System + tiered prompts (planner, navigator, validator).
  - `crates/rzn_plan/src/dom_processor.rs`: `extract_dom_context(html, url)` → `DomContext` for LLM.
  - `crates/rzn_plan/src/dom_analyzer.rs`: HTML reduction when snapshots are unavailable.
  - `crates/rzn_plan/src/plan_sanitizer.rs`: Drops/modifies risky steps before execution.
  - `crates/rzn_plan/src/wait_strategies.rs`: Smart waits and heuristics for dynamic pages.
  - `crates/rzn_plan/src/telemetry.rs`: Session/step cost and trace collection.

- Data contracts
  - Steps: `rzn_core::StepKind` (navigate_to_url, click_element, fill_input_field, press_special_key, extract_structured_data, wait_for_element, etc.).
  - Broker messages: length‑prefixed JSON `Message { action, task_id, task: { steps[...] }, data }`.
  - DOM snapshot: `DomSnapshot { elements[], hash, prompt, metadata{url,title,viewport}, delta? }`.

- Available actions (LLM surface → internal mapping)
  - `navigate(url)` → `StepKind::NavigateToUrl`
  - `click(selector|encoded_id)` → `StepKind::ClickElement`
  - `fill(selector|encoded_id, text)` → `StepKind::FillInputField`
  - `press(selector|encoded_id, key)` → `StepKind::PressSpecialKey`
  - `extract(selector, fields[])` → `StepKind::ExtractStructuredData`
  - `wait(selector)` → `StepKind::WaitForElement`

## Implementation Notes
- Entry points
  - Orchestrator: `plan_llm_only`, `plan_auto`, `plan_with_snapshots` (preferred). Legacy `plan` calls into `plan_auto`.
  - Broker: `execute_step`, `execute_step_and_get_dom`, `execute_steps` (batch), `get_current_dom`, `get_current_dom_snapshot`.
  - LLM: `LLMClient::chat_json` with tier‑specific prompts and timeouts.

- Broker integration & session
  - Maintains `session_id`, `current_tab_id`, and `current_url`; updates from every response.
  - Transforms `Option<T>` fields into extension-friendly JSON prior to send.
  - Compresses large messages to avoid broker crash on payload size limits.
  - Adds `GetPageSource` automatically in `execute_step_and_get_dom` to keep tab state consistent.

- DOM context
  - Prefer `DomSnapshot` for compact context with encoded ids; fallback to HTML → `DomProcessor.extract_dom_context`.
  - Orchestrator tracks DOM hashes to detect loops and recent repeats.

- Validation and safety
  - `PlanSanitizer` prevents unsafe steps (e.g., direct iframe actions without escalation).
  - Validator tier reviews before/after DOM and classifies status (complete/continue/failed) with suggestions.
  - Wait strategies adapt to action type (e.g., typing vs navigation vs dynamic lists).

- Error handling & retries
  - LLM calls retried with small backoff; certain auth errors surface immediately.
  - Failure cache records bad selectors/context for smarter next attempts.
  - On success, clear failure counters; on repeated failures, reconsider strategy (e.g., re‑snapshot, re‑plan).

## Tasks & Status
- [x] DOM snapshot‑based tiered planning in `orchestrator.rs` (Planner/Navigator/Validator)
- [x] `BrokerClient`: transport, session tracking, snapshots, `execute_step_and_get_dom`
- [x] Prompt builder with tool instructions and safety rules
- [x] URL tracking from broker response (not inferred from DOM text)
- [x] Message compression + Option<T> normalization for extension
- [x] Add `BrokerClient::execute_steps(steps)` batch method
- [ ] Expand Navigator tier coverage for additional action families
- [ ] Improve loop detection heuristics (DOM hash windows, thresholds)
- [ ] Add richer metrics to telemetry (selector quality, retries by action type)

## What Works (Do Not Change)
- `rzn_core::StepKind` schema and transport protocol (length‑prefixed JSON)
- Encoded ID and selector usage rules in prompts (never guess selectors)
- DOM snapshot format and Planner/Navigator/Validator separation
- Policy: no direct URL hacks for search pages; prefer realistic human interaction

## Tried & Didn’t Work
- Robust selectors fallback inside broker client: added complexity and drift; replaced with Navigator tier + sanitizer.
- Pure raw HTML context for planning: too large and noisy; replaced with compact DOM snapshots and structured context.
- Constructing Google direct search URLs: flagged in policy; use search box + Enter for stealth and consistency.

