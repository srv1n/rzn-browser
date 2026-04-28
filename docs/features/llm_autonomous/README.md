#+ LLM Autonomous Planner

## Overview
- Goal: Single, CSP-safe autonomous planner that plans and executes browser actions end-to-end without bespoke per-site scripts.
- Constraints: MV3, CSP-safe defaults; minimal permissions; cross-origin frames via CDP as fallback; deterministic broker transport.

## Flow Diagrams

- End-to-end message flow
```
User/CLI → Orchestrator/Planner → Broker (Rust) → Extension (SW) → Content Script → Page DOM
         ←               Results ←          ←                ←                        
```

- Planner loop (FSM + policy)
```
+--------------------+
| LLMAutonomousPlanner|
+--------------------+
        |
        v      (get page state)
  [BrokerClient::get_page_state]
        |
        v      (LLM JSON)
  parse_llm_response → [actions]
        |
        v      (policy + mode)
  PolicyValidator + PlannerState
        |
        v      (execute)
  BrokerClient::execute_step → Extension → DOM
        |
        +--> iterate until complete/limit
```

- DOM routing (extension)
```
Background (SW) → frameRouter.attachToTab(tab) → per-frame sessions
  execute_step → contentScript:
    - enhancedActionHandlers (preferred)
    - fallback legacy static actions
Back to SW → Broker
```

## Decision Record
- Single autonomous file `llm_autonomous.rs` replaces multiple variants to remove confusion and drift.
- CSP-safe static actions first; CDP fallback for reliability (trusted events, cross-origin frames).
- Explicit policy gate prevents URL construction on sensitive/search sites.

## Architecture
- Modules
  - `crates/rzn_plan/src/llm_autonomous.rs`: planner loop, parsing, policy+FSM integration.
  - `crates/rzn_plan/src/broker_client.rs`: transport to extension.
  - `extension/src/background.ts`: routes steps, manages CDP via `frameRouter`.
  - `extension/src/contentScript.ts`: enhanced action handlers + DOM capture.
  - `extension/src/cdp/frameRouter.ts`: session management across frames.
- Data contracts
  - Steps: `rzn_core::StepKind` (navigate, click, fill, press_key, wait, extract, etc.)
  - Messages: length-prefixed JSON (broker ↔ extension), `cmd: 'execute_step'` payloads.

## Implementation Notes
- Entry points
  - `LLMAutonomousPlanner::execute_autonomous(request)`
  - `LLMAutonomousPlanner::parse_llm_response(response)`
  - `BrokerClient::execute_step(step)`
  - Extension background: `onMessage { cmd: 'execute_step' }`
  - Content Script: `enhancedActionHandlers` (preferred)
- Error handling
  - Planner tracks consecutive failures; falls back and retries where safe.
  - Policy validator returns specific violations; planner adapts or stops.

## Tasks & Status
- [x] Single planner (`llm_autonomous.rs`) wired into CLI
- [x] Tests do not require real API keys
- [x] Extension build with Bun; legacy OPFS downloads removed
- [ ] Expand enhanced action coverage (track gaps)
- [ ] Add more FSM policy cases where needed

## What Works (Do Not Change)
- `frameRouter`-based CDP session management
- JSON step schema in `rzn_core::StepKind`
- Broker transport protocol (length-prefixed JSON)

## Tried & Didn’t Work
- OPFS-based image download path (removed): brittle and redundant with Downloads API via background.
- Multiple autonomous planner variants: caused drift; replaced with single planner.
