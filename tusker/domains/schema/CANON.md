---
schema: "tusker.knowledge/v6"
node: "schema/canon"
title: "Schema canon"
domain: "schema"
kind: "canon"
audience: "developer"
agent_layer: "capsule"
canonical_status: "draft"
summary: "Schema canon for Tusker V6 notes, workflow manifest v2, run result envelopes, action schemas, validation, templates, and generated-file boundaries."
aliases:
  - "schema canon"
  - "schema"
source_of_truth:
  - "tusker/_config/knowledge-policy.yaml"
  - "tusker/_system/templates/**"
  - "crates/rzn_contracts/src/v2.rs"
  - "crates/rzn_core/build.rs"
  - "schema/**"
  - "docs/features/workflow_contract_platform/README.md"
  - "docs/workflows/workflow-manifest-authoring.md"
stale_when:
  paths:
    - "tusker/_config/knowledge-policy.yaml"
    - "tusker/_system/templates/**"
    - "tusker/SKILL.md"
    - "crates/rzn_contracts/src/**"
    - "crates/rzn_core/build.rs"
    - "schema/**"
    - "docs/features/workflow_contract_platform/**"
    - "docs/workflows/workflow-manifest-authoring.md"
publish:
  include_in_llms: true
  lane: "internal"
  path: "schema/canon"
created_at: "2026-05-12"
updated_at: "2026-05-13"
tags:
  - "schema"
---

# Schema canon

## Read this when

Read this before changing Tusker frontmatter/templates, workflow manifest contracts, run result envelopes, action schemas, generated type inputs, or validation policy.

## Do not read this when

Do not use this page as a substitute for the actual Rust structs or JSON schemas. For exact fields, open `crates/rzn_contracts/src/v2.rs` or the relevant file under `schema/`.

## Current model

There are two schema systems that matter day to day:

| Schema lane | Authority | Used for |
|---|---|---|
| Tusker V6 vault schema | `tusker/_config/knowledge-policy.yaml`, `tusker/_system/templates/**` | Domain indexes, canon pages, tasks, epics, docs impact, validation expectations. |
| RZN workflow/runtime ABI | `crates/rzn_contracts/src/v2.rs`, `schema/**`, `crates/rzn_core/build.rs` | Workflow manifests, action/result contracts, run envelopes, generated runtime types. |

Keep these lanes separate. Tusker schemas document and govern repo knowledge. RZN contract schemas govern product runtime behavior.

## Tusker V6 knowledge schema

Domain folders require:

- `INDEX.md`
- `CANON.md`

Knowledge pages must preserve these reader sections:

- `Read this when`
- `Do not read this when`
- `Source of truth`
- `Related`

Domain indexes must preserve:

- `Read this when`
- `Do not read this when`
- `Current canon`
- `Start here`
- `Main knowledge nodes`
- `Source of truth`
- `Related domains`
- `Current work`

Allowed knowledge kinds include `canon`, `index`, `architecture`, `reference`, `how-to`, `troubleshooting`, `decision`, `glossary`, `runbook`, `asset`, `feature`, `support`, and `release`.

Allowed audiences are `user`, `developer`, `operator`, `support`, `release`, `agent`, and `internal`. Allowed agent layers are `none`, `capsule`, and `standalone`.

## Tusker authoring rules

- Author durable truth in `tusker/domains/**`.
- Keep task proof in `tusker/epics/**`.
- Do not edit `tusker/_system/generated/**`; those are derived indexes.
- Do not treat generated validation success as reader usefulness.
- If code and canon disagree, trust code, update canon, and record a task/knowledge delta.
- Use `source_of_truth` and `stale_when.paths` to make future freshness checks meaningful.

## Invariants

- Source structs, JSON schemas, and templates are the exact contract; prose explains them but does not override them.
- Generated output is never the hand-authored source of truth.
- Tusker schema changes and product runtime schema changes are separate lanes and should name their affected consumers.

## Current defaults

- Use Tusker V6 frontmatter and section names from `tusker/_system/templates/**`.
- Treat `rzn.workflow_manifest` and `rzn.run_result.v2` as the current workflow/runtime ABI.
- Prefer strict validation and `deny_unknown_fields` for launch-facing contracts.

## Deprecated behavior

- Loose generated scaffolds are not acceptable canon after source-backed migration.
- Legacy workflow JSON without manifest structure is migration-only for production workflows.
- Schema sidecars should not replace the canonical production workflow file.

## Workflow manifest schema

`WorkflowManifestV2` is strict Rust `serde` data with `deny_unknown_fields` on major structs. Validation enforces:

- `schema_version == "rzn.workflow_manifest"`
- non-empty identity fields
- valid params/defaults
- valid runtime references
- unique step ids
- custom actions declare `custom_kind`
- step side effects are included in top-level `side_effects`
- result selectors reference known step ids when steps are present

The manifest runtime actor enum is:

- `extension`
- `supervisor`
- `cloud`

The run result version constant is `rzn.run_result.v2`; run envelope constant is `rzn.run_envelope.v1`.

## Parameter schema

Manifest params live under `params.properties` and normalize/coerce caller input. Use the narrowest honest type:

| Kind | Use for |
|---|---|
| `string` | text, ids, URLs, mode values, handles, scalar filters |
| `integer` | whole-number counts/limits |
| `number` | non-integer numeric values |
| `boolean` | true binary toggles |
| `array` | real lists only |
| `object` | structured JSON bags |

CLI array convenience is not schema permission to mark scalars as arrays.

## Runtime/action schemas

`schema/actions-v1.json`, `schema/rzn-actions-complete.json`, `schema/extraction-plan-v1.json`, and related schemas define action surfaces consumed by runtime and generated code. `crates/rzn_core/build.rs` is the build-time bridge for schema-derived code.

When action schema changes:

- Check generated/compiled Rust consumers in `crates/rzn_core`.
- Check extension action handling in `extension/src/**`.
- Check workflow manifests that use the affected action kind.
- Add or update contract tests where possible.

## Generated output boundaries

Do not author source truth in:

- `tusker/_system/generated/**`
- docs-site generated output if present under `site/src/content/docs/**`
- extension build output such as `extension/dist-chrome/**` unless the task is packaging/release verification

Author in schema/source/template files, then regenerate with the repo toolchain when the relevant command exists.

## Source of truth

- Tusker policy: `tusker/_config/knowledge-policy.yaml`
- Tusker templates: `tusker/_system/templates/**`
- Workflow ABI: `crates/rzn_contracts/src/v2.rs`
- Runtime action schemas: `schema/**`
- Schema build bridge: `crates/rzn_core/build.rs`
- Workflow manifest explanation: `docs/features/workflow_contract_platform/README.md`

## Open questions

- Whether Tusker V6 should get a repo-local validator script committed here while the global `tusker` CLI is unavailable.
- When to remove legacy workflow schema fields once all production manifests execute from `steps[]`.
- Which generated schemas should be published for external workflow authors.

## Related

- [[schema/INDEX]]
- [[workflow/CANON]]
- [[codebase/CANON]]

## Recent changes

<!-- tusker:backrefs:begin -->
- [[OPS-T-0002]] - Replaced generated scaffold with source-backed schema canon.
<!-- tusker:backrefs:end -->
