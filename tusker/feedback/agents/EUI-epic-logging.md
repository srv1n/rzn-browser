# EUI epic logging feedback (2026-07-11)

- EUI (this repo, 9 tasks) + FLD (backend, 3 tasks) logged and `ready` — tasks only, no implementation, per operator instruction. Suggested order: EUI 0001→0002→0003 supervisor-side, then 0004 shell unlocks 0005–0009 in parallel; FLD 0001+0002 parallel, then 0003.
- New cross-repo canon introduced (mirrors the manifest-hash lesson): `failure_summary {error_class, failing_step_index, fingerprint, message}` optional on RunResultV2; error_class enum fixed at 8 values; device side (EUI-T-0002) produces, backend health rollup (FLD-T-0001) consumes with `unknown` fallback. Never change one side alone.
- Architecture decision binding all EUI UI tasks: extension owns zero state — every panel is a supervisor RPC via the existing background.ts native-messaging path; no second port, no framework dependency without owner sign-off.
- Trap encoded in EUI-T-0009: do NOT run fleet jobs in a minimized window (Chrome occlusion-throttles timers/rendering) — unfocused normal window only.
- Vault mechanics reminder: direct body edits to task files trigger CAS_CONFLICT on the next control op — run `tusker reconcile` first, then `tusker status ... ready` works.
