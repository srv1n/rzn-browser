---
schema: tusker.epic/v6
id: WCP
title: Workflow Contract Platform
status: draft
owner: sarav
summary: 'Define and enforce the core workflow/capability ABI before public launch:
  typed manifests, stable run envelopes, capability routing, side-effect policy, catalog
  lifecycle, contract tests, and downstream workflow-team enablement.'
created: '2026-05-09'
updated: '2026-05-09'
primary_domains:
- workflow
knowledge_nodes:
- workflow/canon
created_at: '2026-05-09'
updated_at: '2026-05-09'
---

# WCP · Workflow Contract Platform

## Thesis

RZN needs a workflow contract platform before public launch. The public API should be capability-first and manifest-declared; workflows are implementation details. Core owns typed manifests, result envelopes, side-effect policy, catalog lifecycle, strict validation, and runtime normalization. Workflow-pack teams own system-specific JSON and live-site proof after those primitives exist.

## Scope

In:
- `rzn_contracts::v2` workflow, capability, action, result, artifact, warning, and runtime capability ABI.
- Manifest-declared catalog records and capability routing.
- Typed parameter validation and typed placeholder substitution.
- Supervisor-owned run orchestration and normalized run results.
- Side-effect classification, approval policy, and artifact/download references.
- Fixture-based contract tests and downstream workflow-team authoring gates.

Out:
- Preserving legacy workflow names, aliases, or inferred catalog behavior.
- Designing a universal assistant transcript envelope in core.
- Rewriting every workflow pack before the strict validator and fixture harness exist.

## Success metrics

- Public execution can route by manifest-declared capability id with explicit system selection and typed params.
- Every workflow run returns a stable envelope with status, final output, warnings, artifacts, debug metadata, and effect metadata.
- `catalog validate --strict` blocks malformed manifests, undeclared params, missing output contracts, unsafe side effects, and tombstone collisions.
- Local fixture workflows prove the ABI without live third-party sites.
- Downstream workflow teams have a single contract document and acceptance gate.

## Canon

- `docs/features/workflow_contract_platform/README.md`
- `crates/rzn_contracts/src/v1.rs`
- `crates/rzn_core/src/lib.rs`
- `crates/rzn_browser/src/workflow_catalog.rs`
- `crates/rzn_browser/src/native_runner.rs`
- `crates/rzn_browser/src/supervisor.rs`
- `docs/features/workflow_engine_improvements/README.md`

## Task stack

_Open tasks only. Closed/cancelled work is intentionally omitted; use `tusker list --epic WCP --type task --status done` for closed history._

- [[WCP-T-0024]] — Browser-engine eval bugs surfaced by chatgpt+contracts smokes (active, p1, medium)
- [[WCP-T-0011]] — Create workflow contract platform scratchpad and ABI decision record (review, p0, medium)
- [[WCP-T-0023]] — Make manifest canonical workflow contract and inspection surface (review, p0, medium)
- [[WCP-T-0025]] — Make strict catalog validation a real rollout gate (review, p0, high)
- [[WCP-T-0026]] — Make manifest steps the native runner source (review, p0, high)
- [[WCP-T-0028]] — Enforce declared side effects end to end (review, p0, high)
- [[WCP-T-0029]] — Make RunResultV2 the only host-visible run envelope (review, p0, high)
- [[WCP-T-0007]] — Add fixture-based workflow contract test harness (review, p1, medium)
- [[WCP-T-0008]] — Create downstream workflow-team authoring guide and acceptance gate (review, p1, medium)
- [[WCP-T-0020]] — Pilot ManifestV2 with two local fixture workflows (review, p1, medium)
- [[WCP-T-0031]] — Publish workflow authoring gate for downstream agents (review, p1, medium)
- [[WCP-T-0001]] — Define WorkflowManifestV2 and RunEnvelopeV1 contracts (ready, p0, high)
- [[WCP-T-0003]] — Add capability registry and explicit system routing (ready, p0, high)
- [[WCP-T-0004]] — Replace workflow parameter prose with typed input schema validation (ready, p0, high)
- [[WCP-T-0030]] — Certify migrated workflow pack against manifest standard (ready, p0, high)
- [[WCP-T-0002]] — Enforce manifest-declared side-effect policy at runtime (backlog, p0, high)
- [[WCP-T-0005]] — Standardize workflow result selection, artifacts, warnings, and debug output (backlog, p0, high)
- [[WCP-T-0012]] — Add extension-wide typed action result normalizer (backlog, p0, high)
- [[WCP-T-0013]] — Move workflow run orchestration into the supervisor (backlog, p0, high)
- [[WCP-T-0017]] — Remove implicit success and final-output heuristics from CLI paths (backlog, p0, high)
- [[WCP-T-0018]] — Replace generated StepKind with compact ActionV2 and StepV2 (backlog, p0, high)
- [[WCP-T-0019]] — Upgrade workflow validation commands to strict contract mode (backlog, p0, high)
- [[WCP-T-0006]] — Implement catalog manifest lifecycle and dev auto-reconciliation (backlog, p1, medium)
- [[WCP-T-0009]] — Convert reference workflow packs to ManifestV2 capability contracts (backlog, p1, high)
- [[WCP-T-0014]] — Make native host a typed extension bridge only (backlog, p1, high)
- [[WCP-T-0015]] — Move cloud actor ownership and result replay into supervisor (backlog, p1, high)
- [[WCP-T-0016]] — Update MCP browser adapter to emit structured run envelopes (backlog, p1, medium)
- [[WCP-T-0010]] — Design large-output and download artifact primitives (backlog, p2, medium)

## Open questions

- Should cloud execution be in the launch ABI? If yes, `CloudBrowserCommandV1` payloads need typed v2 replacements now.
- How aggressive should `execute_javascript` policy be? Current recommendation: treat it as code execution and require explicit manifest allowance.
- Which capability id namespace is final for launch: product-style (`assistant.read`) or system/resource-style (`assistant.conversation.read`)?
