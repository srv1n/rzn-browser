# Repo Map (Start Here)

Goal: stealth-first browser automation with a Rust workspace and an MV3 extension communicating through the native host plus browser worker bridge.

Key paths:
- crates/: Rust workspace crates (rzn_browser, rzn_core, rzn_plan, rzn_sdk, rzn_native_host, rzn_browser_worker)
- extension/: MV3 extension (TypeScript, Vite, Vitest, Playwright); build → dist-chrome/
- workflows/: JSON workflows runnable via CLI/scripts
- test/ and tests/: HTML/manual test pages and Rust integration/unit tests
- .env.example → .env: LLM provider and runtime options

Entry points:
- CLI: `./target/release/rzn-browser`
- Native host: `./target/release/rzn-native-host`
- Browser worker: `./target/release/rzn-browser-worker`
- Extension: `extension/src` (content/background, bridge: `__rznExecuteStep`, `captureEnhancedDOMSnapshot`)

Comms flow:
- Extension ⇄ Native host: Chrome native messaging (JSON frames)
- Native host ⇄ Browser worker: local browser bridge socket + JSON-RPC `browser.session`
- CLI ⇄ Browser worker: MCP stdio + unified logging

Migrations policy:
- No schema DDL in code. Keep all DDL under a `migrations/` directory if/when added. Guardrail: `make sg-guards` (set `STRICT=1` to fail).

Scoped workflow:
- Map/context: `make scope`
- Quick lookup: `make scope-q Q="..."`
- Guardrails: `STRICT=1 make sg-guards`
- Agent flow: `make agent-run M="..." [S=1]` then `make agent-validate OUT=... STRICT=1`
