# Feature: Browser System Metadata

## Overview

Goal: ship the browser plugin as a first-party **system provider**, not just a pair of binaries, by
packaging `browser_automation` metadata plus curated example workflows inside the signed
`rzn-browser` bundle. The constraints come from the RZN plugin bundle spec and the rznapp system
metadata/result-shaping handoffs: capability groups must be user-meaningful, setup must be
explicit, quick starts must avoid raw JSON-first onboarding, and pure action outputs must stay out
of indexable result views by default.

## Flow Diagrams

### Bundle authoring and packaging

```text
repo sources
  ├─ resources/systems/browser_automation/system.metadata.yaml
  ├─ examples/browser_automation/*.json
  └─ scripts/plugins/config/rzn-browser.json
          |
          v
rzn_plugin_devkit build
  ├─ hashes worker binaries
  ├─ hashes metadata/examples payloads
  ├─ emits plugin.json + plugin.sig
  └─ writes signed ZIP
```

### Host-facing system surface

```text
rzn-browser plugin bundle
  ├─ worker binaries
  ├─ native host
  ├─ system metadata
  └─ curated workflow examples
          |
          v
host loader (future)
  ├─ setup checklist
  ├─ capability groups
  ├─ quick starts
  └─ result shaping rules
```

## Decision Record

- Chosen: author one `browser_automation` system metadata file now and package it as a signed
  resource. This matches the handoff contract and avoids re-authoring metadata later inside app UI
  code.
- Chosen: ship curated workflow examples under `examples/browser_automation/` instead of pointing
  users directly at raw `browser.execute_step` payloads. This is the minimum sane onboarding path
  for browser automation.
- Chosen: model `browser.execute_step` with default action-style result handling plus
  extraction-specific variants. The worker currently multiplexes many step types through one tool,
  so a single blanket `index_view` would be misleading.
- Rejected: only documenting the handoff without wiring bundle payloads. That would produce dead
  metadata not present in `plugin.json` or the signed ZIP.

## Architecture

- `resources/systems/browser_automation/system.metadata.yaml`
  Declares the browser system identity, capability groups, setup steps, context parameters, quick
  starts, and result-handling guidance.
- `examples/browser_automation/*.json`
  Curated workflows that the host can surface as quick starts or packaged examples.
- `scripts/plugins/config/rzn-browser.json`
  Includes the metadata/examples directories as signed payloads and marks the system metadata
  directory as a plugin resource.
- `crates/rzn_plugin_devkit/src/main.rs`
  Verifies that directory resources and examples are included in the payload map and therefore in
  the signed manifest.
- `crates/rzn_browser_worker/src/main.rs`
  Reports the packaged browser system metadata/examples through `rzn.worker.health` so install-time
  diagnostics can prove the bundle is self-describing before the desktop loader consumes it.

## Implementation Notes

- The metadata uses `workflow_path` quick starts because the host loader is not implemented in this
  repo yet and browser automation needs curated workflows more than individual raw tool calls.
- `browser.snapshot` intentionally keeps `index_view.mode: none`; the handoff explicitly warns
  against pushing large DOM-ish payloads into LLM/index views by default.
- `browser.execute_step` is split into:
  - default action handling with `index_view.mode: none`
  - `extract_structured_data` handling with `index_view.mode: records`
  - `get_element_text` handling as a small read-only projection
- `rzn.worker.health` now includes a `packaged_browser_system` block with the installed
  `system.metadata.yaml` path, example directory path, and packaged example file list. That keeps
  plugin-side diagnostics aligned with the new systems install/discovery contract without adding a
  new worker tool.

## Tasks & Status

- [x] Add `resources/systems/browser_automation/system.metadata.yaml`
- [x] Add curated browser quick-start workflows under `examples/browser_automation/`
- [x] Register metadata/examples in the signed rzn-browser bundle config
- [x] Add plugin-devkit coverage for directory resources in bundle manifests
- [x] Expose packaged metadata/example inventory through `rzn.worker.health`
- [ ] Wire the host loader to consume this metadata and render quick starts/result views

## What Works (Do Not Change)

- The plugin bundle remains a standard signed `plugin.json` + `plugin.sig` ZIP; no bundle format
  changes are required.
- Worker and native-host binaries remain the runtime control plane for browser automation.
- Pure action acknowledgements remain non-indexable by default.

## Tried & Didn’t Work

- Treating the browser plugin as only binaries plus a README: insufficient for the systems UX
  contract and does not satisfy the handoff’s quick-start/setup/result-shaping requirements.
- Giving `browser.execute_step` one generic index policy: too coarse because extraction outputs and
  click/type/navigation acknowledgements have materially different downstream value.
