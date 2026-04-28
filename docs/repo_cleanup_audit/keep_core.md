# Keep Core

These are the parts that are clearly live and should not be swept out just because the repo looks noisy.

| Path | Why it stays |
| --- | --- |
| `crates/rzn_browser/` | Main `rzn-browser` CLI. |
| `crates/rzn_plan/` | Planner/orchestrator engine. |
| `crates/rzn_core/` | Shared workflow DSL and schema handling. |
| `crates/rzn_contracts/` | Versioned wire contracts. |
| `crates/rzn_browser_worker/` | Active worker runtime. |
| `crates/rzn_native_host/` | Active native messaging host. |
| `crates/rzn_broker_endpoint/` | Current worker/native-host coordination layer. |
| `crates/rzn_sdk/` | Actively imported by the CLI host layer. |
| `crates/rzn_plugin_devkit/` | Release/plugin bundle tooling. |
| `extension/src/` | Real extension product code. |
| `extension/tests/e2e/` | Real frontend e2e coverage. |
| `extension/package.json` + `extension/bun.lock` | Actual JS toolchain source of truth. |
| `workflows/<domain>/` | Shipped workflow catalog surface. |
| `examples/browser_automation/` | Packaged examples used by install/plugin flow. |
| `resources/systems/browser_automation/` | Active plugin/runtime metadata. |
| `docs/features/` | Required feature scratchpads and real engineering memory. |
| `test/fixtures/` | Shared local fixtures used by extension e2e and manual harnesses. |
| `test/manual/README.md` + `test/manual/workflows/` | Manual debugging harness that still maps to repo-local flows. |
| `scripts/release/` | Current install/release pipeline. |
| `scripts/build-ext.ts` | Manifest/output assembly used by extension build flow. |
| `setup.sh`, `install.sh`, `install.ps1`, `start_app.sh` | Active setup/install/run entrypoints. |
| `Makefile` | Current operator entry surface for build/test/release commands. |

## Keep, but maybe simplify later

These are live enough to keep for now, but still candidates for later tightening:

- `test/manual/browser/` and `test/manual/scripts/`
- `docs/workflows/`
- `resources/cards/`
- `skills/`
- `rzn_broker/`
- `crates/rzn_eval/`
