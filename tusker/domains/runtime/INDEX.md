---
schema: "tusker.domain/v6"
id: "runtime"
title: "Runtime"
status: "current"
owner: "sarav"
summary: "Current local browser runtime model: supervisor, native host, extension, legacy worker fallback, sessions, heal, and logs."
required: false
knowledge_nodes:
  - "runtime/canon"
source_of_truth:
  - "crates/rzn_browser/src/supervisor.rs"
  - "crates/rzn_browser/src/native_runner.rs"
  - "crates/rzn_native_host/src/main.rs"
  - "extension/src/background.ts"
  - "docs/features/local_supervisor_runtime/README.md"
tags:
  - "runtime"
---

# Runtime

## Read this when

Read this when work touches supervisor behavior, native-host bridging, extension execution, browser sessions, heal/status paths, or legacy worker migration.

## Do not read this when

Do not read this for unrelated domains or task proof history unless this index routes you there.

## Current canon

- [[runtime/CANON]]

## Start here

Read [[runtime/CANON]] first. Then open source files for exact behavior or LRT tasks for implementation proof.

## Main knowledge nodes

- [[runtime/CANON]]
- [[runtime/runtime]]

## Source of truth

- `crates/rzn_browser/src/supervisor.rs`
- `crates/rzn_browser/src/native_runner.rs`
- `crates/rzn_native_host/src/main.rs`
- `extension/src/background.ts`
- `docs/features/local_supervisor_runtime/README.md`

## Related domains

- [[codebase/INDEX]]

## Current work

<!-- tusker:current-work:begin -->
- [[OPS-T-0002]] - Replace generated V6 domain canon with repo-specific documentation (review)
- [[WCP-T-0023]] - Make manifest canonical workflow contract and inspection surface (review)
- [[WCP-T-0026]] - Make manifest steps the native runner source (review)
- [[WCP-T-0028]] - Enforce declared side effects end to end (review)
- [[WCP-T-0029]] - Make RunResultV2 the only host-visible run envelope (review)
- [[WCP-T-0030]] - Certify migrated workflow pack against manifest standard (ready)
<!-- tusker:current-work:end -->
