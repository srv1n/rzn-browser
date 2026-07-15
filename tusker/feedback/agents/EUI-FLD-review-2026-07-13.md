# EUI + FLD second-wave review (2026-07-13, 3 Opus reviewers, code-read only — no builds/tests run)

Scope: uncommitted rzn-browser tree (EUI Rust + extension TS) and committed backend fleet work (FLD). Every finding from EUI-review-2026-07-12.md was dispositioned.

## Prior-findings scoreboard

- Rust worklist 1–10: ALL fixed or mostly fixed. gc wired (startup+daily), origins local_cli/mcp/fleet:<job> correct, runs.start/replay real, pause gates fleet claiming (unit test), LogBuffer fed by registered tracing Layer, now_running live, supervisor_control tests exist INCLUDING the security zip test (device_token redacted + *.params.json excluded, asserted against zip bytes), native-host passthrough now allowlist-enforced supervisor-side with a rejection test. Two partials: failure capture (#3) and step-index (#4) — see blockers.
- Extension worklist 1–7: ALL fixed. tsc type error resolved by eye, alarms badge tick (0.5min), elapsed + cloud poll age in popup, dot no longer bound to extension_connected, real behavioral tests, zod added for the named methods, tabs split into per-tab modules with tests.
- backend results.rs test helper: fixed (failure_summary: None present).
- Security posture: CLEAN across all three scopes. Tokens hashed at rest, no token/params in logs or diagnostics (tested), SPA never renders secrets, zero AI attribution anywhere (incl. backend git history).

## Ship-blockers (HIGH)

1. **failing_step_index is structurally always None** — every failure routes through run_result_shell which sets steps: Vec::new() (workflow_runner/mod.rs:205); native_runner.rs:112 / supervisor.rs:1428 compute position() over that empty vec; supervisor_fleet.rs:1074 passes None outright. The runner KNOWS the failing idx (workflow_runner/mod.rs:239,296,336) but only threads it into report_context. Net: all fingerprints degenerate to "{hash}::{class}"; the epic's headline output ("failed 4× at step 6") cannot be produced; different-step same-class failures merge (over-triggers broken; also blunts backend dominant-fingerprint classification). Fix: propagate the failing idx from WorkflowRunFailure into failure_summary at the three completion sites.
2. **Fleet notifications never render in a real browser** — extension/src/ui/notifications.ts:3 uses iconUrl 'icons/icon128.png'; shipped icons are icons/brain-{16,32,48,128}.png. chrome.notifications.create fails with lastError and shows nothing. Tests green because none assert iconUrl. One-line fix ('icons/brain-128.png') + add icon assertion to notifications.test.ts.
3. **supervisor workflows.list is a façade** — supervisor.rs:1997-2005 re-projects run-health rows (name=workflow_id, source hard-coded "local"); no catalog+server-cache merge, so never-run and server-cached workflows can never appear. EUI-T-0006 A1 unmet (task honestly self-reports the row as fail and sits at ready).
4. **fleet-admin SPA health page reads a response shape the backend doesn't emit** — web/fleet-admin/src/api.ts:3 + App.tsx:37 expect top-level failing_device_count/total_device_count and classification 'broken'; backend emits nested failing_devices{devices_failing,devices_total} and 'workflow_broken' (health.rs:15-49). Broken workflows render a GREEN chip; device-problem line renders undefined/undefined; the device-vs-workflow branch keys on dominant_fingerprint presence (always set on failing rollups) instead of classification. FLD-T-0003 A4 undeliverable until aligned.

## MEDIUM

- Screenshot/DOM failure capture still dead: take_snapshot discards everything but dom_hash (workflow_runner/mod.rs:405-426) so run_store.rs:236 can never populate screenshot_b64/dom_excerpt → EUI-T-0002 A5 is overstated as "pass" (only console_tail is real).
- Run-store index append not concurrency-safe across RunStore instances (fleet loop, MCP, CLI each open their own over one index.jsonl; serde_json::to_writer multi-syscall append can interleave). Fix: single write_all of a serialized buffer under O_APPEND.
- status.snapshot cache never refreshed on fleet/MCP completions (supervisor.rs:1446,4705,4717 only) — popup can miss fleet runs for up to 24h, the primary monitoring case.
- settings.json written with fs::write (non-atomic) + silent Default fallback on parse error → crash mid-write silently resets retention/notification config.
- ui/rpc.ts:84-86 coerces every failure (incl. app errors like "automation is paused") to SupervisorUnreachable; ZodError dumped raw into the unavailable() pre. runs.cancel / automation.pause / automation.resume have no zod schemas at all.
- Backend rollup window filters/sorts on fleet_job_results.created_at which is unindexed; the new idx_fleet_jobs_health indexes fleet_jobs.created_at instead — comment overstates.
- Single-device tenants: required_devices=1 means one failing device trips workflow_broken even alongside successes — contract-literal but removes the distinction for one-laptop fleets; confirm intended.

## LOW (worth batching, not blocking)

- No golden test pins fingerprint bytes device-side (backend has one: "hash::unknown" → 3f409ed5813ea490); add matching golden in workflow_health.rs.
- MCP failures never get failure_summary (mcp_browser.rs:99-119) → invisible to health.
- session_close outcome only ever succeeded/failed (cancelled/timed_out unreachable) — cosmetic.
- TOCTOU on the single-run guard (two concurrent runs.start can both pass).
- Fleet loop hard-codes default_app_base_dir() for its run store, ignoring SupervisorConfig.app_base.
- Popup dot hard-coded "ok"; badge running count caps at 1; recent_runs[0] assumes newest-first.
- rpc.test.ts only asserts happy path (never rejects malformed) — exactly why the icon bug survived green tests; run.ended_at required in schema (future in-flight rows would throw as false unreachable).
- fleet.enroll response merged into rendered state as z.record(z.unknown()) — no field allow-listing.
- Backend: artifact "streaming" fully buffers; /fleet-admin ServeDir lacks the .exists() guard the /admin mount has; rollup loads full 7-day set into memory before the 50-cap.

## Cross-repo canon verdict

Device fingerprint fn (workflow_health.rs:60-67) matches the pinned canon byte-for-byte; backend never re-derives device fingerprints (pass-through verbatim) and its legacy-fallback "{hash}::unknown" form is byte-compatible with a pinned test vector. Canon is INTACT — but blocker #1 means in practice every device fingerprint is currently the empty-step form.

## Vault vs reality

- Honest: EUI-T-0006 (ready, A1 row self-reports fail), EUI-T-0008 (ready), FLD-T-0003 (ready/partial), EUI-T-0009 manual row pending.
- Overstated: EUI-T-0002 marks A2/A5 pass but step-index and screenshot/DOM capture are undeliverable as written → should go back to rework, not close.
- EUI-T-0002 verification row still uses `-p rzn_contracts` naming inconsistency.

## Suggested close-out order

1. One-liners first: notification iconUrl (+ icon test), SPA type alignment to health.rs shapes + classification strings.
2. Step-index propagation (blocker #1) — small, three completion sites + shell; add golden fingerprint test while there.
3. workflows.list catalog+cache merge (real work, contract-owned by EUI-T-0006).
4. Batch the mediums (capture payload, snapshot refresh on fleet/MCP, atomic settings write, rpc error taxonomy, backend index).
5. Then run the deferred gates + manual proofs and let tasks close.
