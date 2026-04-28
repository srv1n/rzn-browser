# Tools Team Spec: Workflow Failure Reporting

## Goal
Implement the browser/tooling side of explicit workflow failure reporting. When a workflow fails, the CLI should print a readable report command that contains the whole default report. Running that command submits only those visible fields to the report API.

This is not telemetry. This is a user-initiated "this workflow broke" report.

## Command Surface
Add:

```bash
rzn-browser report workflow-broken \
  --system <system> \
  --workflow <system/workflow-id> \
  --version <workflow-version-or-hash> \
  --step <failed-step-id-or-index> \
  --error <stable-error-code> \
  --app-version <semver> \
  --platform <platform> \
  [--note <user-authored-note>] \
  [--dry-run]
```

Do not add these to the recommended output:

| Avoid | Why |
| --- | --- |
| `report last-failure` | Hidden local state, bad trust model |
| `--run-id` | Developer detail, implies stored trace lookup |
| opaque token | Looks like a secret upload handle |
| `--diagnostics` | Loaded word; users assume broad collection |
| `--include-inputs` | Private by default; not v1 |
| `--include-logs` | Too risky and noisy |

## Failure Output Contract
When a workflow failure is attributable to a workflow and step, render:

```text
Workflow failed: <workflow>
Failed at: <step>
Reason: <error>

Reporting this helps us know what broke, group similar failures, and fix the workflow faster.

Report this broken workflow:
  rzn-browser report workflow-broken \
    --system <system> \
    --workflow <workflow> \
    --version <version> \
    --step <step> \
    --error <error> \
    --app-version <app_version> \
    --platform <platform>

This command sends exactly the fields shown above.
It does not read your browser, page content, search terms, form values, cookies,
screenshots, logs, workflow inputs, or browser history.

Optional, if you want to add context in your own words:
  rzn-browser report workflow-broken \
    --system <system> \
    --workflow <workflow> \
    --version <version> \
    --step <step> \
    --error <error> \
    --app-version <app_version> \
    --platform <platform> \
    --note "what happened?"
```

Do not print the block for parser errors, missing local binary setup, invalid CLI flags, or network failures unrelated to workflow execution.

## Safe Field Derivation
The workflow runner can know private things, but the renderer must reduce the failure to safe fields.

| Field | Source | Rule |
| --- | --- | --- |
| `system` | resolved workflow catalog metadata or path namespace | Lowercase slug; no user input values |
| `workflow` | canonical id | Prefer `system/workflow-id` |
| `version` | workflow metadata, release version, or content hash | Do not expose local file paths |
| `step` | step `id` or generated stable index | Prefer workflow-authored id; else `step_<n>_<action>` |
| `error` | normalized category | Use stable code, not raw exception message |
| `app-version` | binary crate version | Explicit arg if sent |
| `platform` | OS family | `macos`, `windows`, `linux`; no usernames or hostnames |
| `note` | user-written CLI argument | Optional, trimmed, length-limited |

Error normalization:

| Raw class | Suggested code |
| --- | --- |
| target/selector missing | `element_not_found` |
| visible button target missing | `button_not_found` |
| found but not actionable | `element_not_clickable` |
| input target missing | `input_not_found` |
| step timeout | `timeout` |
| navigation error | `navigation_failed` |
| extension not connected | `extension_disconnected` |
| native host not connected | `native_host_disconnected` |
| login wall | `auth_required` |
| CAPTCHA or anti-bot interstitial | `blocked_by_captcha` |
| fallback | `unknown_failure` |

## Report Command Behavior
Submission:
- Build JSON only from parsed CLI flags.
- Do not inspect local run artifacts.
- Do not read unified logs.
- Do not query the extension.
- Do not read workflow input params.
- Do not include URLs, DOM, screenshots, LLM prompts/responses, or browser state.

`--dry-run`:
- Print exactly the JSON body that would be sent.
- Do not make a network call.
- Keep key order stable for tests and user readability.

Successful submission output:

```text
Report counted: wfg_01J...
This helps us see which workflows are broken and fix them faster.
```

New group output:

```text
Report sent: wfg_01J...
This helps us see which workflows are broken and fix them faster.
```

Network failure output:

```text
Could not send report: <short reason>
You can retry the same command later.
```

## API Contract
Production default:

```text
https://cloud.rzn.ai/v1/workflow-failure-reports
```

Local/staging override:

```text
RZN_WORKFLOW_REPORT_URL=http://localhost:8787/v1/workflow-failure-reports
```

Request:

```json
{
  "schema_version": 1,
  "source": "rzn-browser-cli",
  "mode": "explicit_minimal",
  "system": "google",
  "workflow": "google/search-v1",
  "workflow_version": "2026-04-24.1",
  "failed_step": "search_button",
  "error": "button_not_found",
  "app_version": "0.8.3",
  "platform": "macos",
  "note": "Optional user-written note"
}
```

Response:

```json
{
  "ok": true,
  "report_id": "wfr_01J...",
  "group_id": "wfg_01J...",
  "status": "created"
}
```

`status` values:

| Status | Meaning |
| --- | --- |
| `created` | New failure group created |
| `counted` | Existing failure group count incremented |
| `rate_limited_counted` | Duplicate/abuse-safe counted response |

## Backend MVP For Tools Infra
Use a Cloudflare Worker with D1.

Validation:
- Require all fields except `note`.
- Reject unknown fields.
- Bound every string length.
- Enforce slug-like `system`, `workflow`, `step`, and `error`.
- Allow app versions and workflow versions to include dots, dashes, plus signs, and `sha256:` prefixes.
- Trim notes and cap at 1000 characters for v1.

Dedupe fingerprint:

```text
sha256(schema_version + system + workflow + workflow_version + failed_step + error)
```

Rate limits:
- Per IP bucket: soft cap reports/minute.
- Per fingerprint/IP bucket: stronger duplicate cap.
- Notes: cap stored notes per fingerprint per hour to avoid note spam.

Maintainer query should answer:
- Which workflows are most broken right now?
- Which versions are affected?
- Which step/error pair is failing?
- Did counts drop after a workflow release?
- Which failures include user-written notes?

## Tests
CLI unit tests:
- Parses full `report workflow-broken` command.
- Rejects missing required fields.
- Rejects unknown fields in request body construction.
- `--dry-run` prints no private fields.
- Generated report command from a simulated failure contains only explicit safe fields.
- Raw error messages with URLs/search terms are normalized and not printed as `--error`.

Integration tests:
- Simulate a workflow failure and snapshot terminal output.
- Run the printed command against a local mock server.
- Assert payload equals the visible command fields.
- Assert no trace/log/read APIs are called during submission.

Backend tests:
- Same fingerprint increments group count.
- Different workflow version creates a separate group.
- Notes are stored separately and count increments.
- Oversized note is rejected or truncated according to final policy.
- Unknown JSON fields are rejected.

## Rollout
1. Add backend endpoint behind staging URL.
2. Add CLI command with `--dry-run` and mock-server tests.
3. Render failure block only when report URL is configured in dev.
4. Ship production endpoint default.
5. Add maintainer view/query.
6. Tune error normalization after real workflow failures.

## Acceptance Criteria
- A user can read the recommended command and know every field being sent.
- Default report submission cannot access hidden local failure state.
- No private workflow params, URLs, DOM, screenshots, cookies, logs, prompts, responses, or browser history are sent.
- Duplicate reports are grouped by fingerprint and increment a counter.
- Optional notes are clearly user-authored and stored separately.
- Failure output includes: "Reporting this helps us know what broke, group similar failures, and fix the workflow faster."
