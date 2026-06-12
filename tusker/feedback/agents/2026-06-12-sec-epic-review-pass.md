# 2026-06-12 — SEC epic independent review pass (post-implementation)

Scope: 5-reviewer fan-out over the uncommitted SEC-T-0001..0014 implementation diff (~9.2k insertions, 119 files), plus first-hand verification of the two critical contracts and all medium+ reviewer claims. Three patches applied during review.

## Patches applied during review

1. `extension/src/background.ts` — persisted CDP lease writes (upsert / remove / sweep-save) now serialize through one mutation queue. Previously `persistCdpLease` was fire-and-forget load-modify-save racing the alarm sweep's load-split-save; concurrent writes could resurrect swept entries in `chrome.storage.session`. Memory/detach state was never wrong — storage-only inconsistency.
2. `install.sh` + `install.ps1` — added `RZN_INSTALL_VERSION` pin (validated tag charset, maps to `releases/download/<tag>`), and install.sh now prints the curl-resolved effective artifact URL so "latest" installs are auditable. This was SEC-T-0006 plan item 2, the only contract item the implementation skipped.

## Reviewer claims dispositioned as false positives (do not re-chase)

- "parse_mode accepts empty string → 0o644" — wrong; `trimmed.is_empty()` bails (devkit main.rs:679).
- "workflow JSON validation test missing" — exists at `workflow_catalog.rs:1960` (reviewer's file scope just excluded that crate).
- "supervisor_cloud backoff unbounded" — `next_backoff` caps at 30s (supervisor_cloud.rs:832).
- "sdk selector_by_encoded_id stale on refresh" — whole map is reassigned with `=` (session.rs:207).
- "broker read timeout desyncs stream" — every read-error/timeout path either `reconnect()`s (broker_client.rs:494) or sets `connection = None` (:2149); handshake timeouts drop the half-built client. No surviving stream after a cancelled read.
- "deadline_at_ms=0 insert window" — consumers are health JSON + disconnect responses only, no reaper compares to now; deadline-after-send is intentional (tests at supervisor.rs:9579 assert it).

## Accepted residuals (documented, not fixed)

- Bare `{param}` in a non-string JS position of a script field is still injectable via `substitute_script_string`'s fragment-escape path. All repo workflows are migrated to `window.__rzn_params`; residual only matters for a trusted-but-sloppy third-party manifest, and such a manifest already controls the script text outright.
- Bridge token is DOM-readable in test builds — inherent (pageBridge lives in MAIN world); prod withholds the token entirely, so the page channel is closed in production.
- Shared reqwest clients follow redirects; reqwest strips Authorization on cross-host redirects, so the flagged token-leak is theoretical.
- `RZN_INSTALL_REPO` env stays unvalidated-but-quoted (TOFU residual per contract).

## Verification after patches

`sh -n`/`bash -n` on install.sh, `scripts/release/test_install_verification.sh` pass, extension vitest 59/59, prod `build:chrome` clean with test hooks absent, Playwright e2e 34 passed / 4 skipped on the dev bundle. No Rust files touched by the patches; the workspace test/clippy runs from the implementation pass stand.

## Operational gotcha: dist/chrome bundle flavor

`bun run build:chrome` (prod) silently overwrites `extension/dist/chrome`, which is the exact path Playwright loads. A prod bundle there makes the whole e2e suite time out (~60s/test, 33 failures) because production correctly withholds the page-bridge token and test hooks — an accidental live negative test of the SEC-T-0001/0008 gating, and a 24-minute trap. Before e2e, rebuild with `RZN_PAGE_TEST_BRIDGE_ENABLED=1 ./build.sh chrome` (or `bun run build:dev`). Also: `cmd | tail` eats the runner's exit code — trust `extension/test-results/.last-run.json`, not pipe exit status.

## Process notes

- The 5-way reviewer fan-out worked, but two reviewers reported "contract gaps" that were really scope-boundary artifacts (the item lived in a file owned by another reviewer). When fanning out reviews, cross-check any "X is missing" claim against the other reviewers' scopes before acting.
- rtk-compressed grep output drops file prefixes on multi-file matches; for line-accurate work, follow up with Read on the specific region rather than trusting the compressed table.
