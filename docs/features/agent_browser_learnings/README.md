# Agent-Browser Learnings (Command/Selector/Config Primitives)

## Overview
`agent-browser` (Vercel Labs) is a **client ↔ daemon** browser-automation CLI designed to be AI-friendly via:
- **Snapshot-first element selection** (short refs like `@e12` generated from a snapshot)
- **Daemon persistence** (browser stays alive across commands)
- **First-class safety rails** (domain allowlists, action policies, confirmation flows, output boundaries/truncation)

Implementation note: upstream is a hybrid codebase (Rust CLI + TypeScript daemon, plus an experimental `--native` Rust daemon path).

This doc captures the primitives worth copying into `rzn-browser`, and maps them onto our step schema + runtime.

**Goal**
- Maintain our **JS-first (content script) execution** model for stealth and compatibility.
- Add an *agent-browser-like* “operator surface” (refs, config ergonomics, safety rails) and ensure parity over time.

**Constraints**
- Stealth-first: avoid DOM mutations and CDP unless necessary (“break-glass” only).
- MV3 extension: cross-origin iframes are constrained; native messaging has size limits.
- Guardrails: avoid domain-specific selectors/rules in core code paths.

## Flow Diagrams
**RZN current high-level flow**
```
CLI / Orchestrator → Broker → Extension (SW/background) → Content Script → Page
                  ←        ←                           ←              ←
```

**JS-first with break-glass fallback (recommended parity model)**
```
Step → Content Script executor (DOM / JS)
    → if blocked or cross-origin → Background executor (CDP / chrome.debugger)
    → return result + compact DOM snapshot
```

**agent-browser comparable flow (conceptual)**
```
Rust CLI → Node daemon (Playwright) OR Native daemon (Rust/CDP) → Browser
```

## Decision Record
**Why borrow these primitives**
- The `snapshot → @eN → interact → resnapshot` loop is empirically the best UX for LLMs (low ambiguity, low token cost).
- Config layering (user + project + env + CLI) makes automation reproducible.
- Output boundaries and truncation are pragmatic mitigations for prompt injection/context floods.

**Tradeoffs**
- Refs are ephemeral per snapshot/session; they need a cached mapping.
- Annotated screenshots are extremely useful, but we should implement without DOM mutation when possible (post-process image).

## Architecture
### Relevant `agent-browser` modules (upstream)
- Docs pages (MDX):
  - `docs/src/app/commands/page.mdx`
  - `docs/src/app/configuration/page.mdx`
  - `docs/src/app/selectors/page.mdx`
- Rust CLI (flags/config/output):
  - `cli/src/flags.rs` (config layering + CLI arg parsing)
  - `cli/src/output.rs` (output boundaries + truncation)
- Selector inventory:
  - `src/snapshot.ts` (build refs from ARIA snapshot; dedupe via role+name, optional `nth`)
  - `src/browser.ts` (resolves `@eN` refs to locators)
- Annotated screenshots:
  - `src/actions.ts` (`screenshot --annotate`: uses snapshot refs + bounding boxes, overlays labels)
- Config layering:
  - `cli/src/flags.rs` (merge user + project configs; env/CLI overrides; extensions concatenation)
- Output boundaries/truncation:
  - `cli/src/output.rs` (nonce-based boundary markers; `--max-output`)
- Daemon reliability:
  - `src/daemon.ts` (serializes command handling to avoid socket contention)
  - `cli/src/connection.rs` (retry logic for transient `EAGAIN`/socket errors)

### RZN step schema + runtime entry points
- Step schema: `schema/actions-v1.json`
- Background intercepts (tab mgmt, navigation waiting, screenshot, page source): `extension/src/background.ts`
- Content executor (standard + enhanced handlers): `extension/src/contentScript.ts`
- Policy gating surface: `crates/rzn_plan/src/policy_gate.rs`
- Snapshot prompt surface: `extension/src/content/dom-capture.ts` (`toPrompt`) + `extension/src/contentScript.ts` (`captureEnhancedDOMSnapshot`)
- Snapshot planner/navigator prompts: `crates/rzn_plan/src/prompt_builder.rs`
- Snapshot validated-action → Step mapping: `crates/rzn_plan/src/orchestrator.rs` (`convert_validated_action_to_step`)

## Implementation Notes
### What upstream docs make explicit (worth copying)
**Commands** (from `docs/src/app/commands/page.mdx`)
- “Core” actions cover the obvious primitives: `open`, `click`, `dblclick`, `fill`, `type`, `press`, `hover`, `focus`, `select`, `check/uncheck`, `scroll`, `scrollintoview`, `drag`, `upload`, `screenshot` (`--full` / `--annotate`), `pdf`, `snapshot`, `eval`, `connect`, `close`.
- “Get info” is a first-class surface (`get text/html/value/attr/title/url/count/box/styles`) instead of forcing everything through `eval`.
- “Find …” is a semantic locator layer (role/label/placeholder/alt/title/testid + `first/last/nth`) that *then performs an action* (click/fill/type/hover/focus/check/uncheck/text).
- `wait` supports multiple modalities: element, time, text, URL pattern, load state (`networkidle`), JS predicate, and downloads.
- Safety rails are operationalized as commands: `confirm`/`deny` if `--confirm-actions` is enabled, and `auth save/login` for credential flows.

**Configuration** (from `docs/src/app/configuration/page.mdx`)
- Config is JSON and layered: `~/.agent-browser/config.json` < `./agent-browser.json` < `AGENT_BROWSER_*` env < CLI flags.
- Every CLI flag is settable via a camelCase config key; booleans can be overridden with explicit `true/false` on the CLI.
- Notable “AI safety” knobs: `contentBoundaries`, `maxOutput`, `allowedDomains`, `actionPolicy`, `confirmActions`, `confirmInteractive`.
- Extensions from user + project configs are concatenated (not replaced); env/CLI behave slightly differently (env replaces, CLI appends).
- Auto-discovered config files can be missing (silently ignored); unknown keys are ignored for forward compatibility.

**Selectors** (from `docs/src/app/selectors/page.mdx`)
- Refs are the “blessed path”: `snapshot` yields `[ref=eN]` and commands accept `@eN`.
- Traditional selectors are still supported: CSS, plus `text=…` and `xpath=…`.
- Semantic locators are exposed as `find role …`, `find label …`, `find placeholder …`, `find testid …`, etc.

### The `agent-browser` selector model we should copy
1. Snapshot produces:
   - A **human/LLM-readable tree** (accessibility-oriented)
   - A **ref map** (`e1`, `e2`, …) that persists in the daemon
2. Subsequent commands accept `@eN` as selectors.
3. Duplicates are disambiguated only when needed (`nth`), and hidden when not needed.

**RZN translation**
- Keep our existing stable identifiers (e.g., encoded IDs) internally.
- Add a **per-snapshot alias layer**:
  - `@eN` → `encoded_id` (and/or `TargetSpec`)
  - Stored alongside the latest snapshot in the extension/orchestrator.
- Ensure alias resolution is **purely internal** (never requires site-specific selectors).

### Implemented in RZN (refs + parity steps)
As of **2026-03-04**, we implemented an agent-browser-style ref layer and a few missing parity steps:

- **Snapshot prompt emits `idx` + `ref`** (aligned with our existing index flow):
  - `extension/src/content/dom-capture.ts` emits `idx="0"` and `ref="@e1"` (note: `ref` uses `N = idx + 1`).
- **Ref selector parsing + resolution**:
  - `extension/src/contentScript.ts` accepts `@eN`, `eN`, and `ref=eN` in `selector`.
  - Resolution is backed by `lastEnhancedElements` (from the last enhanced snapshot) and `lastElementMap` caching.
  - Failure mode: `UNKNOWN_REF` instructs taking a fresh snapshot.
- **Prompts teach the LLM to use refs**:
  - `crates/rzn_plan/src/prompts/planner.md`
  - `crates/rzn_plan/src/prompts/navigator.md`
  - `crates/rzn_plan/src/prompt_builder.rs`
- **Added step handlers**:
  - `dbl_click_element`, `get_element_value`, `get_element_count`, `get_element_attribute` in `extension/src/contentScript.ts`
  - `switch_to_tab` in `extension/src/background.ts`
  - `hover_element`, `scroll_element_into_view` (standard handlers) in `extension/src/contentScript.ts`
- **Typing behavior parity upgrades (legacy + workflow surface)**:
  - `fill_input_field` now supports real DOM typing fallback (per-character `keydown/keypress/input/keyup`) when native typing is unavailable or disabled.
  - `delay_ms` is honored for both DOM simulated typing and native typing.
  - `clear_first` is respected in the content executor (`clear_first: false` appends to existing value).
  - `submit_input` now reuses `fill_input_field` semantics, dispatches full Enter key sequence, and performs a conservative form-submit fallback.
  - Step logging for fill/submit now redacts typed payloads (`<redacted:N>`), reducing secret leakage in extension logs.

### LLM planning parity (Tiered snapshot loop)
The goal is that an upstream LLM can plan with high-signal page context, without us dumping raw HTML.

- **Tier 1 (Planner) snapshot context is structured + compact**:
  - Built from `DomSnapshot.elements` (not raw page HTML), grouped by viewport region.
  - Each element line includes `idx`, `ref=@eN`, optional `eid`, key attrs, and a fallback CSS selector.
  - Example: `[12] ref=@e13 eid=0:42 tag=button text="Sign in" attrs(aria-label="Sign in") selector="#login" pos=120,88 size=96x32`
  - Snapshot-derived content is wrapped in `<rzn_untrusted_content>` and guarded by `COMMON_SECURITY_RULES`.
- **Tier 2 (Navigator) understands refs and indexes**:
  - Accepts either `parameters.selector="@eN"` or `parameters.index`.
  - Provides target element context for `selector`, `index`, and `drag_and_drop` selectors.
- **Snapshot loop can execute a broader action surface**:
  - The validated-action → `StepKind` mapping now supports waits, screenshots, assertions, upload, drag/drop, tabs, and back/forward/reload (via `execute_javascript`).
  - This keeps the “LLM plans, we execute” loop close to the workflow step surface.

### Annotated screenshots (worth copying, but differently)
`agent-browser` injects an overlay DOM element before screenshot capture.

**RZN recommendation**
- Prefer post-processing the screenshot bytes (draw boxes + numbers) to avoid DOM mutations.
- Use the same `@eN` alias numbering as the snapshot output.

### Safety rails
`agent-browser` treats these as core:
- Output boundaries with a per-process nonce
- Output truncation (`--max-output`)
- Action allow/deny/confirm policies
- Domain allowlists (including blocking subresource + WS-ish channels)

**RZN today**
- We already have `PolicyGate` (block/confirm/allow) at the orchestrator layer.
- We now wrap snapshot-derived content in `<rzn_untrusted_content>` with `COMMON_SECURITY_RULES` in the system prompt; output boundary markers for other page-sourced content are still worth adding.

## Command Parity: `agent-browser` → RZN steps
This table maps *agent-browser command primitives* to our canonical `schema/actions-v1.json` step types and notes implementation status.

Legend:
- ✅ implemented (content) = handled in `extension/src/contentScript.ts`
- ✅ implemented (background) = handled/intercepted in `extension/src/background.ts`
- ⚡ enhanced-only = requires enhanced path (`<type>_enhanced`) / `target_spec` / `use_enhanced`
- ❌ missing = present in schema but not implemented in extension runtime

| agent-browser primitive | Example | RZN step type(s) | Status | Notes / gating |
|---|---|---|---|---|
| Navigate | `open <url>` | `navigate_to_url` | ✅ background (waits) | Policy: may require confirmation for “checkout/payment-ish” URLs |
| Tabs | `tab new [url]`, `tab close`, `tab <n>` | `open_new_tab`, `close_current_tab`, `switch_to_tab` | ✅ background | `switch_to_tab` supports numeric and string identifiers |
| URL | `get url` | `get_current_url` | ✅ background | — |
| Click | `click <sel>` / `click @eN` | `click_element` | ✅ content + ⚡ enhanced + (optional) background CDP | Break-glass CDP exists via `use_cdp: true` |
| Double-click | `dblclick <sel>` | `dbl_click_element` | ✅ content | JS-synthesized dblclick (no CDP needed) |
| Hover | `hover <sel>` | `hover_element` | ✅ content | JS-synthesized hover (pointer/mouseover/move); supports `@eN` |
| Fill / type | `fill <sel> <text>` / `type <sel> <text>` | `fill_input_field` | ✅ content + ⚡ enhanced | PolicyGate may require confirmation for auth-ish fields (password/otp hints) |
| Submit | `press Enter` | `submit_input` / `press_special_key` | ✅ content / ✅ background+content | `submit_input` is implemented; `press_special_key` exists |
| Scroll window | `scroll down 200` | `scroll_window_to` | ✅ content | — |
| Scroll into view | `scrollintoview <sel>` | `scroll_element_into_view` | ✅ content | Standard handler uses `scrollIntoView()`; supports `@eN` |
| Wait | `wait <sel>` / `wait <ms>` | `wait_for_element`, `wait_for_timeout`, `wait_for_navigation`, `wait_for_network_idle` | ✅ content+background | `wait_for_network_idle` is best-effort (no webRequest); bounded by `max_wait_ms` |
| Snapshot (refs) | `snapshot` | (no 1:1 step) | ✅ implemented | DOM snapshot prompt includes `idx` + `ref`; steps can use `@eN` |
| Screenshot | `screenshot [--annotate]` | `take_screenshot` | ✅ background+content | `annotate: true` draws `@eN` labels on the captured bitmap (no DOM overlays) |
| Page source | `content` / HTML | `get_page_source` | ✅ background | Also fetches DOM snapshot optionally |
| Get element text | `get text <sel>` | `get_element_text` | ✅ content + ⚡ enhanced | — |
| Get element value/count/attr | `get value/count/attr` | `get_element_value`, `get_element_count`, `get_element_attribute` | ✅ content | `get_element_count` treats `@eN` as 0/1 |
| Eval JS | `eval <js>` | `execute_javascript` | ✅ content | PolicyGate currently **blocks** this by default |
| Same-origin request | (n/a) | `same_origin_request` | ✅ content | PolicyGate: confirmation required (exfil/modification risk) |
| Downloads | `download …` | `download_images` | ✅ content | PolicyGate: confirmation required |
| Uploads | `upload …` | `upload_file` | ✅ background (CDP) | Uses `DOM.setFileInputFiles` (break-glass); PolicyGate confirmation required |
| Cookies / storage | `cookies …`, `storage …` | cookie + localStorage steps | ✅ content | Best-effort via `document.cookie` + `localStorage` (no HttpOnly cookie access); PolicyGate confirmation required |
| Popups / captcha | (n/a) | popup/captcha/wait-for-* | ✅ content | These are RZN-specific; agent-browser doesn’t ship equivalents |

## Current RZN coverage (schema step types)
As of **2026-03-04**, step coverage in the extension runtime:

- Implemented: `navigate_to_url`, `open_new_tab`, `close_current_tab`, `switch_to_tab`, `get_current_url`, `click_element`, `dbl_click_element`, `hover_element`, `fill_input_field`, `submit_input`, `press_special_key`, `select_option_in_dropdown`, `upload_file` (CDP), `drag_and_drop`, `scroll_window_to`, `scroll_element_into_view`, `infinite_scroll`, `wait_for_timeout`, `wait_for_element`, `wait_for_navigation`, `wait_for_network_idle`, `extract_structured_data`, `get_element_text`, `get_element_value`, `get_element_count`, `get_element_attribute`, `take_screenshot`, `get_page_source`, `assert_selector_state`, `assert_text_in_element`, `assert_url_matches`, `execute_javascript`, `same_origin_request`, `set_cookie`, `get_cookies`, `clear_cookies`, `set_local_storage_item`, `get_local_storage_item`, `clear_local_storage`, `download_images`, `simulate_human_behavior`, `detect_popups`, `dismiss_popups`, `wait_for_no_popups`, `handle_captcha`, `configure_captcha_solver`, `request_user_intervention`, `wait_for_auth`, `wait_for_totp`, `wait_for_verification`, `extract_page_assets`.

## Tasks & Status
- [x] Add `@eN` alias layer to snapshot prompt + step parsing (alias → `encoded_id` / `TargetSpec`)
- [x] Implement `switch_to_tab` (background)
- [x] Implement `dbl_click_element` and `get_element_value/count/attribute` (content)
- [x] Implement standard `hover_element` + `scroll_element_into_view` (content)
- [x] Add “annotated screenshot” mode that labels refs (post-processed image; no DOM overlays)
- [x] Add untrusted content wrappers + truncation for snapshot prompts (LLM safety)
- [x] Improve snapshot planner/navigator prompts for ref-first targeting
- [x] Expand snapshot validated-action → `StepKind` mapping (planner parity)
- [x] Implement `wait_for_navigation` / `wait_for_network_idle` (agent-browser `wait --load networkidle`)
- [x] Require confirmation for `same_origin_request` by policy (exfil/modification risk)
- [x] Add smoke workflows: `workflows/tests/agent-browser-parity-smoke.json` + `workflows/tests/upload-file-local-smoke.json`
- [x] Close typing parity gap in extension executor (`fill_input_field` / `submit_input`) for DOM per-char typing + append semantics + delay handling
- [x] Add e2e regression for typing append + key/input event emission: `extension/tests/e2e/actions.spec.ts`
- [x] Propagate `clear_first` end-to-end through typed Rust `StepKind::FillInputField` (`rzn_core` step generator + planner/SDK/CLI mappings now carry it)

## What Works (Do Not Change)
- Stable step schema: `schema/actions-v1.json`
- Policy gating defaults in `crates/rzn_plan/src/policy_gate.rs` (block `execute_javascript` by default)
- “Break-glass CDP” path for trusted clicks via `use_cdp: true`
- Ref convention: `idx` is 0-based and `ref="@e{idx+1}"` (LLM-facing)

## Tried & Didn’t Work
- N/A (this is a learnings + parity mapping document; implementation experiments pending).
