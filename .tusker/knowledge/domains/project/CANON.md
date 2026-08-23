---
schema: "tusker.domain-canon/v7"
kind: "domain_canon"
id: "project/canon"
project: "rzn-browser"
domain: "project"
title: "Project Canon"
status: "current"
summary: "Current durable truth for Project."
capsule:
  skip_when: "Skip when you only need task proof, runtime events, or generated packets."
  use_when: "Use before changing behavior owned by project or reviewing a domain-impacting task."
  what: "Current durable truth, invariants, and constraints for Project."
source_of_truth:
  - "knowledge/domains/project/CANON.md"
created_at: "2026-08-23T10:56:19Z"
updated_at: "2026-08-23T12:29:13Z"
state_rev: "sha256:d18f98188c99dbe5b1611c16d8e9a6f2fbc2944c6a0d63f518bce08a804f0540"
---

# Project Canon

## Current Truth

- RZN Browser runs repeatable browser work on a local computer.
- The Rust CLI starts work. The native host passes messages to the Chrome
  extension. The extension reads or changes the page and returns a result.
- JSON workflows live under `workflows/<system>/`.
- `llm-auto` plans a task when a fixed workflow is not enough.
- The current project tracker is the local `.tusker/` vault. It has no old
  task or epic records.

## Stable Interfaces

- The CLI command surface in `crates/rzn_browser/src/main.rs`.
- The wire data in `crates/rzn_contracts/`.
- The Chrome native messaging host in `crates/rzn_native_host/`.
- The workflow JSON contract in `workflows/README.md`.
- The Makefile build and test targets.

## Constraints

- Use `make` for project builds, checks, tests, and runs.
- Keep site selectors in workflow files. Keep shared code site-neutral.
- Use the DOM action path first, then the input ladder, then CDP when needed.
- Do not put credentials, cookies, page content, or tokens in Git.
- Treat a focused check as proof of one surface. Do not claim release or
  runtime proof from a source-only check.
- Keep this canon short enough to read before implementation.

## Deprecated Or Stale

- _None known._

## Open Questions

- _None known._
