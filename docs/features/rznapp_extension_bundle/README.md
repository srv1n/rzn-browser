# Feature: RZNApp Extension Bundle (RZN Browser)

## Overview

Goal: produce a **signed, installable RZN extension ZIP** (aka “plugin bundle”) for RZN Browser so that:

- `rznapp` can install/update it from a signed backend catalog (Option B).
- Authors do **not** need to clone/build the `rznapp` repo to ship browser automation capability.

This feature is about **packaging + distribution** of the browser tools runtime, not the automation logic itself.

Scope: first-party publishing (RZN-owned) for launch; third-party signing later.

Release completion rule: a build is **not** done when the ZIP exists. It is done only after the
repo has notified the backend through the admin API and the release has been pushed through both
the local backend (`http://localhost:8082`) and cloud (`https://cloud.rzn.ai`). If either target
fails, the release pass stops and the failure must be reported with the exact broken step/target.

## Flow Diagrams

### Runtime (user machine)

```
RZN Desktop (rznapp)
  |
  | installs & verifies extension zip
  v
RZN Browser extension bundle (installed payload)
  - MCP worker binary (stdio)
  - Native host (chrome native messaging)
  - (optional) helper scripts/metadata
  |
  | spawns worker
  v
MCP tool calls -> worker -> native host -> Chrome extension -> browser page
```

### Build/Publish (CI / release pass)

```
rzn-browser repo CI
  | build worker + native host for platform
  | create plugin bundle directory
  | sign plugin.json -> plugin.sig
  | zip -> artifact.zip + sha256
  | upload artifact.zip to R2 (immutable key)
  | call backend admin API: register release on local
  | call backend admin API: publish catalog on local
  | call backend admin API: register release on prod
  | call backend admin API: publish catalog on prod
  v
backends publish signed index.json/index.sig (+ current.json pointer)
```

## Decision Record

### Why ship RZN Browser as an extension bundle?

- Keeps `rznapp` a stable kernel/host.
- Makes iteration faster: build artifact, install in release host, test end-to-end.
- Enables future ecosystem: multiple tool bundles installed without rebuilding the desktop.

### Catalog coordination model

We follow Option B:

- Backend owns a DB registry of releases.
- Backend generates and signs the catalog.

This prevents race conditions where multiple repos publish different `index.json` concurrently.

### Backend notification is part of release, not follow-up

The backend runbook is explicit about this: no human ping is required, but the plugin repo must
notify the backend through the publish API contract. In practice that means:

1. build the immutable ZIP,
2. upload the ZIP to object storage,
3. register the release with backend metadata,
4. publish the catalog,
5. repeat the backend notification/publish pass for both local and cloud.

If step 3 or 4 is skipped, the release is not live no matter how correct the ZIP is.

### Signing key storage (Phase 1)

- Catalog signing private key lives in backend env (pre-launch pragmatism).
- Bundle signing key lives in CI secrets for first-party bundles.

## Architecture

### Bundle contents (RZN Browser)

The bundle should include the minimum required to expose MCP tools:

- MCP worker binary (stdio)
- Native host binary + any manifest/templates needed for installation
- `plugin.json` (manifest) + `plugin.sig` (signature)

For the shareable local macOS bundle used outside `rznapp`, include:

- unpacked Chrome extension payload
- full `workflows/` tree
- bundle-local `AGENTS.md` and README so coding agents can discover and run shipped workflows without repo context

The Chrome extension itself may remain “store installed” or “unpacked in dev”.
The bundle can include UX help and “Install extension” deep links, but should avoid auto-installing browser extensions silently.

### Stable IDs

- `plugin_id`: stable (e.g., `rzn-browser`)
- `worker_id`: stable (e.g., `worker`)
- MCP server exposed in host as `plugin.rzn-browser.worker`

## Implementation Notes

### Versioning

- Semver for plugin version.
- Platform suffixes: `macos_universal`, `windows_x86_64`, etc.

### Determinism

- Bundle zips should be reproducible (same inputs -> same hash) when possible.
- Manifest JSON must be serialized deterministically before signing.

## Tasks & Status

- [x] Define canonical plugin id + bundle layout for RZN Browser.
- [x] Add build script that produces signed ZIP artifacts per platform.
- [ ] Add CI job to upload artifact + register release in backend.
- [x] Add docs/runbook for local install-from-file smoke.
- [x] Add local helper to build once and notify/publish local + prod backends.
- [x] Document backend notification as a release requirement, not an optional handoff.
- [ ] Add CI job to upload artifact + register/publish release in backend automatically.

## Local Runbook: Install from file (rznapp)

This proves Loop A from the strategy memo (build ZIP locally → install via Desktop UI).

### 1) Generate a dev signing key

Recommended: reuse the desktop app’s dev signing key so debug builds of `rznapp` verify your bundle without extra env vars.

```bash
ls /Users/sarav/Downloads/side/rzn/rznapp/.secrets/plugin-signing/ed25519.private
```

If you don’t have it yet, generate a dev keypair in this repo:

```bash
cd /Users/sarav/Downloads/side/rzn/rzn-browser
make plugins-keygen
```

### 2) Build a signed RZN Browser bundle ZIP (macOS)

This builds:
- `rzn-browser` (supervisor CLI/runtime)
- `rzn-native-host` (native messaging host)

```bash
cd /Users/sarav/Downloads/side/rzn/rzn-browser
make plugins-build-rzn-browser-macos
```

Output (example):

```
dist/plugins/rzn-browser/0.1.0/macos_universal/rzn-browser-0.1.0-macos_universal.zip
dist/plugins/rzn-browser/0.1.0/macos_universal/rzn-browser-0.1.0-macos_universal.zip.sha256
```

### 3) Install via rznapp: Settings → Extensions → Install from file…

In debug/dev builds, `rznapp` must trust your dev public key.

Options:
- If you used `../rznapp/.secrets/plugin-signing/ed25519.private`, `rznapp` already trusts the matching public key.
- Otherwise, start `rznapp` with `RZN_PLUGIN_PUBKEY_B64="$(cat .secrets/plugin-signing/ed25519.public)"`.

Then:
1. Open `rznapp`
2. Settings → Extensions
3. Install from file… → choose the ZIP under `dist/plugins/...`
4. Enable the extension/plugin
5. Use Tool Bench to run the RZN Browser supervisor health/echo (e.g. `rzn.supervisor.health`)

### 4) Sanity verify a ZIP locally (optional)

```bash
cd /Users/sarav/Downloads/side/rzn/rzn-browser
make plugins-verify ZIP=dist/plugins/rzn-browser/0.1.0/macos_universal/rzn-browser-0.1.0-macos_universal.zip PUB=.secrets/plugin-signing/ed25519.public
```

## Publish to Backend Catalog (Option B)

In Option B, the backend owns the catalog and signs it; this repo only uploads an
artifact and registers a release.

Prereqs (env):

- `RZN_PLATFORM_ADMIN_TOKEN_LOCAL` or fallback `RZN_PLATFORM_ADMIN_TOKEN`
- `RZN_PLATFORM_ADMIN_TOKEN_PROD` or fallback `RZN_PLATFORM_ADMIN_TOKEN`
- `R2_PLUGINS_*` (see backend runbook / pysandbox runbook)

Standard release command (build once, then notify/publish both backends):

```bash
cd /Users/sarav/Downloads/side/rzn/rzn-browser
make plugins-publish-rzn-browser-all
```

Target mapping:

| Target | Default backend |
| --- | --- |
| `local` | `http://localhost:8082` |
| `cloud` | `https://cloud.rzn.ai` |
| `prod` | legacy alias for `cloud` |
| `all` | local, then cloud |

Notes:
- The script uploads the artifact once, then registers/publishes sequentially per backend target.
- If local succeeds and cloud fails, the command exits on the cloud failure and reports that exact
  target. Do not describe the release as complete in that state.
- `--targets env` remains available for one-off/manual cases that use `RZN_BACKEND_BASE_URL`.
- Legacy compatibility: some downstreams may still refer to `rzn-browser`, but new bundles, catalog entries, and docs use `rzn-browser` as the canonical plugin id.

## What Works (Do Not Change)

- Same-origin restrictions and host-side verification in `rznapp` remain mandatory for the verified distribution path.
- The desktop should always verify signatures and sha256 before install.
- Backend notification through register + publish is part of release completion.

## Tried & Didn’t Work

- Directly editing backend `index.json` from multiple repos leads to races and signature mismatch windows.
