---
schema: "tusker.domain/v6"
id: "codebase"
title: "Codebase"
status: "current"
owner: "sarav"
summary: "Repository layout, implementation anchors, testing, source authority, and safe change rules."
required: true
knowledge_nodes:
  - "codebase/canon"
source_of_truth:
  - "AGENTS.md"
  - "README.md"
  - "Cargo.toml"
  - "crates/**/Cargo.toml"
  - "extension/package.json"
tags:
  - "codebase"
---

# Codebase

## Read this when

Read this when work touches repo layout, crate boundaries, broad implementation choices, verification strategy, or safe-change policy.

## Do not read this when

Do not read this for unrelated domains or task proof history unless this index routes you there.

## Current canon

- [[codebase/CANON]]

## Start here

Read [[codebase/CANON]] first, then source files or feature scratchpads for exact implementation detail.

## Main knowledge nodes

- [[codebase/CANON]]
- [[codebase/repo-map]]
- [[codebase/testing]]
- [[codebase/safe-change-rules]]

## Source of truth

- `AGENTS.md`
- `README.md`
- `Cargo.toml`
- `crates/**/Cargo.toml`
- `extension/package.json`

## Related domains

- [[codebase/INDEX]]

## Current work

<!-- tusker:current-work:begin -->
_No open task currently targets this domain._
<!-- tusker:current-work:end -->
