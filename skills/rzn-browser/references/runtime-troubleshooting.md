# Runtime Troubleshooting

Use this when the CLI cannot run workflows or `llm-auto` cannot connect.

## Fast Checks

```bash
rzn-browser list google
rzn-browser list google search
rzn-browser workflow dirs
```

If those fail, the CLI or workflow catalog is not installed correctly.

## Chrome Extension And Native Host

Symptoms:

- `No native host connected`
- `Timed out waiting for native host connection`
- `Timed out waiting for native host/extension connection`
- workflow opens no tab and then times out

Manual fix to tell the user:

1. Open `chrome://extensions`.
2. Enable Developer mode.
3. Reload the RZN extension.
4. If missing, load unpacked from the stable extension path:
   - macOS: `~/Library/Application Support/RZN/extension/dist-chrome`
   - Linux: `~/.local/share/RZN/extension/dist-chrome`
   - Windows: `%LOCALAPPDATA%\RZN\extension\dist-chrome`
5. Keep a normal Chrome window open.
6. Rerun the smallest known-good workflow:

```bash
rzn-browser run google search --param search_query="browser automation"
```

Do not switch to Playwright or a separate Chrome profile as a workaround. That tests the wrong system.

## Source Checkout Checks

When working inside the repo:

```bash
make doctor
make install
make build
```

Use `make install` only when the task is to repair or refresh local RZN installation. It mutates the local runtime.

## Provider Checks For llm-auto

Real autonomous mode needs a configured provider:

```bash
env | grep -E '^(LLM_PROVIDER|OPENAI_|GEMINI_)'
```

Use dummy mode only for deterministic local smoke tests:

```bash
LLM_PROVIDER=dummy rzn-browser llm-auto "Search Google for OpenAI" --max-steps 10
```

If provider variables are missing and the user asked for a real open-ended task, ask them to configure `OPENAI_*` or `GEMINI_*`.

## Dedicated Tab vs Existing Session

- Prefer dedicated workflow tabs for repeatable extraction and parallel runs.
- Use `runtime.requires_existing_session: true` only when the workflow explicitly requires the already-open signed-in page or review state.
- If a workflow unexpectedly steals the current tab, inspect the workflow for legacy active-tab fields and remove them from production JSON.
- If an authenticated flow fails in a fresh tab but works in the live tab, existing-session behavior may be correct, but call it out.

## Bad Output

If a command succeeds but extracts junk:

1. Run the detailed help for the workflow and check params:

```bash
rzn-browser list <system> <workflow>
```

2. Retry once with clearer params.
3. If still wrong, use `llm-auto` to probe the page.
4. If the flow should be repeatable, save the discovered flow and turn it into a workflow.
