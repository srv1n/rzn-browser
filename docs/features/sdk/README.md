# SDK (rzn_sdk) – v1 substrate

## Overview
This feature introduces an embedding-first Rust SDK (`crates/rzn_sdk`) so non-CLI applications (e.g., a Tauri desktop backend) can embed RZN capabilities through a stable library surface.

The SDK is split into:
- **Stable surface** (default): `rzn_sdk::{host, broker}` with small, serializable request/config types suitable for app embedding.
- **Power escape hatch**: `rzn_sdk::unstable` behind the `unstable` feature, which exposes full `rzn_plan` internals for internal iteration. No semver guarantees.
In addition, v1 adds a **deterministic substrate surface**:
- `rzn_sdk::Session` (connect/snapshot/apply/transcript)
- `rzn_sdk::BrowserTools` (stable observe/act/execute_steps/page_source)
- `crates/rzn_contracts` (versioned contracts: Snapshot/Action/ActionResult/Transcript)

## Flow Diagrams
- End-to-end (host app driving the browser via the broker + extension)
```
Host app (Tauri/CLI) → rzn_sdk::BrowserTools → Broker IPC (pipe/tcp) → Broker → Extension (SW) → Content Script → Page
                      ←                     ←                   ←        ←               ←              ←
```

- Installation (bundling broker as a sidecar + installing native host manifest)
```
App installer/runtime → write native host manifest → Chrome loads manifest → Extension launches broker (stdio)
```

## Decision Record
- Chosen: add a dedicated facade crate (`rzn_sdk`) with feature gates and a stable/unstable split.
  - Pros: stable import surface for downstream apps; keeps iteration velocity high (engine can churn); clean split between “host API” and “distribution/manifest utilities”.
  - Cons: two tiers of API exist; internal teams must be disciplined about using `unstable` only when necessary.
- Chosen (v1): keep the SDK as an Actor-grade deterministic substrate. The host application owns all LLM prompting and planner-loop logic.
- Alternatives:
  - “Just use `rzn_plan` directly”: workable today, but couples downstream apps to planner internals and makes future refactors harder.
  - “Bundle broker in-process”: not viable for native messaging, which requires a separate process launched by Chrome over stdio.
  - “Have `rzn_sdk` build and embed the broker binary automatically”: possible for internal builds, but brittle and non-portable (cross-target, codesigning, installer integration). Prefer sidecar binaries.

## Architecture
- Modules
  - `crates/rzn_sdk/src/host.rs`: stable wrapper over the engine (`rzn_plan::Orchestrator`) intended for embedding.
  - `crates/rzn_sdk/src/broker.rs` (feature `broker`): native messaging host manifest helpers for installing/removing the broker manifest.
  - `crates/rzn_sdk/src/session.rs` (feature `host`): deterministic broker session (connect/snapshot/apply + transcript).
  - `crates/rzn_sdk/src/tools.rs` (feature `host`): stable tool surface for downstream apps (`BrowserTools`).
  - `crates/rzn_sdk/src/unstable.rs` (feature `unstable`): re-exports of `rzn_plan` internals for power-users.
  - `crates/rzn_sdk/src/prelude.rs`: convenience re-exports for downstream apps.
  - `crates/rzn_contracts/src/v1.rs`: versioned wire contracts (v1).
- Data contracts
  - Host operations: `rzn_sdk::host::{PlanRequest, PlanResponse, RunRequest, RunResponse}`
  - Host config: `rzn_sdk::host::HostConfig`
  - Steps/workflows: `rzn_core::{Step, StepKind, Workflow}` (re-exported via the SDK prelude)
  - Substrate (v1): `rzn_contracts::v1::{SnapshotV1, ActionV1, ActionResultV1, TranscriptV1}`
  - Native host manifest JSON: `NativeMessagingHostManifest` (matches Chrome’s native messaging host schema)

## Implementation Notes
- Entry points
  - Host:
    - `rzn_sdk::host::Host::from_env()` / `Host::new(HostConfig)`
    - `Host::{plan_llm_only, plan_auto, run}`
    - Escape hatch: enable `features = ["unstable"]` and use `rzn_sdk::unstable::*` for direct engine access.
  - Broker (distribution):
    - `rzn_sdk::native_host::NativeMessagingHostManifest::rzn_native_host(native_host_path, extension_id)`
    - `rzn_sdk::native_host::NativeMessagingHostManifest::rzn_native_host_with_origins(native_host_path, origins_or_ids)`
    - `install_rzn_native_host_for_browser(...)` / `install_rzn_native_host_for_browser_with_origins(...)`
      return `NativeHostInstallReport` with browser target, manifest path, native-host path, allowed origins, and changed/no-op status.
    - `uninstall_rzn_native_host_for_browser(...)`
      returns `NativeHostUninstallReport` and removes only the selected browser target's manifest.
    - Chrome compatibility wrappers remain available as deprecated helpers:
      `install_rzn_native_host_for_chrome(...)` / `uninstall_rzn_native_host_for_chrome()`
    - `resolve_native_host_executable_path()` / `install_rzn_native_host_for_chrome_auto(...)` for dev-friendly setup.
    - `read_installed_rzn_native_host_manifest_for_browser()` for diagnostics.
  - Substrate:
    - `rzn_sdk::Session::{connect, snapshot, apply, close}`
    - `rzn_sdk::BrowserTools::{observe, act, execute_steps, get_page_source, close}`
- Error handling & retries
  - Stable host errors return `rzn_sdk::host::HostError` (wrapped by `rzn_sdk::Error`).
  - Broker install utilities return structured errors for common setup problems (missing broker binary, empty extension id, unsupported OS for auto-install).
  - Tool surface returns `ToolError` for stable downstream handling (timeout/transport/extension_error/target_not_found).

## Tasks & Status
- [x] Add new `rzn_sdk` crate to the Rust workspace
- [x] Provide `host` feature API (Orchestrator wrapper)
- [x] Provide `broker` feature API (native host manifest helpers)
- [x] Add versioned contracts crate (`crates/rzn_contracts`)
- [x] Add deterministic substrate (`Session`, `BrowserTools`)
- [x] Add a downstream example (`cargo run -p rzn_sdk --example desktop_bridge_sdk -- "<goal>" ["<start_url>"]`)
- [ ] Decide on official bundling strategy (Tauri sidecar + installer hooks vs manual install)

## What Works (Do Not Change)
- The broker native host name must remain aligned with the extension host candidates. The canonical id is `com.rzn.browser.broker`; keep `com.rzn.browser.broker` only as a compatibility fallback.
- The broker/host IPC transport defaults to `pipe` and is expected to remain compatible across CLI and embedded host use.
- Avoid introducing domain-tuned logic (selectors/rules) into SDK surfaces; keep targeting generic and inventory-driven.
 - Do not bake prompts or LLM provider logic into `BrowserTools`; host apps own reasoning.

## Tried & Didn’t Work
- Shipping the broker “inside” a Rust library: native messaging requires Chrome to launch an executable host over stdio, so an in-process broker is not sufficient.
- Platform-unified auto-install on Windows: Chrome uses registry-based registration; support needs a dedicated implementation (and likely installer integration) rather than simple file writes.
