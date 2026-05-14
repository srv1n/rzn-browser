# Repository Guidelines

This repository implements stealth-first browser automation with a Rust workspace and a Chrome extension. Use this guide to build, test, and contribute changes consistently.

## Fast Iteration (Single-User, Main-Only)
- Always work directly on the `main` branch (no feature branches) unless the human explicitly asks otherwise.
- Do **not** run mutating git operations unless explicitly requested by the human (this includes `git commit`, `git push`, `git pull`, `git rebase`, `git merge`, `git checkout`, `git reset`, `git stash`, etc.).
- Read-only git commands are OK for visibility (`git status`, `git diff`, `git log`, `git show`).

## Tool & Workflow Design Rules (MUST READ before building or editing any workflow or agent-facing tool)

Source: Anthropic, "Writing tools for agents" — https://www.anthropic.com/engineering/writing-tools-for-agents. These rules apply to rzn-browser workflows (`workflows/<system>/*.json`), CLI surfaces, MCP tools, and any function an agent can call.

**Also read before workflow work:** `docs/workflows/AGENT_PLAYBOOK.md` — process + DOM/CDP traps + error decoder distilled from prior sessions. Saves tokens on rediscovery.

**Consolidation:**
1. Build few, thoughtful tools — not an exhaustive tool for every endpoint. One workflow per distinct capability.
2. Before creating a new workflow, check whether an existing one can cover it via a new parameter, enum, or mode flag.
3. If two tool descriptions overlap, the tools overlap — merge them or sharpen the line.
4. Acceptable reasons to split: a fundamentally different output contract, a side-effect class that callers need to reason about separately (pure read vs. write), or a top-level runtime flag that genuinely can't be expressed as a param. If the engine is the blocker, patch the engine — don't ship the split. (Historical example: we patched `upload_file` to no-op on empty paths rather than keep a separate `send-attachment` workflow.)
5. Do not ship speculative tools (debug-only, inspect, single-file download) unless a real caller needs them.

**Naming & parameters:**
6. Namespace by system and resource. `claude/export-full-chat-v1` beats `export_chat`.
7. Use unambiguous parameter names: `thread_id` not `id`, `attachment_file_path` not `file`.
8. Prefer enums over booleans when more than two modes are plausible (`mode: full | latest-assistant | latest-user`).

**Descriptions:**
9. Write descriptions like you're briefing a new hire: what it does, when to use it, when not to use it. Make implicit context explicit.
10. Small wording tweaks produce outsized quality changes — iterate on descriptions.
11. Every workflow ships a `help` block: `summary`, `parameters`, `examples`, `returns`, `notes`. `rzn-browser workflow validate` must pass with zero errors before a workflow is considered done.

**Responses:**
12. Return only high-signal fields. Drop internal UUIDs, mime types, thumbnail URLs, and similar noise unless a caller actually needs them.
13. Truncate large payloads with a clear pointer to a more token-efficient strategy (pagination, mode flag, scoped query).
14. Resolve opaque identifiers to human-readable strings in responses where possible.

**Process:**
15. Hand-inspect the live DOM / live API before writing selectors or parsers — don't guess.
16. After building, run the tool end-to-end before declaring success. Structural validation is necessary but not sufficient.
17. Cookie/consent banners and other fixed-position overlays do **not** block DOM extraction (`querySelectorAll` reads through them), but they **do** intercept clicks and focus on landing-page forms. Prefer direct URL navigation (`?qs=…`, `?q=…`, `?search=…`) over "click search box → fill → press Enter" flows on any site that greets visitors with a consent overlay. Historical landmine: ScienceDirect's homepage form-submit was silently dropped because the banner ate the click; switching to `/search?qs={query}` fixed it.

## Project Structure & Modules
- `crates/`: Rust workspace crates (`rzn_browser`, `rzn_core`, `rzn_plan`, `rzn_sdk`, `rzn_native_host`, `rzn_browser_worker`).
- `extension/`: MV3 extension (TypeScript, Vite, Vitest, Playwright). Build outputs in `dist-chrome/`.
- `workflows/`: JSON workflows runnable via the CLI/scripts.
- `resources/`: runtime metadata and connector assets such as system metadata and social card catalogs.
- `scripts/`: developer tooling, release tooling, guards, and helper entrypoints.
- `examples/`: reference integrations and non-core experimental helpers.
- `test/` and `tests/`: HTML/manual test pages and Rust integration/unit tests.
- `.env.example`: Copy to `.env` to configure LLM provider and runtime options.

## Build, Test, and Development
- Build all: `make build` (runs Rust build + extension build).
- Rust only: `cargo build --release`
- Extension: `cd extension && bun install && bun run build`
- Run tests (Rust): `make test` or `cargo test`
- Extension unit tests: `cd extension && bun x vitest`
- Extension e2e tests: `cd extension && bun x playwright test`
- Run a workflow: `rzn-browser run google search --param search_query="rust"`
- Logs: `make logs-follow` (tail unified log), `make logs-clear`

## Coding Style & Naming
- Rust: format with `rustfmt`; prefer `clippy` clean. Files and modules `snake_case`; types `PascalCase`; functions/vars `snake_case`.
- TypeScript: 2‑space indent; types/interfaces `PascalCase`; functions/vars `camelCase`. Follow existing file naming in `extension/src/`.
- JSON: 2‑space indent; trailing commas discouraged in workflow files.

## Agent Guardrails (Targeting and Heuristics)
- Do not add site-specific selectors or domain-tuned rules to code paths. Keep targeting generic: encoded IDs from CDP, accessibility roles, repeated-list detection, and same-origin preference only.
- If a scenario seems to require site knowledge, design a generic heuristic (e.g., “prefer internal links over external when choosing between multiple anchors”) and document it here; do not special-case a domain.
- All improvements to selectors must come from inventory/observation layers (CDP AX tree, robust list detection) rather than hard-coded CSS.

## Agent Browser Guardrails
- For browser automation work in this repo, prefer the existing user-open Google Chrome session with the installed extension and native host already connected.
- Do **not** launch isolated browser instances, temporary Chrome profiles, Playwright-managed browsers, or any separate browser app/icon unless the human explicitly asks for that.
- Do **not** use Playwright for routine workflow execution, authenticated site debugging, or extension-orchestration tasks when the extension/native-host path is the real system under test.
- If the extension/native-host connection is missing, stop and ask the human to reload/reconnect the existing browser session instead of silently switching to another browser automation stack.
- Use Playwright only for repo-owned Playwright tests/e2e work, or when the human explicitly asks for Playwright.

## Testing Guidelines
- Rust unit tests inline with `#[cfg(test)]`; integration tests under `crates/<crate>/tests/*.rs`.
- Extension tests use Vitest (`*.test.ts`) and Playwright for e2e.
- Aim to cover core planners, native-host/browser-bridge transport, and DOM routing. Add minimal, focused tests near changed code.

## Commit & Pull Requests
- Commits: concise, imperative subject. Prefer Conventional Commits where reasonable (`feat:`, `fix:`, `chore:`). Group related changes.
- Before PR: run `make build`, `cargo test`, and `cd extension && bun x vitest`.
- PRs should include: clear description, rationale, and test plan; linked issues; relevant logs or screenshots (e.g., extension loaded from `extension/dist-chrome/`); any config notes (`.env` keys touched).

## Security & Configuration
- Never commit secrets. Use `.env` (copy from `.env.example`). Supported providers: `OPENAI_*` and `GEMINI_*`; select via `LLM_PROVIDER`.
- Minimal permissions approach: keep extension manifests and native host manifest changes scoped and documented in PRs.

## Feature Scratchpads (Single Source of Truth)
- Every feature MUST have a scratchpad document under `docs/features/<feature>/README.md`.
- Use it as the only onboarding doc for that feature; keep it current.
- Prefer a single document; if the feature grows large, create a folder with focused docs.

Scratchpad structure (required sections):
- Overview: single-paragraph goal and constraints.
- Flow Diagrams: end-to-end message flow and key internal flows (FSM, planner loop, DOM routing). ASCII diagrams are fine.
- Decision Record: tradeoffs and rationale for chosen approach vs. alternatives.
- Architecture: modules, responsibilities, and data contracts (schemas, message shapes).
- Implementation Notes: key function/tool calls, event flow, error handling, and retries.
- Tasks & Status: checklist of built/validated items and open work.
- What Works (Do Not Change): stable behaviors/APIs that must remain intact.
- Tried & Didn’t Work: approaches attempted and why they were rejected.

Templates and examples:
- Template: `docs/features/_template/README.md`
- Example: `docs/features/llm_autonomous/README.md`

## Phase 2 E2E (Enhanced Actions)
- Bridge: content script exposes `window.__rznExecuteStep(step)` and `window.captureEnhancedDOMSnapshot(opts)` via a postMessage bridge, available on any http(s) page.
- Tests: Playwright spins up a local HTTP server and runs actions end‑to‑end (click, fill, press_key, wait, scroll, extract) under `extension/tests/e2e/`.
- Run locally:
  - `cd extension && bun run build`
  - `bun x playwright install chromium`
  - `bun x playwright test --project=chromium-extension`
  - Artifacts: dist extension at `extension/dist-chrome/` and test output in `extension/test-results/`.

## CI
- GitHub Actions runs extension e2e on PRs/commits: `.github/workflows/extension-e2e.yml`
- Local Makefile shortcuts:
  - `make test-ext-e2e` → build + install browsers + run Playwright
  - `make phase2` → alias for Phase 2 validation

## Phase 3 (LLM Autonomous)
- Dummy provider: set `LLM_PROVIDER=dummy` to run autonomous planning without real API keys.
- Command-line:
  - `make phase3` (builds the runtime and runs an example task via dummy LLM)
  - Or manually: `LLM_PROVIDER=dummy ./target/release/rzn-browser llm-auto "Search Google for OpenAI" --max-steps 10`
- Notes:
  - Dummy LLM emits a fixed plan: navigate → type query → press Enter → extract.
  - Use real providers later by exporting keys (`OPENAI_API_KEY` or `GEMINI_API_KEY`) and `LLM_PROVIDER` accordingly.

## Unified Logging
- All components write to a single file: `~/rzn_build.log`.
  - Extension → sends JSON logs through the native host into the unified log
  - CLI → writes JSON lines with component=`cli`
  - Native host / worker → write runtime diagnostics into the same log stream
- Handy scripts:
  - `./scripts/logger.sh follow` — live tail (raw)
  - `./scripts/logger.sh follow-json` — live tail (pretty, requires `jq`)
  - `./scripts/logger.sh show 300` — last 300 lines
  - `./scripts/logger.sh clear` — rotate/clear unified log
  - `make logs-follow` / `make logs-show` / `make logs-clear` — aliases

## Scoped Mode (Single Path)
- Map/context: `make scope` (writes docs/index/*)
- Quick lookups: `make scope-q Q="…"`
- Guardrails: `STRICT=1 make sg-guards`
- Agent flow: `make agent-run M="…" [S=1]` → edit → `make agent-validate OUT=docs/index/agent_runs/<timestamp> STRICT=1`

## Plugin Release Requirement

If the task includes building or publishing the `rzn-browser` plugin bundle, release completion
also requires backend notification using the contract documented at:

- `/Users/sarav/Downloads/side/rzn/backend/docs/runbook/plugin_team_release_guide.md`

For plugin release work:

- Building a ZIP alone is not enough.
- Notify the backend through the release registration and catalog publish API flow.
- Publish to local `http://localhost:8082` first, then cloud `https://cloud.rzn.ai`, unless the human explicitly says otherwise.
- The repo’s release script also supports `prod` as a legacy alias for the cloud target.
- If local or cloud publish fails at any stage, stop and report exactly what failed.

## Landing the Plane (Session Completion)

Only follow this section when the human explicitly requests a commit/push/release pass. Do not auto-commit/push as part of normal iteration.

**MANDATORY WORKFLOW:**

1. **Run quality gates** (if code changed) - Tests, linters, builds
2. **PUSH TO REMOTE** - Only when requested:
   ```bash
   git pull --rebase
   git push
   git status  # MUST show "up to date with origin"
   ```
3. **Clean up** - Clear stashes, prune remote branches
4. **Verify** - All changes committed AND pushed
5. **Hand off** - Provide context for next session

**Notes:**
- If the human asks for a push and it fails, resolve and retry until it succeeds.
- Otherwise, keep work local and uncommitted until asked.

<!-- tusker:epic-index:begin -->
## Tusker V6 knowledge graph

Tusker is the sole repo-local system for planning, task tracking, evidence, knowledge-impact checks, verification, and closeout.

This project uses the V6 layout. Start with `tusker/SKILL.md`, route through the narrowest `tusker/domains/<domain>/INDEX.md`, then read that domain's `CANON.md` before opening task history. Durable truth lives in `tusker/domains/**`; task proof lives in `tusker/epics/**`.

When logging work: pick the epic and primary domain whose summaries best match, and announce both choices. If no existing domain or epic fits durable work, create the missing V6 domain/epic before adding tasks.
<!-- tusker:epic-index:end -->
