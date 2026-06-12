---
schema: "tusker.knowledge/v6"
node: "codebase/repo-map"
title: "Repository map"
domain: "codebase"
kind: "reference"
audience: "developer"
agent_layer: "capsule"
canonical_status: "draft"
summary: "Quick map of rzn-browser repository modules and source ownership."
aliases:
  - "repository map"
source_of_truth:
  - "README.md"
  - "Cargo.toml"
  - "crates/**/Cargo.toml"
  - "extension/package.json"
stale_when:
  paths:
    - "README.md"
    - "Cargo.toml"
    - "crates/**/Cargo.toml"
    - "extension/package.json"
publish:
  include_in_llms: true
  lane: "internal"
  path: "codebase/repo-map"
created_at: "2026-05-12"
updated_at: "2026-05-13"
---

# Repository map

## Read this when

Read this when choosing where to inspect or place code.

## Do not read this when

Do not use this for exact behavior; open the source file or [[codebase/CANON]] for broader change rules.

## Map

| Path | Owns |
|---|---|
| `crates/rzn_browser` | CLI, workflow catalog, supervisor/native runner, MCP browser adapter, skill installer, cloud/reporting commands. |
| `crates/rzn_contracts` | Versioned contracts for workflows, actions, effects, run results, and runtime matrices. |
| `crates/rzn_core` | Core DSL/executor/shared schema bridge. |
| `crates/rzn_plan` | LLM planning, action surface helpers, extraction/observe/act support. |
| `crates/rzn_sdk` | Host-facing SDK facade. |
| `crates/rzn_native_host` | Chrome native messaging host. |
| `extension/src` | Extension background, content script, CDP, action implementations, tab/runtime helpers. |
| `workflows` | Production workflow packs. |
| `schema` | JSON schemas and generated-type inputs. |
| `skills` | Agent skill packages. |
| `docs/features` | Feature scratchpads and architecture records. |
| `tusker` | Repo-local task/evidence/knowledge graph. |

## Source of truth

- `README.md`
- `Cargo.toml`
- `crates/**/Cargo.toml`
- `extension/package.json`

## Related

- [[codebase/CANON]]

## Recent changes

<!-- tusker:backrefs:begin -->
_No task proof recorded yet._
<!-- tusker:backrefs:end -->
