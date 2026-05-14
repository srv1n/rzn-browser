#+ Enhanced Action Executor

## Overview
- Goal: Reliable, human-like actions with escalation (DOM → JS synthesis → CDP) via an input “ladder”.
- Constraints: CSP-safe by default (DOM events), CDP only as needed; works across frames via `frameRouter`.

## Flow Diagrams

- Execution path
```
Content Script → EnhancedActionExecutor.execute(action)
  ├─ Resolve TargetSpec (encoded_id | css | xpath | role_name | text_near)
  ├─ Rung 1: DOM events (fastest)
  ├─ Rung 2: JS synthesis (better compatibility)
  └─ Rung 3: CDP Input (trusted events, cross-origin)
```

- Example (click)
```
click_element
  ↓ resolve element
  ↓ try DOM click
  ↓ if fail → synthesize events
  ↓ if still fail → frameRouter → cdpClient Input.dispatchMouseEvent
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
        └─ try rung 3 (CDP via frameRouter)
```

## Architecture
- `extension/src/content/actions-enhanced.ts`: registry and helpers
- `extension/src/input/ladder.ts`: rung sequencing, retries, delays
- `extension/src/input/rungs/*`: DOM, synthetic, CDP implementations
- `extension/src/cdp/*`: router and client for CDP fallback

## Implementation Notes
- Record actions to flight recorder for debugging.
- Keep rung budgets and retry counts conservative.
- Return structured result with `rung_used`, `escalated`, `execution_time_ms`.
- Treat DOM/scripted rung misses as normal ladder signals, not operator-facing warnings, unless the final action fails.
- Keep CDP diagnostics readable when break-glass is used; log the concrete protocol/runtime error text instead of opaque object dumps.
- Preserve the default policy: DOM first, scripted second, CDP only when earlier rungs cannot complete the action.
- Eval-backed JavaScript actions must fail loudly when injected code throws; do not map Chrome scripting errors or wrapper exceptions to `success: true` with a null result.

## Tasks & Status
- [x] Click, fill, press_key, hover, scroll_into_view
- [x] Text & structured data extraction
- [x] Reduce transient ladder/CDP log noise so fallback attempts do not look like terminal failures
- [ ] File upload & clipboard integration via CDP (as needed)

## What Works (Do Not Change)
- Rung ordering and minimal-CDP default
- TargetSpec normalization

## Tried & Didn’t Work
- Single-path action without escalation: brittle across sites
- Always-CDP: detectable and heavier on perf
- Treating every rung miss as a warning/error: too noisy during normal escalation and obscures the actual terminal failure.
