# Delete Now

These are the cleanup candidates that are either generated, cached, machine-local, or already marked disposable by repo behavior.

| Path | Why it is safe | Evidence |
| --- | --- | --- |
| `.beads/` | Local Beads database/log/state. Not product runtime. If you are removing Beads from the repo story, this is dead weight. | Contains `beads.db`, `daemon.log`, `issues.jsonl`, `metadata.json` and other local state files. |
| `target/` | Rust build output. | Already ignored in `.gitignore`; 506 MB local buildup. |
| `extension/node_modules/` | JS dependency install cache. | Already ignored; 155 MB of local package state. |
| `extension/.pw-user-data*/` | Playwright persistent browser profiles. | Already ignored by `.gitignore`; multiple 4-7 MB directories. |
| `extension/test-results/` | Test artifact output. | Already ignored by `.gitignore`. |
| `output/` | Generated helper output, not source. | `.gitignore` marks it generated; current content includes assistant output under `output/assistants/`. |
| `com.rzn.browser.broker.json` | Local machine native-host manifest snapshot. | `.gitignore` already ignores it; path is machine-specific. |
| `extension/pnpm-lock.yaml` | Stale lockfile from the wrong package manager. | Repo uses Bun in `Makefile`, `README.md`, and `extension/package.json`; pnpm lock is version-drifted. |
| `pnpm-lock.yaml` | Root lockfile appears fossilized and mismatched with current root `package.json`. | Root JS tooling does not use pnpm in current docs/scripts. |
| `tests/__pycache__/` | Generated Python bytecode. | Local cache only. |
| `scripts/__pycache__/` | Generated Python bytecode. | Local cache only. |
| `scripts/release/__pycache__/` | Generated Python bytecode. | Local cache only. |
| `examples/aso_phone/__pycache__/` | Generated Python bytecode. | Local cache only. |
| `docs/index/agent_runs/` | Generated scope snapshots, not authoritative docs. | `scripts/agent/agent-run.sh` writes there. |
| `docs/index/TREE.md`, `docs/index/TREE_DEPTH_.md`, `docs/index/HOTSPOTS.rg`, `docs/index/CONTEXT_SNIPPETS.md`, `docs/index/REDUCERS_INDEX.md`, `docs/index/INVARIANTS.md`, `docs/index/SUMMARY.md` | Generated context/index files. | `make scope` and related scripts regenerate them. |
| `.DS_Store` files | Finder garbage. | Local metadata only. |

## Paired cleanup edits

When you actually delete the items above, also clean these policy leftovers:

| File | Follow-up |
| --- | --- |
| `.gitignore` | Add `__pycache__/` and `*.pyc` if you want Python junk gone for good. |
| `.gitattributes` | Remove the `.beads/issues.jsonl merge=beads` rule if Beads is removed. |
| `AGENTS.md` | Remove or rewrite the Beads-specific tracking workflow if that process is dead. |

## One caveat

Do not blindly delete `extension/dist-chrome/` and `extension/dist-firefox/` in the same sweep. They are generated, yes, but the current release/install flow still treats them as staged payloads.
