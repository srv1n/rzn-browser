#+ Enhanced Action Executor

## Overview
- Goal: Reliable content-side actions with a DOM → scripted-event fallback ladder.
- Constraints: CSP-safe by default; background-owned CDP actions remain a separate path.

## Flow Diagrams

- Execution path
```
Content Script → EnhancedActionExecutor.execute(action)
  ├─ Resolve TargetSpec (encoded_id | css | xpath | role_name | text_near)
  ├─ Rung 1: DOM events (fastest)
  ├─ Rung 2: JS synthesis (better compatibility)
  └─ Return the final content-ladder result
```

- Example (click)
```
click_element
  ↓ resolve element
  ↓ try DOM click
  ↓ if fail → synthesize events
  ↓ return failure for the caller to handle
```

## Call Graphs

- From dispatcher
```
contentScript.ts
  └─ enhancedActionHandlers.click_element_enhanced(step)
     └─ EnhancedActionExecutor.execute({ type: 'click_element', target_spec, ... })
        ├─ resolve element (map, selector, xpath, AX)
        ├─ try rung 1 (DOM)
        ├─ try rung 2 (synthesis)
        └─ return the final content-ladder result
```

## Architecture
- `extension/src/content/actions-enhanced.ts`: registry and helpers
- `extension/src/input/ladder_content.ts`: content-side rung sequencing
- `extension/src/input/rungs/dom.ts` and `scripted.ts`: content-side implementations
- `extension/src/cdp/*`: separate background-owned CDP actions

## Implementation Notes
- Record actions to flight recorder for debugging.
- Keep rung budgets and retry counts conservative.
- Return structured result with `rung_used`, `escalated`, `execution_time_ms`.
- Treat DOM/scripted rung misses as normal ladder signals, not operator-facing warnings, unless the final action fails.
- Keep CDP diagnostics readable when break-glass is used; log the concrete protocol/runtime error text instead of opaque object dumps.
- Preserve the content policy: DOM first, scripted second. Do not smuggle background CDP ownership back into this ladder.
- Eval-backed JavaScript actions must fail loudly when injected code throws; do not map Chrome scripting errors or wrapper exceptions to `success: true` with a null result.

## Tasks & Status
- [x] Click, fill, press_key, hover, scroll_into_view
- [x] Text & structured data extraction
- [x] Reduce transient ladder log noise so fallback attempts do not look like terminal failures
- [ ] File upload & clipboard integration via CDP (as needed)

## What Works (Do Not Change)
- Content rung ordering
- TargetSpec normalization

## Tried & Didn’t Work
- Single-path action without escalation: brittle across sites
- Always-CDP: detectable and heavier on perf
- Treating every rung miss as a warning/error: too noisy during normal escalation and obscures the actual terminal failure.
