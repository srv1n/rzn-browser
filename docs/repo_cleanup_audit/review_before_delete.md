# Review Before Delete

These are the parts that smell old, bloated, or scope-crept, but they still touch docs, release flow, tests, or runtime wiring enough that deleting them cold would be sloppy.

| Path | Why it is suspicious | Why it is not a blind delete |
| --- | --- | --- |
| `rzn_broker/` | Legacy standalone broker architecture. | Still built by `make phase3`, still referenced by tests, and still part of some docs. |
| `crates/rzn_eval/` + `eval/` | Feels like an abandoned evaluation harness. | It is a workspace member with its own bins and docs, but not part of the main install/release spine. |
| `bindings/node/` | Placeholder package with no implementation tree. | Easy removal candidate, but if you want future SDK bindings it may become the seed. |
| `examples/aso_phone/` | Appium/iOS collector code looks like sidequest territory. | It is real code with its own README, but not wired into the main runtime or release path. |
| `examples/action_surface/` | Demo-only quickstart docs. | Safe to split out if you want a tighter product story, but harmless if you still want reference snippets. |
| `resources/cards/` | Looks like spec/catalog content without obvious runtime consumers. | Docs reference it heavily, especially for X workflows. |
| `workflows/generated/` | Mixed bag: debug junk and active ASO workflow content live in the same directory. | `generated/aso/` is actively called by skill wrappers and docs; the rest mostly looks disposable. |
| `workflows/tests/` + top-level `workflows/test-*.json` | Real test assets, not product catalog. | Installer code already skips them, but manual harnesses and docs still use some of them. |
| `docs/workflows/` | Documentation drift. | Useful if kept aligned; currently some pages describe legacy file names and older resource names. |
| `docs/bugs/` | Single orphaned bug memo. | Could be folded into feature docs or deleted, but not urgent. |
| `extension/dist-firefox/` + `extension/src/manifest.firefox.json` | Firefox path looks half-dead. | Still built/staged by current scripts, so removal requires release-flow edits. |
| Root `package.json` | Looks underused and oddly detached from actual repo scripts. | Deleting a root manifest can break tooling expectations even if nobody uses it intentionally. |
| `skills/` | Scope creep from “browser runtime” into domain-specific workflow wrappers. | Actively called by `Makefile` and referenced by workflow docs; decide based on repo scope, not aesthetics. |
| Root `tests/` Python tests | Real tests, but disconnected from documented test commands. | They validate helper scripts; decide whether those scripts remain part of the repo story. |
| `.mailmap` | Possibly obsolete after history reset. | Harmless, but maybe pointless in a freshly rewritten repo. |

## Recommended calls

### Call 1: product scope

Decide whether this repo is:

- a focused browser runtime + extension + release bundle repo, or
- a kitchen sink for ASO, assistant sync, X export, app-store phone tooling, and workflow experiments.

That one decision determines whether `skills/`, large chunks of `workflows/generated/`, `examples/aso_phone/`, and helper Python scripts stay or go.

### Call 2: legacy transport

Pick one of these and commit to it:

- keep `rzn_broker/` as a supported architecture surface, or
- migrate fully to worker + native-host and start deleting legacy broker paths/tests/docs.

### Call 3: evaluation story

Either:

- adopt `rzn_eval` and make it part of the real test/release story, or
- remove it before it continues aging into decorative complexity.
