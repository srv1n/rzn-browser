# Full-repo review findings (2026-06-11)

5-agent parallel audit: native host, Rust crates, extension background, content scripts, workflows/ops.

Top actionable signal:

1. **CRIT — unauthenticated page→extension bridge.** `contentScript.ts:6382` window message listener and `:6485` DOM bridge accept `RZN_TEST_EXECUTE` from any page/iframe with no origin/source/token check; results posted with targetOrigin `'*'`. Any visited page can run ~60 action handlers (eval_main_world, get_cookies, get_local_storage_item, same_origin_request, take_screenshot, download_images) at content-script privilege, incl. cross-origin iframe → parent escalation. Fix: per-load secret token minted by background + step-type allowlist for page-reachable handlers.
2. **CRIT — param injection into workflow JS.** `native_runner.rs:1033` `substitute_string` does raw `replace("{key}", val)`; 13 workflows interpolate `'{param}'` inside script strings. Apostrophe breaks the step; crafted input = arbitrary MAIN-world JS. Fix: JSON-escape at substitution + migrate all workflows to `args`/`window.__rzn_params`.
3. **CRIT — chat_json fallback returns raw API envelope as "parsed plan"** (`rzn_plan/src/llm.rs:1141`). Delete stage-3 brace-scan.
4. **HIGH — install pipeline unverified**: `curl|sh` → no checksum (`.sha256` sidecars generated but never checked) → `codesign --sign -` → quarantine strip.
5. **HIGH — secrets perms**: actor_token config, supervisor token, devkit ed25519 private key all written 0644; `/tmp/llm_raw_*.jsonl` logs full prompts world-readable.
6. **HIGH — broker framing**: timeout cancels `read_exact` mid-frame, no reconnect → permanent desync (`broker_client.rs:349`); 10MB client cap vs 16MB supervisor cap mismatch.
7. Unused `security_prompts.rs` — autonomous agent ingests raw DOM with zero injection hardening (`llm_autonomous.rs:1348`).
8. Hygiene: 833KB source zip tracked at root; dist-chrome/dist-firefox tracked despite intent to ignore; clippy/cargo-audit advisory-only in CI; no workflow-JSON validation gate.

Structural: background.ts 7.7k lines / contentScript.ts 6.8k lines / rzn_plan 33k god crate; frame protocol implemented twice (broker_client vs supervisor) — move to rzn_core.
