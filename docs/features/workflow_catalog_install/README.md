# Workflow Catalog Install

## Overview
Ship deterministic JSON workflows and packaged examples as part of the installed runtime so users can run namespaced workflow ids like `google/search` and `examples/search-google` without cloning the repo or typing repo-relative paths, while keeping a separate user-owned workflow directory for custom flows and imports.

## Flow Diagrams
End-to-end install + resolution flow:
```text
repo workflows/ JSON
   |
   | make install / install.sh
  v
RZN runtime dir
  ├─ bin/
  │   ├─ rzn-browser
  │   └─ rzn-native-host
  ├─ extension/
  │   └─ dist-chrome/  <- stable unpacked extension copy
  └─ workflows/
      ├─ builtin/   <- shipped JSON workflows
      │   └─ examples/browser_automation/*.json
      └─ user/      <- imported or generated user workflows

rzn-browser run google search
   |
   v
workflow resolver
  ├─ explicit file path?
  ├─ canonical workflow id (`system/workflow`)?
  ├─ user-installed workflow?
  ├─ builtin workflow?
  └─ legacy ~/.rzn/workflows fallback?
   |
   v
resolved JSON path -> run / run-workflow
```

Alias precedence:
```text
explicit path > user workflow id > builtin workflow id > legacy fallback
```

## Decision Record
- Use an installed `builtin/` catalog instead of repo-relative `workflows/` paths. Requiring a clone for shipped workflows is dumb.
- Keep `user/` separate from `builtin/` so upgrades can replace shipped workflows without stomping user files.
- Install a stable `extension/dist-chrome` copy into the runtime so the unpacked extension does not depend on keeping the repo checkout around.
- Use canonical ids in the form `system/workflow`, with CLI sugar that accepts `rzn-browser run <system> <workflow>`.
- Keep legacy `~/.rzn/workflows` as a fallback read path so old setups do not instantly break.
- Add `install.sh` as the distribution-facing installer entrypoint and keep `setup.sh` as the lower-level bootstrap script.
- Add `workflow pull` so shipped workflows/examples can be refreshed independently of the binary install.

## Architecture
- Installer:
  - [install.sh](../../../../install.sh): release-oriented entrypoint.
  - [setup.sh](../../../../setup.sh): builds binaries, installs native-host manifest, installs a stable extension copy, and refreshes the builtin runtime catalog.
  - [scripts/release/build_release_artifacts.sh](../../../../scripts/release/build_release_artifacts.sh): builds sh-installable runtime/workflow tarballs.
- Resolver:
  - [crates/rzn_browser/src/workflow_catalog.rs](../../../../crates/rzn_browser/src/workflow_catalog.rs): runtime root discovery, builtin catalog installation, canonical id inference, legacy fallback handling, workflow import, and resolution.
- CLI wiring:
  - [crates/rzn_browser/src/main.rs](../../../../crates/rzn_browser/src/main.rs): `run` and `run-workflow` accept `<system> <workflow>` or `system/workflow`; `workflow list|catalog|dirs|add|pull` expose and refresh the installed catalog.
- Planner defaults:
  - [crates/rzn_plan/src/lib.rs](../../../../crates/rzn_plan/src/lib.rs): default user workflow directory now points at the installed runtime path.

## Implementation Notes
- Runtime root defaults to `dirs::data_local_dir()/RZN`.
- Installer copies JSON files from repo `workflows/` into runtime `workflows/builtin/`, excluding test fixtures (`tests/*`, `test-*.json`).
- Installer also copies `examples/browser_automation/*.json` into runtime `workflows/builtin/examples/browser_automation/`.
- Installer copies `extension/dist-chrome` into the runtime so Chrome loads a stable path outside the repo checkout.
- User imports land in runtime `workflows/user/`.
- `rzn-browser workflow pull` refreshes the builtin catalog from either a local repo root (`--repo-root`) or the latest GitHub release workflow tarball.
- Single-file imports require `--system` and `--name` unless the workflow JSON itself declares `system` + `workflow` metadata.
- `rzn-browser workflow list [system]` shows installed workflow ids, legacy aliases, source, and resolved paths.
- `rzn-browser run` and `rzn-browser run-workflow` accept `<system> <workflow>`, `system/workflow`, legacy flat aliases, or direct file paths.
- Downstream umbrella wrapper contract: `rzn browser ...` should mirror the same trailing grammar and flags.

## Tasks & Status
- [x] Add release-facing `install.sh`
- [x] Copy shipped workflow JSONs into an installed builtin catalog
- [x] Copy packaged examples into the installed builtin catalog
- [x] Add `workflow pull` for catalog refreshes
- [x] Add sh-installable release artifacts
- [x] Add namespaced workflow resolution for CLI execution paths
- [x] Add user workflow import command
- [x] Add catalog and directory inspection commands
- [ ] Add catalog-aware docs for each shipped workflow pack
- [ ] Add alias collision tests

## What Works (Do Not Change)
- Builtin workflows remain plain JSON files that can still be run directly by explicit path.
- User workflows override builtin workflow ids when names collide.
- Native host name remains `com.rzn.browser.broker`.
- Manual Chrome extension load remains manual; the installer should not fake auto-install.

## Tried & Didn’t Work
- Repo-relative workflow paths as the primary UX:
  Fine for developers, garbage for end users.
- One shared workflow directory for both shipped and user files:
  Upgrade behavior becomes unsafe and you lose a clean override model.
- Flat aliases as the primary product surface:
  Fine as compatibility sugar, bad as the long-term namespace model.
