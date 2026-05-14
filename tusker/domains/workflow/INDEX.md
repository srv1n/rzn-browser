---
schema: "tusker.domain/v6"
id: "workflow"
title: "Workflow"
status: "current"
owner: "sarav"
summary: "Workflow contract platform, manifest-shaped workflow files, capability routing, validation, side effects, and smoke proof."
required: false
knowledge_nodes:
  - "workflow/canon"
source_of_truth:
  - "crates/rzn_contracts/src/v2.rs"
  - "crates/rzn_browser/src/workflow_catalog.rs"
  - "docs/features/workflow_contract_platform/README.md"
  - "docs/workflows/AGENT_PLAYBOOK.md"
  - "docs/workflows/workflow-manifest-authoring.md"
tags:
  - "workflow"
---

# Workflow

## Read this when

Read this when work touches workflow JSON, manifest contracts, capability routes, side-effect declarations, validation, inspect output, or workflow smoke evidence.

## Do not read this when

Do not read this for unrelated domains or task proof history unless this index routes you there.

## Current canon

- [[workflow/CANON]]

## Start here

Read [[workflow/CANON]] first. Then read `docs/workflows/AGENT_PLAYBOOK.md` before editing workflow files.

## Main knowledge nodes

- [[workflow/CANON]]

## Source of truth

- `crates/rzn_contracts/src/v2.rs`
- `crates/rzn_browser/src/workflow_catalog.rs`
- `docs/features/workflow_contract_platform/README.md`
- `docs/workflows/AGENT_PLAYBOOK.md`
- `docs/workflows/workflow-manifest-authoring.md`

## Related domains

- [[codebase/INDEX]]

## Current work

<!-- tusker:current-work:begin -->
- [[WAT-T-0002]] - ChatGPT picker: testid-first + CDP commit + verify (draft)
- [[WCP-T-0001]] - Define WorkflowManifestV2 and RunEnvelopeV1 contracts (ready)
- [[WCP-T-0002]] - Enforce manifest-declared side-effect policy at runtime (backlog)
- [[WCP-T-0003]] - Add capability registry and explicit system routing (ready)
- [[WCP-T-0004]] - Replace workflow parameter prose with typed input schema validation (ready)
- [[WCP-T-0005]] - Standardize workflow result selection, artifacts, warnings, and debug output (backlog)
- [[WCP-T-0006]] - Implement catalog manifest lifecycle and dev auto-reconciliation (backlog)
- [[WCP-T-0007]] - Add fixture-based workflow contract test harness (review)
- [[WCP-T-0008]] - Create downstream workflow-team authoring guide and acceptance gate (review)
- [[WCP-T-0009]] - Convert reference workflow packs to ManifestV2 capability contracts (backlog)
- [[WCP-T-0010]] - Design large-output and download artifact primitives (backlog)
- [[WCP-T-0011]] - Create workflow contract platform scratchpad and ABI decision record (review)
- [[WCP-T-0012]] - Add extension-wide typed action result normalizer (backlog)
- [[WCP-T-0013]] - Move workflow run orchestration into the supervisor (backlog)
- [[WCP-T-0014]] - Make native host a typed extension bridge only (backlog)
- [[WCP-T-0015]] - Move cloud actor ownership and result replay into supervisor (backlog)
- [[WCP-T-0016]] - Update MCP browser adapter to emit structured run envelopes (backlog)
- [[WCP-T-0017]] - Remove implicit success and final-output heuristics from CLI paths (backlog)
- [[WCP-T-0018]] - Replace generated StepKind with compact ActionV2 and StepV2 (backlog)
- [[WCP-T-0019]] - Upgrade workflow validation commands to strict contract mode (backlog)
- [[WCP-T-0020]] - Pilot ManifestV2 with two local fixture workflows (review)
- [[WCP-T-0023]] - Make manifest canonical workflow contract and inspection surface (review)
- [[WCP-T-0024]] - Browser-engine eval bugs surfaced by chatgpt+contracts smokes (active)
- [[WCP-T-0025]] - Make strict catalog validation a real rollout gate (review)
- [[WCP-T-0026]] - Make manifest steps the native runner source (review)
- [[WCP-T-0030]] - Certify migrated workflow pack against manifest standard (ready)
- [[WCP-T-0031]] - Publish workflow authoring gate for downstream agents (review)
<!-- tusker:current-work:end -->
