---
schema: tusker.knowledge/v6
title: Validator and failure modes
node: schema/validator
audience: developer
agent_layer: capsule
kind: reference
domains:
- schema
source_of_truth:
- tusker/_config/knowledge-policy.yaml
- tusker/SKILL.md
- crates/rzn_contracts/src/v2.rs
canonical_status: draft
created: '2026-05-08'
updated: '2026-05-13'
domain: schema
stale_when:
  paths:
  - tusker/_config/knowledge-policy.yaml
  - tusker/SKILL.md
  - crates/rzn_contracts/src/v2.rs
publish:
  lane: internal
  path: schema/validator
  include_in_llms: true
summary: Validation responsibilities and common failure modes for Tusker and workflow contracts.
---

# Validator and failure modes

## Read this when

Read this when interpreting validation failures or deciding which validator applies.

## Do not read this when

Do not use this as proof that validation passed. Check the task verification log.

## Validator lanes

| Validator | Checks | Does not prove |
|---|---|---|
| `tusker validate` | Vault shape, required sections, frontmatter/link consistency, docs impact state. | That documentation is useful or source-backed. |
| `rzn-browser workflow validate <path> --strict --json` | Manifest schema, params, effects, steps, selector references. | That the live website flow works. |
| `rzn-browser workflow validate-catalog --strict --json` | Production catalog health. | That each workflow has fresh smoke proof. |
| Rust/extension tests | Code behavior in covered paths. | Manual browser-profile state or third-party website stability. |

If the validator CLI is unavailable, record that as blocked. Do not mark validation complete from vibes.

## Source of truth

- `tusker/_config/knowledge-policy.yaml`
- `tusker/SKILL.md`
- `crates/rzn_contracts/src/v2.rs`

## Related

- [[schema/CANON]]
- [[workflow/CANON]]

## Recent changes

<!-- tusker:backrefs:begin -->
- [[OPS-T-0002]] - Replaced generated scaffold with validator reference.
<!-- tusker:backrefs:end -->
