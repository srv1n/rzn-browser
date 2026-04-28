- **Overview**
- Goal: Recreate the "AI copilot" browser side-panel experience demonstrated in public demos by leveraging our MV3 extension, adding a minimal side-panel UI, and reusing the broker + unified logging. Keep the same primitives; focus on UX (inline actions, highlights, step feedback).
- Constraints: MV3; maintain stealth behavior and minimal permissions; no hard dependency on a hosted service.

**Flow Diagrams**
`Side Panel UI ↔ Content Script ↔ Service Worker ↔ Broker ↔ rzn-browser`

- **Decision Record**
- Start with programmatic parity (CLI surfaces) and layer on UI parity (side panel + highlight).
- Prefer postMessage bridge we already expose; avoid direct CDP in the panel.

**Architecture**
- Modules:
  - `extension/src/panel/` new side panel (Vite/TS) with action log and retry.
  - `extension/src/bridge/` reuse `window.__rznExecuteStep` / `captureEnhancedDOMSnapshot`.
  - `rzn_broker` remains the transport hub.
- Data contracts:
  - Panel → bridge: `{ type: 'act'|'observe'|'extract'|'agent', payload }`.
  - Bridge → panel: step results, errors, and snapshots.

**Implementation Notes**
- Add “highlight” overlay for observed candidates and executed selectors.
- Provide provider selection in panel (OPENAI/GEMINI/Anthropic); reuse `.env` defaults.
- Persist transcripts to `~/rzn_build.log` and show last N in the panel.

**Tasks & Status**
- [ ] Side panel bootstrap and toggle.
- [ ] Highlight overlay on act/observe.
- [ ] Panel controls: act input, observe list, extract schema paste.
- [ ] Connect to `rzn-browser` for agent runs (optional; start with in-page act/observe/extract).
- [ ] E2E: Playwright validates side panel actions on test pages.

**What Works (Do Not Change)**
- Existing extension build/test pipeline and messaging.

**Tried & Didn’t Work**
- Direct CDP from panel (blocked by MV3); keep using broker.
