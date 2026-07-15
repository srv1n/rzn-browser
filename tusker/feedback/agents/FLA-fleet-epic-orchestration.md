# FLA epic orchestration feedback (2026-07-11)

- FLA-T-0001..0005 implemented and at `review` (proof satisfied); FLA-T-0006 (cross-repo smoke) remains `ready` for external pickup. Deferred rows: release build + manual CLI regression on FLA-T-0002/0005.
- Cross-repo trap worth canon: the manifest content hash is sha256 over COMPACT serde_json bytes with object keys RECURSIVELY SORTED — explicit canonicalization is load-bearing because the backend workspace builds serde_json with `preserve_order` while this repo does not. Both `workflow_cache::manifest_content_hash` (here) and `canonical_manifest_hash` (backend rzn-fleet) implement it; never change one side alone.
- Supervisor fleet loop test/e2e knobs: `RZN_FLEET_POLL_INTERVAL_MS`, `RZN_FLEET_DISABLE_JITTER=1`, `RZN_FLEET_CONFIG_PATH`; journal/results/cache root at `default_app_base_dir()`. Fleet-dispatched manifests must carry inline `steps[]` (no workflows/ root on the cache path).
- Runner now lives in `workflow_runner/` (execute_workflow + StepTransport + RunEventSink); `native_runner.rs` is CLI glue only. `StepTransport::call(timeout_ms=0)` means "no client watchdog".
- Orchestration pattern that worked: pre-stub shared files (module stubs + mod lines) before fanning out agents so no two agents edit the same file concurrently — zero merge conflicts across 8 parallel implementation agents in this repo.
