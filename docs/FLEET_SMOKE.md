# Fleet smoke test

`scripts/fleet_smoke.sh` is the cross-repository gate for the laptop fleet
agent. It builds and starts the real backend, enrolls one isolated device,
publishes an inline-step workflow, submits a job, and proves the backend can
read the terminal result and recent device heartbeat. It also checks
`rzn-browser fleet status --json` for the laptop-side `posted` journal entry.

## Prerequisites

- The sibling backend checkout at `../backend`, or `BACKEND_DIR=/path/to/backend`.
- Rust/Cargo, `curl`, `jq`, and Python 3.
- Free local disk for both Rust builds.
- Backend FLT-T-0001 through FLT-T-0008 at `review` or `done`.

The script creates one temporary root containing the backend SQLite database,
fleet config, supervisor socket/token, result spool, journal, and workflow
cache. `RZN_FLEET_CONFIG_PATH`, `RZN_SUPERVISOR_APP_BASE`, `RZN_APP_BASE_DIR`,
and `RZN_RUNTIME_DIR` all point into it. A trap kills both child processes and
removes the root on success, failure, or interruption. It never reads or writes
the developer's normal fleet identity or supervisor state.

Operator calls use a tenant API key inserted into the disposable database. The
request still traverses the backend's normal API-key authentication middleware;
the smoke adds no backend bypass or production credential.

## Tier 1: CI-safe transport loop

Run:

```bash
bash scripts/fleet_smoke.sh
```

Tier 1 starts no browser. Its one-step `wait_for_timeout` manifest has no browser
side effects, but the shared workflow runner opens a browser session before the
first step. Because the isolated supervisor intentionally has no native-host
bridge, the expected job status is `failed`. That terminal failure is useful:
it proves enrollment, heartbeat, polling, claim, manifest fetch/hash, execution
attempt, durable result/journal handling, result posting, and operator readback.

The poll cadence is reduced to 200 ms with `RZN_FLEET_POLL_INTERVAL_MS=200` and
jitter is disabled with `RZN_FLEET_DISABLE_JITTER=1`; production defaults are
unchanged.

Expected final output resembles:

```text
[pass] job=<uuid> status=failed device=<uuid> heartbeat=recent journal=posted
```

## Tier 2: real Google Search

Run:

```bash
bash scripts/fleet_smoke.sh --with-browser
```

Tier 2 publishes `workflows/google/google-search.json`, submits the query
`rzn fleet laptop smoke`, requires `succeeded`, and rejects an empty output.
Chrome, the extension, and the native host must connect to the script's isolated
supervisor—not the normal developer supervisor.

For a manual operator run, choose the temporary root before launching a separate
Chrome profile so the Chrome-owned native host inherits the matching app base.
For example on macOS, from the repository root:

```bash
SMOKE_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/rzn-fleet-browser.XXXXXX")
CHROME_PROFILE=$(mktemp -d "${TMPDIR:-/tmp}/rzn-fleet-chrome.XXXXXX")
RZN_SUPERVISOR_APP_BASE="$SMOKE_ROOT/app" RZN_APP_BASE_DIR="$SMOKE_ROOT/app" \
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  --user-data-dir="$CHROME_PROFILE" \
  --disable-extensions-except="$PWD/extension/dist-chrome" \
  --load-extension="$PWD/extension/dist-chrome" &
CHROME_PID=$!
RZN_FLEET_SMOKE_ROOT="$SMOKE_ROOT" bash scripts/fleet_smoke.sh --with-browser
kill "$CHROME_PID" 2>/dev/null || true
rm -rf "$CHROME_PROFILE"
```

Adjust the executable and unpacked extension paths for another platform. The
native-host e2e precedent is
`extension/tests/e2e/native_host_smoke.spec.ts`. The smoke owns and removes
`SMOKE_ROOT`, so do not put anything else in it.

Expected final output resembles:

```text
[pass] job=<uuid> status=succeeded device=<uuid> heartbeat=recent journal=posted
```

On failure the script prints short backend and supervisor log tails before
cleanup. Set `PORT=<unused-port>` only when a fixed port is needed.
