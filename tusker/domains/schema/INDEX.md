---
schema: "tusker.domain/v6"
id: "schema"
title: "Schema"
status: "current"
owner: "sarav"
summary: "Tusker V6 note schema, workflow manifest v2, run result envelopes, action schemas, templates, validation, and generated boundaries."
required: false
knowledge_nodes:
  - "schema/canon"
source_of_truth:
  - "tusker/_config/knowledge-policy.yaml"
  - "tusker/_system/templates/**"
  - "crates/rzn_contracts/src/v2.rs"
  - "schema/**"
tags:
  - "schema"
---

# Schema

## Read this when

Read this when work touches Tusker frontmatter/templates, workflow manifest ABI, run result envelopes, action JSON schemas, or generated-file boundaries.

## Do not read this when

Do not read this for unrelated domains or task proof history unless this index routes you there.

## Current canon

- [[schema/CANON]]

## Start here

Read [[schema/CANON]] first. Then open the actual Rust struct or JSON schema before changing exact fields.

## Main knowledge nodes

- [[schema/CANON]]
- [[schema/templates]]
- [[schema/validator]]

## Source of truth

- `tusker/_config/knowledge-policy.yaml`
- `tusker/_system/templates/**`
- `crates/rzn_contracts/src/v2.rs`
- `schema/**`

## Related domains

- [[codebase/INDEX]]

## Current work

<!-- tusker:current-work:begin -->
- [[OPS-T-0002]] - Replace generated V6 domain canon with repo-specific documentation (review)
- [[WCP-T-0026]] - Make manifest steps the native runner source (review)
<!-- tusker:current-work:end -->
