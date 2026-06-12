# Epic: RZNApp Browser Tools Extension Bundle (Build + Sign + Publish)

We want Browser Tools to ship as a **signed RZN extension** that can be installed into the desktop app (`rznapp`) from a backend catalog, without cloning/building the desktop repo.

This epic is the concrete work to make that end-to-end loop undeniable:

1) Build artifact ZIP in this repo (per platform)  
2) Upload to R2  
3) Register release to backend (Option B)  
4) Backend publishes signed catalog  
5) Desktop installs and runs health/echo tools

Reference docs:

- Feature scratchpad: `docs/features/rznapp_extension_bundle/README.md`
- Backend spec: `/Users/sarav/Downloads/side/rzn/backend/docs/specs/extensions_registry_db_and_catalog_publish_v1.md`
- Desktop memo: `/Users/sarav/Downloads/side/rzn/rznapp/docs/design/EXTENSIONS_BUILD_SIGN_PUBLISH_INSTALL_STRATEGY_MEMO_2026-02-13.md`

## Story 1: Define canonical plugin identity + bundle layout (rzn-browser)

### Type
feature

### Description

Lock the stable IDs and bundle layout so other repos (backend + desktop) can depend on them.

- plugin id: `rzn-browser`
- worker id: `worker`
- MCP server id in host: `plugin.rzn-browser.worker`

Bundle must contain:

- `plugin.json` + `plugin.sig`
- worker binary
- native host binary (+ any templates/manifests needed)

### Acceptance Criteria

- IDs are documented and stable in `docs/features/rznapp_extension_bundle/README.md`.
- Bundle layout is documented and matches what the devkit expects.

## Story 2: Build script produces signed ZIP artifact (per platform)

### Type
feature

### Description

Add a single build entrypoint (script or Make target) that:

1. builds the worker + native host
2. creates bundle directory layout
3. signs `plugin.json` to produce `plugin.sig`
4. zips deterministically (as much as possible)
5. outputs:
   - `dist/plugins/<plugin_id>/<version>/<platform>/<zip>`
   - `dist/plugins/<...>.sha256` (or prints sha256)

### Acceptance Criteria

- Running the build produces a ZIP artifact that `rznapp` can install via “Install from file…” and run `rzn.supervisor.health`.
- Output paths are predictable and documented.

## Story 3: CI publish to R2 + register release to backend (Option B)

### Type
feature

### Description

Create a CI workflow that:

- uploads the artifact ZIP to R2 under the standard prefix
- computes sha256
- calls backend admin API `POST /admin/plugins/releases` to register the release

Secrets required (Phase 1):

- R2 write credentials
- backend admin token
- bundle signing private key (first-party)

### Acceptance Criteria

- A CI run can publish an artifact to R2 and register it in backend DB.
- Backend can later publish a catalog that includes this release.

## Story 4: Local dev runbook (install from file + tool bench smoke)

### Type
task

### Description

Write a crisp local smoke guide that proves:

- build zip locally
- install into `rznapp`
- enable extension
- run tools (health/echo) and see output

### Acceptance Criteria

- A new developer can follow the doc and see success in < 5 minutes.
