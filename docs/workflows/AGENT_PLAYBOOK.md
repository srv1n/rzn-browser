---
title: Agent Playbook for Workflow Work
slug: /workflows/agent-playbook
---

# Agent Playbook — Building & Auditing Workflows

Read this before touching any `workflows/<system>/*.json`. Every item below is a lesson paid for in tokens and failed runs. Following them keeps future sessions cheap.

This is a **process and trap-list** doc, not a spec. For schema and design rules see `AGENTS.md` → "Tool & Workflow Design Rules".

---

## Phase 1 — Before you write anything

1. **Probe the live DOM first.** Use the Claude-in-Chrome MCP tool (`tabs_context_mcp` → `javascript_tool`) on the real authenticated page. Selectors guessed from screenshots or memory are almost always wrong. Single most common cause of wasted iterations.
2. **Audit existing workflows for overlap.** `ls workflows/<system>/`. If a new workflow differs only by a flag, add a param to the existing one (consolidation rule). One workflow per distinct capability.
3. **Sanity-check the user gesture surface.** Ordinary DOM reads, JS extraction, clicking, and typing should stay non-CDP. Downloads, popups, file pickers, trusted paste, and hostile rich composers may need explicit CDP. See "CDP traps" below.
4. **Don't bump file names with `-v1`, `-v2`.** `id` and `version` live inside the JSON. Renaming files breaks installs.

---

## Phase 2 — Writing the JSON

### Required scaffolding
- `id`, `name`, `description`, `domain`, `version` (semver), and a `help` block with `summary`, `parameters`, `examples`, `returns`, `notes`. `rzn-browser workflow validate <system> <workflow>` must pass with **0 errors**.
- Every param needs `name`, `required`, `shape`, `description`, `example`. Use enums (`mode: full | latest-assistant | latest-user`) instead of bare booleans when more than two modes are plausible.

### `execute_javascript` patterns

- **Always include the `cleanArg` helper at the top of every script.** Unset params arrive as the literal string `{param_name}` because the engine doesn't substitute. Without this, your "optional" param is never actually optional.
  ```js
  const cleanArg = (v) => { const t = String(v == null ? '' : v).trim(); return /^\{[a-zA-Z0-9_]+\}$/.test(t) ? '' : t; };
  ```
- **`execute_javascript` is JS-first and defaults to `world: "main"`.** Keep `world: "main"` in workflow JSON when clarity matters; use `eval_isolated_world` or `world: "isolated"` only when you intentionally do not need page-world state.
- **Do not mark JS steps as CDP unless the browser requires a trusted gesture.** `use_cdp_eval: true` forces the debugger banner. For parameter-gated downloads, use `use_cdp_eval_when_arg_truthy: <arg_index>` so the banner appears only when that parameter is enabled.
- **One trusted browser gesture per CDP eval.** Multiple gesture-required actions (downloads, `window.open`, `navigator.clipboard.write`) can silently fail beyond the first. Solution: split into separate steps. See the `claude_export_chat.json` artifact-download pattern for conditional CDP download clicks.
- **Don't mutate state and then immediately re-evaluate from another step.** When you click a model picker option, change a preference toggle, or trigger any "save"-style action, the app fires a background network request. When the response arrives ~1–3 s later, React re-renders and can **destroy the page execution context**. The next evaluate may throw `-32000 "Promise was collected"`. Two fixes:
  1. Wait inside the same evaluate (`await sleep(3000)`) before returning, so the destruction happens before the next step starts.
  2. Add a `wait_for_timeout` step of ≥ 3000 ms between the change and the next evaluate.
- **Don't dispatch Escape blindly.** Global handlers may pick it up (close modal, navigate away). Prefer clicking the trigger button again to toggle a menu closed.
- **For SPA navigation, fire-and-forget then return.**
  ```js
  setTimeout(() => window.location.assign(target), 0);
  return { redirected: true, target };
  ```
  Awaiting `location.assign` synchronously inside an eval causes `Inspected target navigated or closed` because the context dies before the promise resolves.

### DOM gotchas (real ones we hit)

- **`<input role="switch" class="sr-only">`** uses the `.checked` DOM property, **not** the `aria-checked` attribute. The attribute is `null` even when the switch is on. Misreading this means you toggle in the wrong direction.
- **ProseMirror / contenteditable composers** need `document.execCommand('insertText', false, text)`. Setting `.textContent` or `.value` doesn't fire the editor's input plumbing and the Send button stays disabled.
- **The Send button only appears after text is in the composer.** Poll for it after typing, don't query it upfront.
- **Aria-labels are often the most stable read of state.** A "Model: Opus 4.7 Adaptive" label tells you the current model + adaptive thinking state without opening any menu. Read it first; skip the menu entirely if no change is needed.
- **File inputs cannot be set from page JS.** Browsers block this. The only reliable path is CDP `DOM.setFileInputFiles` via the existing `upload_file` action. Don't attempt a JS workaround.
- **Class names from Tailwind/utility CSS rot.** Prefer `data-testid`, `role`, `aria-label`, and structural relationships. Worst-case use class substrings (`[class*='font-claude-response']`).
- **Strip ARIA noise from extracted text.** `sr-only`, hidden buttons, `[aria-hidden='true']`, `<style>`, `<script>`, `<svg>` — clone the node and remove these before reading `innerText` if you want clean transcript.

---

## Phase 3 — Validation

1. **Structural validate** — `rzn-browser workflow validate <system> <workflow>`. Must report `0 errors, 0 warnings`. This catches schema drift but **does not** prove the workflow runs.
2. **End-to-end via CLI** — `rzn-browser run <system> <workflow> --param ...`. **Mandatory** before declaring done. A workflow that validates but fails E2E is a regression.
3. **Reinstall after every JSON edit** — `rzn-browser workflow pull --repo-root .`. The CLI runs from `~/Library/Application Support/RZN/workflows/builtin/` (a copy, not a symlink). Without `pull`, your edit doesn't reach the runtime.
4. **Test every param permutation that touches different code paths.** Adding `model_slug` as a no-op param is not enough — actually pass it through the menu-open path. We shipped a "passing" `send` workflow that broke for any user who set `adaptive_thinking`.
5. **Regression-test the bare baseline.** After adding a feature param, run the workflow without it to confirm the original path still works.

---

## Phase 4 — Common error decoder

| Error / symptom | Likely cause | Fix |
| --- | --- | --- |
| `-32000 Promise was collected` | Page execution context destroyed during evaluate (state-change re-render, navigation, iframe detach). | Add a wait of ≥ 3 s after the state-changing action. Or move the wait inside the evaluate that did the change. |
| `Inspected target navigated or closed` | Awaited a navigation inside an eval. | Fire-and-forget: `setTimeout(() => location.assign(url), 0); return ...;` |
| `Debugger is not attached to the tab with id` | A CDP action lost its debugger attachment. | Keep ordinary steps JS-first. If forced to use CDP (`upload_file`, trusted click/type/eval), let the engine reattach per explicit CDP step. |
| `UPLOAD_FILE_SELECTOR_NOT_FOUND` | The file input isn't in the document at click time. | Add a `wait_for_element` for the input, or trigger the modal that mounts it before the upload step. |
| Silently only the first download lands | Multiple downloads consumed one trusted gesture. | One download click per conditional CDP eval step. See `claude_export_chat.json`. |
| Workflow validates but `--param x="..."` does nothing | Script doesn't apply `cleanArg`; the literal `{x}` is being treated as a real value. | Add the `cleanArg` boilerplate at the top of every script. |
| CLI hangs on "Timed out waiting for native host connection" | Stale `rzn-browser-worker` from a prior session. | `pkill rzn-browser-worker` then retry. (Long-term: `docs/features/connection_reliability/`.) |

---

## Phase 5 — When to ask vs. proceed

- **Ask before** scope expands to engine work (extension code, native host, new step types). Workflows that need engine help should be flagged early — handed off as feature scratchpads.
- **Don't ask** for workflow-internal decisions (selector choice, param naming, wait timing). Probe, decide, ship, test.
- **Always say what was tested.** Listing CLI commands you ran is the proof. Structural validation alone is not.

---

## Cross-references

- `AGENTS.md` → "Tool & Workflow Design Rules" — the consolidation/naming/description rules.
- `docs/features/connection_reliability/README.md` — handoff spec for the broker/worker handshake bug.
- `docs/features/workflow_engine_improvements/README.md` — engine-level fixes that would obsolete several of the patterns above.
