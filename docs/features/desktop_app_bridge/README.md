# Desktop App Bridge (Tauri Tool Adapter)

## Overview
- Goal: Integrate an external desktop agent (Tauri app with LLM + local context/tools) with RZN’s stealth-first browser control so the desktop can **observe / extract / act** inside the user’s *real* browser profile via **native messaging** (no webdriver flags by default).
- Constraints: MV3 extension boundaries; native messaging size/latency limits; event trust (`isTrusted`) limitations for synthetic DOM events; cross-origin iframes; keep actions deterministic/auditable; unified logging; **avoid site-specific selectors and domain-tuned rules** (prefer AX/CDP IDs, roles, repeated-list detection, same-origin preference).

## Flow Diagrams
- End-to-end flow (desktop-driven)
```
Tauri Desktop (LLM + tools)
  ↕ (TCP/pipe, length-prefixed JSON)
rzn_broker (native messaging host)
  ↕ (Chrome native messaging)
Extension Service Worker
  ↕ (tabs.sendMessage / frameRouter)
Content Script / CDP (chrome.debugger)
  ↕
Web Page (DOM)
```

- Two-tier control (what vs how)
```
Tier A: Desktop Agent decides intent
  - observe (get a compact page inventory)
  - extract (structured JSON)
  - act (click/type/scroll/navigate)
  - think (no browser call)

Tier B: RZN executes atomic steps
  - StepKind / TargetSpec → ladder: DOM → JS synthesis → CDP
  - Returns structured results + provenance
```

## Decision Record
- **Reuse the existing broker transport** (length-prefixed JSON over TCP/pipe) and the existing `rzn_core` step schema, so the desktop app can call the same primitives as the CLI.
- **Prefer the step DSL and TargetSpec over arbitrary JS snippets.** Arbitrary snippets are powerful but unsafe and hard to audit; most extraction/actions should be expressible as structured “plans” executed by the extension.
- **Use an escalation ladder for reliability**: DOM events first (least invasive), then synthetic sequences, then CDP input (trusted events + cross-origin). Keep CDP on-demand to minimize surface area.
- **Keep Playwright/WebDriver as an optional last resort**, not the default. When used, prefer connecting to an existing browser/profile rather than launching automation-flavored browsers.

## Architecture
- Existing modules (already in-tree)
  - `rzn_broker/src/main.rs`: broker relay (desktop ↔ extension), supports TCP/pipe with 4‑byte LE length prefix framing.
  - `crates/rzn_plan/src/broker_client.rs`: client that speaks the broker protocol (session tracking, compression, retries).
  - `crates/rzn_plan/src/orchestrator.rs`: tiered planning loop (Planner/Navigator/Validator) when you want “full autonomy”.
  - `extension/src/background.ts`: native port connection, CDP lease + frame router, per-host flags/circuit breakers.
  - `extension/src/contentScript.ts`: step dispatcher, DOM snapshot capture, enhanced action executor entry points.
  - `extension/src/content/ax-capture.ts`: AX slice helpers (frameId + backendNodeId inventory).

- Implemented additions (desktop-facing surface + safety)
  - **Desktop tool adapter layer** (library, reusable by Tauri):
    - `crates/rzn_plan/src/desktop_tools.rs`: desktop-friendly `observe/extract/act/execute_steps` wrappers with structured error codes.
    - `crates/rzn_plan/src/desktop_session.rs`: `DesktopSession` wrapper for safe defaults + policy enforcement.
  - **Policy gates** (high-risk actions require confirmation or are blocked):
    - `crates/rzn_plan/src/policy_gate.rs`: `PolicyGate` + `PolicyConfirmer` trait (`RZN_POLICY_AUTO_APPROVE=1` escape hatch for local dev).
  - **Safe “extraction plan” DSL** (no arbitrary JS):
    - `schema/extraction-plan-v1.json` and `extension/src/types/extractionPlan.ts` (Zod runtime validation).
    - `extension/src/contentScript.ts`: `execute_extraction_plan` command + step handler returning provenance (`plan_version`, `rung_used`, `dom_hash`).

- Data contracts (current + recommended)
  - Transport framing: `u32le(len)` + `len` bytes JSON (same as native messaging).
  - Request patterns:
    - `rzn_core::dsl::Message { action, task_id, task?: { steps[...] }, data?: ... }`
    - `rzn_core::dsl::WorkflowRequest { action, task_id, workflow }` (workflow execution)
    - JSON-RPC bridge compatibility: `{ "jsonrpc":"2.0", "id", "method":"browser.session", "params": { "cmd", "req_id"?, "payload"?, "timeout_ms"? } }`
  - Response:
    - `rzn_core::dsl::ExtensionResponse { action, task_id, success, result?, error? }`
    - JSON-RPC bridge response: `{ "jsonrpc":"2.0", "id", "result": <extension response> }` or `{ "jsonrpc":"2.0", "id", "error": { code, message } }`
  - Targeting (recommended):
    - Prefer encoded IDs from AX/CDP (`frameId:backendNodeId`) + role/name over CSS selectors.

## Implementation Notes
- Connection lifecycle (desktop)
  - Desktop connects to broker (TCP `127.0.0.1:<port>` or named pipe), sends `ping`, then issues tasks.
  - Desktop tracks `session_id` + `current_tab_id` (reused across steps) via `BrokerClient`.
  - If broker is not running, the UX should instruct the user to open Chrome and ensure the extension is enabled (Chrome launches the native host when the extension connects).
  - Broker app-side accepts both legacy task envelopes and JSON-RPC `browser.session`, translating `params.cmd/req_id/payload` into extension-native `cmd/req_id/payload` to keep desktop and CLI flows aligned.

- Observe / inventory
  - Prefer AX slice for compact, semantic targets (roles/names + backend node IDs).
  - If AX is unavailable, fallback to compact DOM outline snapshot with stable-ish selectors.
  - Cache by `{url, dom_hash}`; invalidate on navigation/hash drift.

- Extraction
  - Fast path: code-first extraction (`extract_structured_data`) with a detected list/container + per-item field mapping.
  - Hybrid: `observe` → propose container/item mapping → code extraction.
  - Semantic fallback: small-model JSON extraction from a scoped outline (never ship full HTML unless explicitly needed).

- Actions
  - Execute atomic `StepKind` via the extension’s input ladder:
    - Rung 1: direct DOM methods (fast, minimal side effects)
    - Rung 2: synthetic event sequences (better compatibility)
    - Rung 3: CDP input (trusted events + cross-origin frame routing)
  - When sites check `event.isTrusted`, skip directly to rung 3 based on failure signatures.

- Policy / safety
  - Treat “payments / checkout / irreversible deletes / file downloads/uploads / auth prompts” as *high-risk*.
  - Default behavior: refuse or request explicit user confirmation via a desktop UI prompt; do not rely on model judgment alone.

## Tasks & Status
- [x] Define a stable desktop-facing tool surface (`observe/extract/act/execute_steps`) on top of `BrokerClient`
- [x] Add “extraction plan” DSL (validated JSON, no arbitrary JS execution)
- [x] Replace/remove site-specific profiles and selector lists in hot paths; use generic heuristics
- [x] Add policy gates for high-risk actions with explicit user confirmation hooks
- [x] Add e2e parity flows: observe → extract → act → extract (no external websites)
- [x] Publish a minimal Tauri integration example under `examples/tauri_bridge/`
- [x] Keep broker app-side protocol compatibility for both legacy task envelopes and JSON-RPC `browser.session` requests

## What Works (Do Not Change)
- Length-prefixed transport and broker relay semantics (desktop ↔ broker ↔ extension)
- Tiered execution ladder (DOM → synthesis → CDP) and the “CDP on-demand” posture
- Unified logging contract (`~/rzn_build.log`) and correlation IDs for debugging

## Tried & Didn’t Work
- Pure synthetic DOM events on some sites: fails when sites verify `event.isTrusted` → requires CDP rung.
- Injecting ad-hoc page scripts: often blocked by CSP and harder to reason about → prefer content-script + validated plans.
- Hard-coded, site-specific selectors: brittle and violates the project’s generic targeting guardrails.
