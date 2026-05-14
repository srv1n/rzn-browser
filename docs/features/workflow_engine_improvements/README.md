# Workflow Engine Improvements — Pickup Backlog

## Overview

This is a **prioritized backlog** of engine improvements derived from a real workflow-build session (Claude connector — `recent-chats`, `export-chat`, `send`). Each item is something a workflow author currently has to **work around**, with measurable token and time cost. Implementing any of these obsoletes a class of workarounds and shortens future workflow PRs.

Owner audience: the broker / extension / native host / CLI team.

Format: `pain → fix sketch → who benefits → rough scope`.

---

## Flow Diagrams

```
                                  Workflow author
                                        │
                                        ▼
         ┌─────────────────────────────────────────────────────────┐
         │  Workflow JSON (per system) — execute_javascript steps  │
         └─────────────────┬───────────────────────────────────────┘
                           │ pull / install
                           ▼
                ┌──────────────────────┐
                │  rzn-browser CLI     │
                └──────────┬───────────┘
                           │ broker socket
                           ▼
                ┌──────────────────────┐         ┌─────────────────────┐
                │  rzn-browser-worker  │ ◄──── ► │  rzn-native-host    │
                └──────────┬───────────┘         └──────────┬──────────┘
                           │                                │ stdio
                           │                                ▼
                           │                     ┌─────────────────────┐
                           └── JS-first / CDP ─► │  Chrome extension   │
                                                 │  (background.ts)    │
                                                 └──────────┬──────────┘
                                                            │ page bridge / chrome.scripting
                                                            │ explicit CDP Runtime.evaluate
                                                            ▼
                                                  ┌──────────────────┐
                                                  │  Page (main)     │
                                                  └──────────────────┘
```

The improvements below target every layer except the page itself.

---

## Decision Record

Why a wishlist doc instead of opening N issues: each item is small individually, but they share a common motivation (reducing per-workflow boilerplate / surfacing better errors). Bundling them lets the engine team batch-prioritize and pick the high-leverage subset. Several items have alternative implementations — flagged inline.

---

## Backlog (priority order)

### 1. Surface "page context destroyed" with an actionable error

**Status.** Implemented for `Promise was collected`, `Inspected target navigated or closed`, execution-context loss, and `Detached while handling command`. Detach during CDP commands is treated as lifecycle churn, clears cached frame/session routes, and is logged as a warning instead of polluting the extension error page.

**Pain.** The CDP error `-32000 Promise was collected` shows up when a state-changing action triggers a React re-render that destroys the execution context mid-evaluate. Currently this surfaces verbatim. Workflow authors waste 30+ min the first time they hit it (we did) because the message gives zero hint about cause or fix. Eventual workaround is "add a 3 s wait" — discoverable only by experimentation.

**Fix sketch.** In `extension/src/background.ts` `runCdpEval`: catch `-32000 / Promise was collected`, `Inspected target navigated or closed`, context loss, and debugger-target detachment, then rethrow as a structured error: `EXEC_CONTEXT_DESTROYED: page execution context died during await`. The CLI prints the structured `code` plus the suggestion.

**Who benefits.** Every workflow author hitting this for the first time. Free token savings.

**Scope.** Single function, ~20 lines. Extension-only.

---

### 2. `for_each` / `repeat` step type

**Pain.** Multi-download in `claude_export_chat.json` required hardcoding 8 click steps + 8 wait steps because each click needs its own CDP user gesture (one per evaluate). Authors who don't know the limit copy a single loop into one `execute_javascript` and hit silent failure. Authors who do know it write 16 nearly-identical step blocks.

**Fix sketch.** New step type in `crates/rzn_plan/`:
```json
{ "id": "s5_dl", "type": "for_each",
  "items_from": "step.s4.result.artifacts",
  "max_iterations": 16,
  "step": {
    "type": "execute_javascript",
    "args": ["{loop.index}"],
    "script": "..."
  },
  "between_iterations": { "type": "wait_for_timeout", "timeout_ms": 700 }
}
```
Each iteration spawns a fresh CDP evaluate — preserves the user-gesture-per-step property naturally.

**Who benefits.** Any connector that downloads multiple assets, opens multiple modals, or iterates list rows. Currently every such workflow either hardcodes N or sits in single-evaluate failure mode.

**Scope.** Workflow schema + orchestrator changes. Medium (1 day).

---

### 3. Auto-substitute unset params instead of literal `{name}`

**Pain.** Every `execute_javascript` script has to start with the `cleanArg` boilerplate to detect the literal `{thread_id}` placeholder when the param is unset. Forgetting this means optional params behave as the string `"{thread_id}"`. We hit it twice in the Claude session.

**Fix sketch.** In the CLI's param-substitution layer, replace unset `{param}` placeholders with empty string before passing to the workflow. Or pass `null` and let `args[i]` be `null` in the script.

**Who benefits.** Every author. Removes ~10 lines of boilerplate per script.

**Scope.** Param substitution function in `crates/rzn_browser/src/`. Small (~2 hours), but needs an audit of existing workflows that may rely on the current behavior.

---

### 4. CLI prints the richest step result, not just the last

**Pain.** Workflows now end on a small wait/bookkeeping step, so the useful payload (transcript, manifest) gets printed mid-run and scrolls off. Workaround: add a final "consolidation" `execute_javascript` step that re-reads everything. Wasteful.

**Fix sketch.** Either (a) print every step's result as JSON-lines and let the caller pick, (b) support a `final_result_step: <id>` field in the workflow root, or (c) compute and print a structured aggregate `{ steps: { s4: {...}, s6: {...} } }` so the caller never loses data. (b) is the smallest change.

**Who benefits.** Every multi-step workflow. Avoids ~20 lines of duplicated JS per workflow.

**Scope.** CLI output formatting + workflow schema. Small.

---

### 5. Step-level `condition` field

**Pain.** The 8 hardcoded download-click steps each contain `if (idx >= btns.length) return { skipped: true }`. Wasted CDP round-trips when there are only 3 artifacts but 8 steps run.

**Fix sketch.** Step schema: `condition: "step.s4.result.artifacts.length > 5"` evaluated by the orchestrator (simple JSONPath/JMESPath). Step skipped when false.

**Who benefits.** Loop-style workflows, branchy workflows (e.g. only run "dismiss popup" if popup detected).

**Scope.** Orchestrator change. Small. Could ship together with #2.

---

### 6. `chrome.downloads`-aware download primitive

**Pain.** `download_artifacts` on `export-chat` returns a manifest of *triggered* downloads, not *completed* ones. Caller has no programmatic way to know when files are saved or to route them to a folder outside `~/Downloads`. We deliberately scoped to "manifest only" because the proper fix needs engine support.

**Fix sketch.** New extension action `click_and_capture_download`:
- Params: `selector` (button), `target_folder` (absolute path or relative-to-Downloads).
- Listens once on `chrome.downloads.onCreated` → `onDeterminingFilename` → `onChanged{state:complete}`.
- For paths under `~/Downloads`, suggests filename via `onDeterminingFilename`.
- For absolute paths outside `~/Downloads`, completes the download to a temp subfolder and forwards a move request to the native host.
- Returns `{ filename, bytes, final_path, content_type }`.

**Requires.** New native-host command for file move (small Rust change in `crates/rzn_native_host/`).

**Who benefits.** Every connector that exports user artifacts (Claude, ChatGPT exports, design tools, screenshot tools, scrapers). Today these all bottom out at "files appear somewhere in `~/Downloads`, good luck".

**Scope.** Medium (1–2 days). Would justify a separate scratchpad before build.

---

### 7. Explicit CDP debugger lifecycle

**Pain.** The old design treated broad JS eval as CDP-adjacent, so the debugger banner appeared in workflows that only needed DOM reads or plain JavaScript. It also made real CDP actions sensitive to attach/detach timing.

**Fix sketch.** Keep `execute_javascript` JS-first through page bridge / `chrome.scripting`. Centralize explicit CDP paths in `CdpSessionManager`, attach just in time for `upload_file`, trusted click/type/key, AX/CDP reads, and eval steps marked `use_cdp_eval`, then release/expire promptly.

**Who benefits.** Every workflow. Most avoid the debugger banner entirely; the few that need trusted browser gestures still work.

**Scope.** Medium. Already specced.

---

### 8. Shadow-DOM-aware querySelector helper

**Pain.** Every workflow that needs to find an element in a shadow root re-implements `qsDeep` (we have a copy in `extension/src/actions/upload_file.ts`). Different copies, different bugs.

**Fix sketch.** Inject a `__rzn.qsDeep(selector)`, `__rzn.qsAllDeep(selector)` helper into the page world before user script runs. Authors write `__rzn.qsDeep('input[type=file]')` instead of inlining the walk.

**Who benefits.** Sites using shadow DOM (modern web components, some Google properties, Zendesk widgets, etc.).

**Scope.** Small. ~30 lines in extension.

---

### 9. Snapshot-on-failure improvements

**Pain.** When a step fails, we get `dom_hash=<8 chars>`. To debug, we need the actual DOM, screenshot, console log, and network state. Currently those require a manual rerun with extra instrumentation.

**Fix sketch.** When a step throws, automatically write to a debug folder (`~/.rzn/debug/<workflow>/<run_id>/`):
- Full outerHTML of `<body>`
- PNG screenshot
- Last 200 console messages
- Last 50 network requests
- Step args, step script, error text
Print the path in the CLI error.

**Who benefits.** Every author chasing a failure. Today the round-trip is "rerun with `--snapshot=full`, parse output, repeat" — we could one-shot it.

**Scope.** Medium. Touches extension + CLI.

---

### 10. `world: "main"` default

**Pain.** Most workflow scripts need to see page-world React state, so they all set `world: "main"`. The old isolated default was rarely useful for UI automation.

**Fix sketch.** Landed for the workflow/broker and content-script paths. `execute_javascript` now defaults to main-world JS-first execution; isolated eval remains opt-in.

**Who benefits.** Future workflow authors. Removes a class of "I forgot the flag" bugs.

**Scope.** Small. Docs + default change. Audit for breakage.

---

## Architecture (where each lands)

| Item | Layer | Files (approx) |
| --- | --- | --- |
| 1 (error decoder) | extension | `extension/src/background.ts` runCdpEval |
| 2 (`for_each`) | engine | `crates/rzn_plan/src/`, schema |
| 3 (param subst) | CLI | `crates/rzn_browser/src/` |
| 4 (final_result) | CLI + schema | `crates/rzn_browser/src/main.rs` output formatting |
| 5 (condition) | engine | `crates/rzn_plan/src/orchestrator.rs` |
| 6 (download primitive) | extension + native host | `extension/src/actions/`, `crates/rzn_native_host/src/` |
| 7 (CDP lifecycle) | extension | `extension/src/background.ts`, `extension/src/runtime/cdp_session_manager.ts` |
| 8 (shadow qs helper) | extension | injection in JS-first eval preamble |
| 9 (snapshot+) | extension + CLI | snapshot writer + CLI flag |
| 10 (world default) | extension | default in `runCdpEval` |

---

## Implementation Notes

- **Test infra.** The Claude connector has 3 workflows that exercise: pure read (`recent-chats`), state-changing read (`export-chat` with download_artifacts), state-changing write (`send`). It's a good regression target for items 1, 4, 5, 7.
- **Backwards compat.** Items 3 and 10 change defaults — both need a one-pass audit of existing workflows. Easier to ship as part of a coordinated workflow `version` bump.
- **Sequencing.** 1 (error decoder) is highest leverage / lowest cost — ship first. 2 (`for_each`) + 5 (`condition`) are natural pair. 6 (downloads) wants its own scratchpad.

---

## Tasks & Status

- [x] **#1** Structured error for context destruction — implemented in `runCdpEval`
- [ ] **#2** `for_each` step type
- [x] **#3** Auto-substitute unset params — implemented for `execute_javascript.args`
- [ ] **#4** Final-result step or per-step JSON-lines output — _pickup-ready_
- [ ] **#5** Step-level `condition`
- [ ] **#6** `click_and_capture_download` action — _needs scratchpad_
- [x] **#7** JS-first eval + explicit CDP lifecycle — landed for broker/content eval, upload, trusted click/type, and conditional CDP eval
- [x] **#8** `__rzn.qsDeep` shadow-DOM helper — implemented for `execute_javascript`
- [ ] **#9** Snapshot-on-failure expansion
- [x] **#10** Flip `world: "main"` default — landed for `execute_javascript`

---

## What Works (Do Not Change)

- `execute_javascript` arg passing via positional `args` array — clean and works.
- `upload_file` CDP path with `DOM.setFileInputFiles` — only reliable way to drive file inputs; do not regress.
- Workflow `pull --repo-root .` install model — fast feedback loop for authors.
- `dismiss_popups` action — quietly handles cookie banners across most sites.
- The unified log at `~/rzn_build.log` — useful when it has detail; keep it.

---

## Tried & Didn't Work

- **Closing a Claude menu via Escape dispatched to `document`.** Too easy to be caught by other handlers. We switched to clicking the trigger button again to toggle closed.
- **Detecting menuitem state via `aria-checked`.** Claude's switches use `<input type="checkbox" role="switch" class="sr-only">` with the `checked` DOM property and no `aria-checked` attribute. Cost us a debugging cycle.
- **Single eval looping over `.click()` for downloads.** One trusted gesture is not a batch-download API. Solution was N separate conditional CDP eval steps.

---

## Cross-references

- `docs/workflows/AGENT_PLAYBOOK.md` — the user-facing version of these lessons (workflow author perspective).
- `docs/features/connection_reliability/README.md` — broker/worker handshake spec; #7 here is its sibling.
- `AGENTS.md` → "Tool & Workflow Design Rules" — the workflow consolidation rules these improvements support.
