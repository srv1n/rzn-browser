# Feature: RZN Browser Rename

## Overview
Rename the repo-local browser surface to `rzn-browser` everywhere we own it. The standalone contract is `rzn-browser ...`; the umbrella contract is `rzn browser ...` with argument passthrough after `browser`. Old browser naming is removed instead of preserved behind aliases.

## Flow Diagrams
End-to-end naming surface:
```text
standalone install
  rzn-browser <subcommand> ...
          |
          v
    rzn-browser binary
          |
          v
native host: com.rzn.browser.broker
plugin id : rzn-browser

umbrella install
  rzn browser <subcommand> ...
          |
          v
wrapper forwards argv[2..] unchanged
          |
          v
    rzn-browser <subcommand> ...
```

Canonical surface:
```text
rzn-browser binary       -> standalone CLI
rzn browser ...          -> umbrella wrapper passthrough
com.rzn.browser.broker   -> native host id
crates/rzn_browser       -> standalone CLI crate path
```

## Decision Record
- Chosen: rename the standalone binary/package to `rzn-browser`. The old names were implementation leakage.
- Chosen: rename the workspace crate path to `crates/rzn_browser` so repo-local docs and scripts stop speaking two different names.
- Chosen: document the umbrella wrapper contract instead of implementing umbrella CLI behavior in this repo. This repo owns the standalone binary, not the parent CLI.
- Rejected: keeping aliases “for safety.” We own the downstreams, so dragging dead names around would just preserve confusion.

## Architecture
- Standalone CLI:
  - `crates/rzn_browser/Cargo.toml` publishes `rzn-browser` as the package and primary bin.
  - `crates/rzn_browser/src/main.rs` exposes `rzn-browser` as the CLI name.
- Install/runtime:
  - `setup.sh`, `start_app.sh`, and bundle scripts install/use `rzn-browser` as the canonical CLI.
  - standalone app base defaults to `~/Library/Application Support/rzn-browser`.
- Native host:
  - host id is `com.rzn.browser.broker`.
- Plugin/release:
  - canonical plugin id is `rzn-browser`.
  - release config/script names and Make targets move to `rzn-browser`.

## Implementation Notes
- Wrapper contract for downstreams:
  - `rzn browser native-run ...` must behave exactly like `rzn-browser run --via native ...`.
  - No flag translation, no subcommand rewriting, no hidden defaults.
- Bundle/release:
  - new Make targets are `plugins-build-rzn-browser-macos` and `plugins-publish-rzn-browser-*`.
  - old browser rename aliases are removed.
- Native host/runtime:
  - write/install the canonical manifest name only.
  - connect to the canonical host id only.

## Tasks & Status
- [x] Rename standalone binary/package/install docs to `rzn-browser`
- [x] Document mirrored standalone and umbrella command grammar
- [x] Rename plugin bundle/release metadata to `rzn-browser`
- [x] Remove browser rename compatibility aliases and fallback ids
- [x] Rename the standalone CLI crate path to `crates/rzn_browser`
- [x] Sweep repo-local docs and CI references
- [ ] Rename repo directory on disk and remote GitHub slug outside this workspace when the surrounding tooling is ready

## What Works (Do Not Change)
- `rzn-browser-worker` and `rzn-native-host` stay as internal runtime binaries unless a packaging need forces a rename.
- The umbrella CLI remains a thin wrapper contract, not a forked command surface.

## Tried & Didn’t Work
- Treating `rzn-browser` as doc-only while keeping old names alive in scripts and crate paths: fails the rename because the repo still teaches two names.
- Keeping compatibility aliases after the contract is settled: buys nothing and leaves a permanent footgun.
