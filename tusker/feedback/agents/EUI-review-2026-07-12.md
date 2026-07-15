# EUI implementation review (2026-07-12, 3 Opus reviewers, uncommitted tree)

## Verdicts
- EUI-T-0001 run store: partial — core correct, but gc() never wired (no startup/daily call → unbounded runs/ growth), CLI writes origin "cli" not "local_cli" (breaks runs.list filter), MCP origin not wired.
- EUI-T-0002 health: partial — classify/fingerprint/compute_health/FailureSummaryV1 green; real failure capture unimplemented (always capture_unavailable); CLI failure_summary drops failing_step_index + workflow_hash → fingerprints collapse per error_class.
- EUI-T-0003 control RPCs: largely façade — runs.start/runs.replay only exist as paused-guard then fall through to Unknown method; pause does NOT gate fleet claiming; LogBuffer never fed (no tracing Layer → logs.tail/diagnostics logs always empty); now_running hardcoded null; ZERO supervisor_control tests incl. the security zip test (code path verified safe by reading — token redacted, params excluded — but unproven).
- EUI-T-0004 popup/shell: partial — rpc client/popup/scaffold/badge-fn real and building; missing chrome.alarms badge tick (badge stale when popup closed), popup missing elapsed + cloud last-poll age, supervisor dot bound to wrong signal (extension_connected), no popup/badge/dashboard tests.
- EUI-T-0005..0008 tabs: implemented ahead of contract inline in dashboard/index.ts — functional but untested, partial zod coverage (schema map has workflows.health but code calls workflows.list; fleet.status/settings.get/diagnostics.export/runs.get unvalidated).
- EUI-T-0009: notification half only; window isolation entirely absent; no supervisor fleet_run_notice emission.
- FLA-T-0006 smoke: complete per contract (tier-1 sanctioned-failure path real end-to-end incl. API-key seeding byte-identical to backend; tier-2 manual runbook). MEDIUM: cleanup trap `rm -rf $ROOT` on user-supplied RZN_FLEET_SMOKE_ROOT — refuse pre-existing non-empty dirs.

## Gates
- rzn-browser: cargo test rzn-browser 278 pass; rzn_contracts 35 pass; native host check clean; extension vitest 3 pass (undercovers); tsc FAIL (ui/notifications.ts:3 void-return TS2322); build.sh PASS all 4 browsers + dashboard.html.
- backend: cargo check --features browser-fleet PASS (cross-repo contracts gate GREEN); cargo test -p rzn-fleet FAIL — results.rs:619 test helper missing `failure_summary: None` (one-line fix).

## Systemic
- Task Verification rows say `-p rzn_browser` / `-p rzn_native_host`; real names are hyphenated (rzn-browser, rzn-native-host). Fix rows in EUI-T-0001/2/3/6/8/9 + future contracts.
- Tusker bypassed entirely: all EUI + FLA-T-0006 still `ready`, zero verify rows recorded. Vault ≠ reality.
- Native host: 3-method allowlist replaced by generic supervisor_rpc passthrough — safe as gated (origin-restricted manifest + requirePopupSender + supervisor token auth) but supervisor should now enforce a method allowlist.
- Build-infra scope creep: AGENTS.md Build Policy + Makefile hard-$(error) without sccache + new .cargo/config.toml rustc-wrapper — breaks any host without sccache; decide policy explicitly, don't let it ride in with UI work.
- Junk: .tusker/scratch/ADI-* proof files in wrong vault path (repo vault is tusker/, dot-less) — do not commit.
- Cross-repo canon gap: fingerprint construction ("{hash}:{step}:{class}", empty for None, hex[..16]) not pinned in canon; FLD-T-0001 must match byte-for-byte — freeze it in both epics before backend implements.
