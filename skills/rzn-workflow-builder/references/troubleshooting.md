# Troubleshooting

Use this reference when the runtime is not ready or workflow validation is failing.

## Runtime Not Installed

Run:

```bash
./skills/rzn-workflow-builder/scripts/ensure-runtime.sh
```

That script:

- probes `rzn-browser list google`
- runs `make install` if the probe fails
- runs `make doctor`

## Native Host Or Extension Not Connected

Typical symptoms:

- `No native host connected`
- `Timed out waiting for native host connection`
- `Timed out waiting for native host/extension connection`

Fixes:

1. Open `chrome://extensions`
2. Enable Developer mode
3. Load unpacked from the stable extension copy:
   - macOS: `~/Library/Application Support/RZN/extension/dist-chrome`
   - Linux: `~/.local/share/RZN/extension/dist-chrome`
4. Reload the extension if it is already installed
5. Keep a Chrome window open
6. Rerun `make doctor`

## Avoid Real Provider Billing During Discovery

Use dummy mode for workflow discovery:

```bash
LLM_PROVIDER=dummy ./skills/rzn-workflow-builder/scripts/discover-workflow.sh "Search Google for rust lang and extract the top results"
```

## Dedicated Tab vs Existing Session Bugs

If a site behaves badly in a new tab but works in the live signed-in tab:

- set `runtime.requires_existing_session: true`
- keep the flow review-style

If the workflow steals the operator's current tab and should not:

- remove legacy active-tab fields
- run in a dedicated tab instead

## Logs

Useful places to inspect:

- unified log: `~/rzn_build.log`
- wiring check: `make doctor`
- workflow docs: `docs/BROWSER_DEV_LOOP.md`

## Last Resort

If the runtime still smells broken after install + doctor:

1. rerun `make install`
2. reload the extension
3. rerun `make doctor`
4. retry with a tiny known-good flow:

```bash
rzn-browser run google search --param search_query="browser automation"
```
