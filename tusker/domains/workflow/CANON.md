---
schema: "tusker.knowledge/v6"
node: "workflow/canon"
title: "Workflow canon"
domain: "workflow"
kind: "canon"
audience: "developer"
agent_layer: "capsule"
canonical_status: "draft"
summary: "Workflow contract platform canon: manifest-shaped workflow files, capability routing, side effects, validation, smoke proof, and agent authoring rules."
aliases:
  - "workflow canon"
  - "workflow"
  - "workflow manifest"
source_of_truth:
  - "crates/rzn_contracts/src/v2.rs"
  - "crates/rzn_browser/src/workflow_catalog.rs"
  - "crates/rzn_browser/src/native_runner.rs"
  - "crates/rzn_browser/src/main.rs"
  - "docs/features/workflow_contract_platform/README.md"
  - "docs/workflows/AGENT_PLAYBOOK.md"
  - "docs/workflows/workflow-manifest-authoring.md"
  - "AGENTS.md"
  - "workflows/**"
stale_when:
  paths:
    - "crates/rzn_contracts/src/v2.rs"
    - "crates/rzn_browser/src/workflow_catalog.rs"
    - "crates/rzn_browser/src/native_runner.rs"
    - "crates/rzn_browser/src/main.rs"
    - "docs/features/workflow_contract_platform/**"
    - "docs/workflows/**"
    - "workflows/**"
    - "AGENTS.md"
publish:
  include_in_llms: true
  lane: "internal"
  path: "workflow/canon"
created_at: "2026-05-12"
updated_at: "2026-05-13"
tags:
  - "workflow"
---

# Workflow canon

## Read this when

Read this before creating, migrating, validating, listing, inspecting, routing, or smoke-testing workflows under `workflows/<system>/*.json`.

## Do not read this when

Do not use this for low-level browser bridge behavior; read [[runtime/CANON]]. Do not use it as a replacement for live DOM inspection when writing a site workflow; this page defines the contract, not the current third-party website.

## Current model

Production workflows are normal JSON files in `workflows/<system>/<name>.json` whose body is a manifest. The manifest is the contract. Do not create production `*.manifest.json`, `*.manifest.v2.json`, or loose sidecar files.

The canonical schema version is:

```json
"schema_version": "rzn.workflow_manifest"
```

The public surface is capability/route oriented, but implementation remains a single production workflow file. The runtime and catalog now understand manifest-shaped files with typed params, declared effects, executable steps, output contracts, and help text.

## Contract shape

`WorkflowManifestV2` in `crates/rzn_contracts/src/v2.rs` is the source for the manifest ABI:

| Field | Required meaning |
|---|---|
| `schema_version` | Must equal `rzn.workflow_manifest`. |
| `id`, `name`, `version`, `system`, `capability` | Stable identity and route/capability metadata. |
| `params.properties` | Typed input schema. Use `string`, `integer`, `number`, `boolean`, `array`, or `object` honestly. |
| `params.additional_params` | Default false; unknown params are not accepted unless explicitly allowed. |
| `side_effects` | Top-level effect budget for the workflow. |
| `runtime` | Actor, timeout, CDP/session requirements, and migration-only workflow refs. |
| `steps[]` | Executable actions. Step ids must be unique. Step effects must be declared at top level. |
| `result.output_selector` | Points to the step/path used as final output. |
| `result.output_schema` | Declares caller-facing output shape. |
| `help` | Human/agent callable summary, parameters, examples, returns, and notes. |

`RunResultV2` is the host-visible result envelope. CLI, MCP, and downstream hosts should consume `RunResultV2.output`, not infer success or final output from legacy payload shape.

## Side-effect taxonomy

Unknown effect names are validation errors. Declare every effect the runtime or user must care about:

| Effect | Meaning |
|---|---|
| `read_only` | Reads page or browser-visible content without mutation. |
| `external_read` | Reads data from a remote origin or third-party URL. |
| `network_access` | Performs outbound network access outside local browser state. |
| `browser_state` | Changes tab, DOM state, navigation, focus, or session-local browser state. |
| `file_write` | Writes local files. |
| `download` | Starts or records browser downloads. |
| `external_write` | Writes to a remote service or user account. |
| `auth` | Uses or changes authentication/session state. |
| `destructive` | Deletes, posts irreversible changes, or performs high-risk mutation. |

CLI post-processing is part of side-effect review too: `--output-file` requires file-write policy; `--download-dir` requires download/file/network/external-read policy.

## Authoring rules

- Probe the live DOM/API before choosing selectors or parsers.
- Check existing workflows for overlap. Split only when caller-visible output, side effects, or runtime behavior truly differ.
- Use enums instead of booleans when more than two modes are plausible.
- Do not ship speculative debug/inspection/download-only tools unless a real caller needs them.
- Prefer `data-testid`, ARIA roles/labels, and stable structure over utility class names.
- Do not make shipped workflows depend on active-tab state unless the workflow genuinely continues an already-open session and declares `runtime.requires_existing_session`.
- Use `array` only for real lists. Scalar ids, URLs, mode flags, handles, and filters are usually `string`.
- Keep workflow filenames stable. Version the JSON body, not the path.

## Validation and handoff gate

A workflow is not ready just because JSON parses. The minimum gate is:

| Gate | Command/proof |
|---|---|
| Per-file structure | `rzn-browser workflow validate <path-or-ref> --strict --json` returns zero errors and warnings. |
| Inspectability | `rzn-browser workflow inspect <system> <workflow> --json` exposes inputs, optionality, effects, runtime, steps count, output selector, and schema. |
| Catalog health | `rzn-browser workflow validate-catalog --strict --json` passes for production catalog entries. |
| Runtime proof | `rzn-browser run <system> <workflow> --param ...` succeeds or stops at a documented approval gate. |
| Installed-copy proof | If testing installed workflows, run `rzn-browser workflow pull --repo-root .` after editing the repo copy. |
| Mutation policy | Post/send/delete flows stop before irreversible action unless the operator explicitly approves the write. |

Structural validation proves contract shape, not live usefulness. Smoke evidence is mandatory for handoff.

## Current workflow catalog behavior

`crates/rzn_browser/src/workflow_catalog.rs` owns workflow roots and catalog inspection:

- Runtime root: `RZN_RUNTIME_DIR` or platform local data dir plus `RZN`.
- Built-in workflow dir: `RZN_BUILTIN_WORKFLOWS_DIR` or `<runtime>/workflows/builtin`.
- User workflow dir: `RZN_WORKFLOWS_DIR` or `<runtime>/workflows/user`.
- Legacy user workflows may still be discovered under `~/.rzn/workflows`.
- User entries can shadow built-in entries; catalog listing exposes source/shadowing.
- Capability records include manifest path, workflow path, manifest version, content hash, description, and effects.

## Common workflow traps

- Optional JS params arrive as literal placeholders if the engine does not substitute them. Use a `cleanArg` helper in workflow scripts.
- `execute_javascript` is JS-first; do not force CDP unless a trusted browser gesture is required.
- One trusted gesture per CDP eval. Split downloads, popups, clipboard, and file interactions.
- SPA navigation should fire-and-forget inside eval and return before the execution context is destroyed.
- ProseMirror/contenteditable editors need editor-aware input, often `document.execCommand('insertText', false, text)`.
- File inputs cannot be set from page JS; use the existing upload action.
- Some controlled Radix/React widgets ignore synthetic clicks. Use CDP mouse input only for the specific trusted gesture and verify afterward.

## Invariants

- The production workflow file is the manifest.
- Manifest steps are the runtime source when present.
- Help text is required, but machine contract comes from schema fields.
- Runtime and workflow policy must be honest about side effects.
- Workflow teams own system-specific JSON and live-site proof; core owns manifest ABI, validation, runner behavior, and result envelopes.

## Current defaults

- Production workflows should be manifest-shaped JSON in `workflows/<system>/<name>.json`.
- Catalog and run paths should consume manifest identity, params, side effects, steps, result selectors, and help text.
- Validate narrowly first, then prove live behavior with the real extension/native-host path when the workflow touches a site.

## Deprecated behavior

- Loose legacy workflow JSON is migration-only for production work.
- Manifest sidecars are not allowed in production workflow packs.
- Inferring final output from the last step is not acceptable when `result.output_selector` can express it.
- Active-tab production workflows are disfavored because they create hidden state coupling.

## Source of truth

- ABI: `crates/rzn_contracts/src/v2.rs`
- Catalog/routing: `crates/rzn_browser/src/workflow_catalog.rs`
- Runtime execution: `crates/rzn_browser/src/native_runner.rs`
- CLI commands: `crates/rzn_browser/src/main.rs`
- Platform standard: `docs/features/workflow_contract_platform/README.md`
- Agent authoring traps: `docs/workflows/AGENT_PLAYBOOK.md`
- Public authoring gate: `docs/workflows/workflow-manifest-authoring.md`

## Open questions

- Whether cloud execution is launch-scope in the workflow ABI.
- Final capability id namespace for launch: compact product names versus system/resource/action names.
- How quickly to remove migration fields such as `runtime.workflow_ref` and `runtime.workflow_path` once executable manifest steps are complete.

## Related

- [[workflow/INDEX]]
- [[runtime/CANON]]
- [[schema/CANON]]

## Recent changes

<!-- tusker:backrefs:begin -->
- [[OPS-T-0002]] touched this knowledge node.
<!-- tusker:backrefs:end -->
