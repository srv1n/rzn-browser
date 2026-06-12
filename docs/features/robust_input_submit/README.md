#+ Robust Input & Submit

## Overview
- Goal: Make typing and submitting queries/forms reliable across sites (e.g., Google), without stalls or brittle paths. Provide a generic text submission primitive that works with suggestion overlays, forms, and submit buttons, and escalates to trusted events via CDP only as a last rung. Keep the planner/tool prompts aligned with the actual executor to prevent action mismatches.
- Constraints: MV3, CSP-safe (no inline injection), minimal permissions, short CDP leases, per-domain overrides (e.g., google.*). Avoid content-side CDP (chrome.debugger/chrome.tabs not available in content scripts).

## Flow Diagrams
- Submit Ladder (generic)
```
submit_text_query(sel, value?)
  ↓ focus + fill (DOM)
  ↓ Enter (DOM key events)
  ↓ If listbox/combobox open → form.requestSubmit/submit
  ↓ Click submit-like button (local → global)
  ↓ CDP Enter (background) — trusted, one shot
  ↓ Success = URL or DOM hash change
```

- Planner (Search) fast path
```
type(selector, text) → FillInputField (DOM)
press("Enter") in Search mode → submit_text_query
wait_for_element(results_h3) → FSM: Search → Results
extract (profile → observe → fallback)
```

- CDP attach policy
```
Background SW: frameRouter.attachToTab(tab)
  Target.setAutoAttach(flatten=true)
  Page.enable / Runtime.enable (Target.enable optional/omitted)
On error "Another debugger..." → log once + return; fallback to DOM
Short lease; detach on idle
```

## Decision Record
- Why background-only CDP: Content scripts cannot use chrome.debugger/chrome.tabs; trusted events must be issued in background SW. Content-side CDP threw runtime errors and caused stalls.
- Why submit ladder: Suggestion overlays often swallow Enter; form semantics/requestSubmit and submit buttons are robust. CDP Enter is kept as a last rung for consistency.
- Why remove CDP context (get_cdp_context) from planner page state: CDP attach churns on startup and is unnecessary for the LLM context. DOM snapshot is sufficient and lighter.
- Why CDP-first for google.*: To stabilize development on Google, default trusted key rung for Enter via site overrides, then revert later when DOM path proves reliable.

## Architecture
- Modules
  - `extension/src/contentScript.ts`: adds `submit_text_query` with rung ladder; logs via content-logger; prefers standard handlers by default.
  - `extension/src/background.ts`: routes static commands; `press_key_cdp` path executes trusted key via chrome.debugger; `set_flags` to override per-domain flags.
  - `extension/src/cdp/frameRouter.ts`: stable attach (Target.setAutoAttach(flatten=true)); no `Target.enable`; graceful errors.
  - `extension/src/input/rungs/cdp.ts`: CDP rung guarded (disabled in content); executes only where chrome.debugger available (background).
  - `extension/src/config/siteOverrides.ts`: google.* treated as requiring trusted input (CDP rung allowed).
  - `crates/rzn_plan/src/llm_autonomous.rs`: type → FillInputField only; press Enter (Search) → submit_text_query + wait → FSM to Results; UTF‑8 safe truncation; use `get_dom_snapshot` for page state.
- Data contracts
  - `execute_step` (unchanged): steps routed to content; robust submit invoked via raw step `{ type: "submit_text_query", selector, ... }`.
  - `press_key_cdp` (background message): `{ action: 'press_key_cdp', key }` returns `{ success }`.
  - `set_flags` (background static): `{ cmd: 'set_flags', payload: { overrides } }`.

## Implementation Notes
- submit_text_query
  - Input resolution: `selector` → element; fallback to activeElement or common search inputs.
  - Fill value (input/change events), then baseline (beforeUrl, domHash).
  - Rungs (each with ~1.2–1.5s change wait): Enter → (if listbox) form.requestSubmit/submit → local submit button → global submit button → background CDP Enter via `press_key_cdp`.
  - Logging: logs each rung outcome to background via CONTENT_LOG; includes baseline, changed flags, and rung method used.
- Content CDP rung guard
  - `canExecute` early returns false in content; CDP rung only runs where chrome.debugger is present (background SW).
- Frame Router attach
  - Removed `Target.enable` (not required; caused protocol errors). On “Another debugger attached”, log + return; caller falls back.
- Planner
  - Removed `get_cdp_context` call from page state; use only `get_dom_snapshot` to avoid CDP churn.
  - Type: DOM only; Google rewrite to `textarea[name='q']`.
  - Press Enter (Search): raw step `submit_text_query` then `wait_for_element` for results; transition FSM to Results.
  - UTF‑8 logging: `safe_truncate_utf8` helper for prompt/content truncation.

## Tasks & Status
- [x] Generic submit ladder (DOM → form/button → background CDP Enter)
- [x] Content-side CDP rung disabled; background-only CDP path wired (press_key_cdp)
- [x] Frame router: no Target.enable; graceful attach errors
- [x] Planner: type/Enter fixed; Search→Results transition after wait
- [x] Planner page state: remove CDP context probe; use DOM snapshot
- [x] UTF‑8 safe truncation in logs
- [x] Google CDP-first via site overrides
- [x] CLI: tiered extraction and pretty print; raw response on empty
- [ ] Dismiss popups before submit (cookie banners) — optional next
- [ ] Session-aware tabs (Map<session_id, tab_id>) for parallel tabs
- [ ] select_option_in_dropdown handler in extension
- [ ] Background-only CDP macro for generic type/click (batch mode)
- [ ] LLM-assisted observe for selector discovery (scoped snapshot)

## What Works (Do Not Change)
- Background-only CDP for trusted key (no content-side chrome.debugger usage)
- submit_text_query rung ordering and short waits
- DOM snapshot path for planner page state (no CDP attach there)
- FSM transition to Results before extract

## Tried & Didn’t Work
- Content-side CDP rung (ladder rung 3): accessing chrome.tabs/chrome.debugger in content caused runtime errors and stalls.
- `Target.enable` during attach: not required; threw “wasn’t found” and increased attach churn.
- Inline test bridge injection: tripped CSP (unsafe-inline); removed in favor of manifest-provided pageBridge in MAIN world.

