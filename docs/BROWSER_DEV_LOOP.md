# RZN Browser CLI Dev Loop

This repo can be used as a lightweight “workflow factory”: run the CLI with an LLM to iteratively
discover + validate repeatable browser steps, then save the resulting workflow JSON for reuse.

## 1) Install the Extension (dev)
1. Build the extension:
   - `cd extension && bun install && bun run build`
2. Load unpacked:
   - Open `chrome://extensions`
   - Enable **Developer mode**
   - **Load unpacked** from `extension/dist/chrome/`

**Default dev extension ID (pinned via manifest `key`):** `bogjdnehdficgkhklinmnbgiiofbamji`

## 2) Install Native Host Manifest
This repo uses the **same native host name** as the desktop app by default so the dev loop
matches production wiring:
- Native host name: `com.rzn.browser.broker`

Release-style install:
```
make install
```

That install populates:
- builtin workflows: `~/Library/Application Support/RZN/workflows/builtin`
- user workflows: `~/Library/Application Support/RZN/workflows/user`
- stable extension copy: `~/Library/Application Support/RZN/extension/dist/chrome`

Debug-first local setup:
```
make setup
```

Manual path (Chrome):
`~/Library/Application Support/Google/Chrome/NativeMessagingHosts/com.rzn.browser.broker.json`

## 3) Run the CLI (legacy pipe mode, no desktop app)
Recommended loop: use the **desktop-compatible wiring** (native host → worker bridge) so the
workflows you validate here behave the same way in the desktop app.

```
make run W=google/search PARAMS='--param search_query="rust lang"'
# or the explicit path form:
make run W=workflows/google/google-search.json PARAMS='--param search_query="rust lang"'
```

Defaults:
- Prefer `rzn-browser run ...` for the deterministic dev loop.
- Native messaging host name is `com.rzn.browser.broker`.
- Bare workflow references resolve from the installed workflow catalog before falling back to file paths.
- Standalone uses `rzn-browser ...`; the umbrella CLI contract is `rzn browser ...` with passthrough args.

If you see `zsh: killed` when executing scripts directly, use the `make` targets (they invoke `bash ...`).

## 4) Run the CLI through the supervisor

```
APP_BASE="$HOME/Library/Application Support/rzn_debug" \
./target/release/rzn-browser run workflows/tests/google-test-simple.json \
  --param search_query="rust"
```

## 4b) Run LLM Autonomous + Save Workflow JSON (recommended)
This mode runs an observe → plan → act loop until it completes (or hits limits), then saves a
deterministic replay workflow under the workflows cache directory.

To save workflows into this repo (instead of `~/.rzn/workflows`), set:
- `RZN_WORKFLOWS_DIR="$(pwd)/workflows/generated"`

Example:
```
RZN_WORKFLOWS_DIR="$(pwd)/workflows/generated" \
LLM_PROVIDER=dummy \
./target/release/rzn-browser llm-auto "Search Google for rust lang and extract the top results" \
  --max-steps 20 \
  --save-workflow
```

## 5) Re-check supervisor readiness

```
./target/release/rzn-browser supervisor ensure-ready
./target/release/rzn-browser heal --json
```

## Notes
- Discover installed workflows with `rzn-browser workflow list google`.
- Refresh shipped workflows/examples with `rzn-browser workflow pull`.
- Packaged examples live under the `examples/*` namespace, e.g. `rzn-browser workflow list examples`.
- Import your own workflow JSONs with `rzn-browser workflow add /path/to/workflow.json --system google --name my-flow`.
- The runner submits work to the local supervisor, which dispatches through the native-host bridge.
- Use `--snapshot after-step` or `--snapshot on-error` to collect DOM snapshots during runs.
