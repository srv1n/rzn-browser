# Repo Map (Start Here)

Goal: stealth-first browser automation with a Rust workspace and an MV3 extension communicating through the native host plus the local browser supervisor.

Key paths:
- crates/: Rust workspace crates (rzn_browser, rzn_core, rzn_plan, rzn_sdk, rzn_native_host)
- extension/: MV3 extension (TypeScript, Vite, Vitest, Playwright); build → dist/{chrome,edge,chromium}/
- workflows/: JSON workflows runnable via CLI/scripts
- test/ and tests/: HTML/manual test pages and Rust integration/unit tests
- .env.example → .env: LLM provider and runtime options

Entry points:
- CLI: `./target/release/rzn-browser`
- Native host: `./target/release/rzn-native-host`
- Extension: `extension/src` (content/background, bridge: `__rznExecuteStep`, `captureEnhancedDOMSnapshot`)

Comms flow:
- Extension ⇄ Native host: Chrome native messaging (JSON frames)
- Native host ⇄ Supervisor: local `rzn.local.v1` socket/token handshake
- CLI/MCP/app/cloud ⇄ Supervisor: local `rzn.local.v1` calls

Migrations policy:
- No schema DDL in code. Keep all DDL under a `migrations/` directory if/when added. Guardrail: `make sg-guards` (set `STRICT=1` to fail).

Scoped workflow:
- Map/context: `make scope`
- Quick lookup: `make scope-q Q="..."`
- Guardrails: `STRICT=1 make sg-guards`
- Agent flow: `make agent-run M="..." [S=1]` then `make agent-validate OUT=... STRICT=1`
