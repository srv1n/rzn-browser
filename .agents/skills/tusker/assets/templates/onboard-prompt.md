You are analyzing an existing software repository from a curated Tusker onboarding packet. Produce a conservative onboarding plan for local Tusker review and held import. Do not modify application code.

Repository: {{REPO}}
Packet schema: tusker.onboard_packet/v1

## Operating Rules

1. Use only facts present in the packet. Every fact must name packet evidence; missing facts are assumptions, questions, or source-keyed human gates.
2. Do not output code patches, final Tusker V7 markdown, final IDs, lifecycle fields, paths for committed records, or `state_rev` values.
3. This is an onboarding proposal, not execution authority. It never enables automation, arms work, installs or starts a daemon, authorizes release or spending, or satisfies a gate.
4. Do not emit an executable parallel task-list file. Emit zero or more `tusker.delivery-plan/v2` files only where packet evidence supports exact observable acceptance, verification, artifact, and dependencies.
5. Unknown commands, ownership, credentials, product decisions, and source-of-truth conflicts stay labeled as assumptions, questions, or gates. Never use a placeholder command, `TBD`, “manual verification,” or a human confirmation as fake dispatchable verification.
6. A delivery plan names source keys, not final Tusker identities. Tusker allocates final epic, task, gate, wave, revision, and event identities during import.
7. Existing V1 packets and plans remain readable only through the non-executable legacy compatibility adapter. Its migration report must enumerate every field that cannot be converted without human intent. Legacy work never bypasses V2 doctor, product review, held import, and fingerprint-bound Start.
8. The only execution route is deterministic: doctor → product review → dry-run/held import → exact fingerprint-bound Start. Import allocates IDs and leaves records held/disarmed.
9. Do not include secrets, env values, keys, tokens, private URLs, raw logs, or user data. Keep records compact.
10. For multi-unit work, author a source-keyed V2 DAG. A lone task is a wave of one; dependencies unlock runnable frontiers. Use Tusker CLI lifecycle commands after import, never arbitrary Markdown status edits.

## Input

The attached packet may contain repository tree and file statistics; README and docs summaries; build, test, and CI summaries; manifests; selected source excerpts; public API, route, model, CLI, or test summaries; and skipped-file/redaction reports. Treat skipped files as blind spots and existing docs as evidence rather than guaranteed truth.

## Output

Return a zip or clearly separated files with this structure:

```text
onboarding-plan/
  manifest.json
  report.md
  domains.yaml
  epic-contracts.yaml
  delivery-plans/                 # zero or more ordinary tusker.delivery-plan/v2 files
  gates.yaml
  decisions.yaml
  docs-map.yaml
  assumptions.md
  open-questions.md
  migration-report.md
  legacy-compatibility.yaml       # optional, explicitly non-executable V1 adapter
```

## manifest.json

```json
{
  "schema": "tusker.onboard_plan/v2",
  "repo": "<repo name from packet>",
  "based_on_packet_schema": "tusker.onboard_packet/v1",
  "confidence": "low|medium|high",
  "warnings": [],
  "assumptions": [],
  "open_questions": [],
  "execution_route": "doctor -> product review -> dry-run/held import -> fingerprint-bound Start"
}
```

## report.md

Use these sections: What this repo appears to be; main user/product surfaces; main technical surfaces; build and verification surface; existing documentation quality; highest-risk unknowns; recommended Tusker domain map; proposed epic contracts; delivery-plan candidates; what must not be automated yet.

## domains.yaml

Generate 3-8 domains (fewer for a small repo). Each durable statement cites packet evidence or is labeled as an assumption/question.

```yaml
domains:
  - source_key: project
    title: Project
    summary: Repository-wide workflow, architecture, build, and ownership knowledge.
    evidence: [README.md]
    source_of_truth: [README.md]
    canonical_files: [INDEX.md, CANON.md]
    canon:
      current_truth:
        - statement: <supported fact>
          evidence: [README.md]
      invariants: []
      verification: []
      open_questions: []
```

## epic-contracts.yaml

Propose 3-10 source-keyed epic contracts. They are not final epics and carry no lifecycle state.

```yaml
epic_contracts:
  - source_key: repository-foundation
    acronym_hint: FND
    title: Repository foundation
    domains: [project, build]
    thesis: Make the repo understandable, verifiable, and safe for agent work.
    evidence: [README.md, ci/config.yml]
    success_criteria:
      - outcome: The focused verification command is documented from packet evidence.
        evidence: [ci/config.yml]
```

## delivery-plans/

Emit a plan only when the packet supports every exact proof obligation. Each file is an ordinary `tusker.delivery-plan/v2`, suitable for doctor and review but still inert until held import and Start. Do not emit a plan merely to make a backlog look complete.

```yaml
schema: tusker.delivery-plan/v2
scope: repository-foundation/v1
title: Document packet-confirmed focused verification
spec_refs: [docs/specs/repository-foundation.md]
context_fingerprint: sha256:<planning-context-fingerprint supplied by Tusker>
factory_intake_contract_schema: tusker.factory-intake-contract/v1
factory_intake_contract_version: 1.1.0
factory_intake_contract_fingerprint: sha256:0704d5ee907d738c496512b5ae948e96590a7b732c4ab774bee1de1429b5b13c
summary: Make the packet-confirmed focused verification path reviewable.
epic_contract:
  source_key: repository-foundation
  acronym_hint: FND
  title: Repository foundation
requirements:
  - id: R1
    outcome: Maintainers can run the documented focused verification command.
tasks:
  - source_key: document-focused-verification
    requirement_refs: [R1]
    title: Document focused verification
    outcome: The repository documents the packet-confirmed focused verification command.
    acceptance:
      - id: A1
        outcome: The documented command is the exact command evidenced by CI.
    verification:
      - covers: A1
        check: "command: <exact packet-evidenced command>"
    artifact:
      kind: documentation
      path: <packet-evidenced documentation path>
      summary: Documents the focused verification command and its scope.
      acceptance_ids: [A1]
    dependencies: []
```

`<...>` in this example is explanatory only: substitute it only with packet evidence. If it is unknown, omit the delivery plan and create an assumption, question, discovery proposal, or gate instead.

## gates.yaml

Create source-keyed gates only for a missing human fact, capability, authority, or subjective judgment that blocks a specific proposed delivery. Do not use a gate to paper over unknown machine verification.

```yaml
gates:
  - source_key: production-deployment-owner
    title: Confirm production deployment owner
    gate_kind: decision
    owner: human:<role-or-name-if-evidenced>
    task_source_keys: []
    covers: []
    action: Identify the production deployment approver and authoritative release documentation.
    verification: The human answer is recorded in the governing specification or accepted decision.
    why_agent_cannot: The packet contains no authoritative release ownership.
    evidence: []
```

## decisions.yaml, docs-map.yaml, assumptions.md, and open-questions.md

Keep decisions proposed and evidence-backed. Map docs to source-keyed domains with quality and evidence. List every meaningful assumption with its basis and what would verify it. Prioritize open questions; distinguish blocking questions from nice-to-have questions.

## migration-report.md and legacy-compatibility.yaml

Always write `migration-report.md`. State whether legacy V1 material was present, list each source file read, what was preserved as readable context, and every field that cannot be converted without human intent (including ambiguous epic mapping, final identity, lifecycle status, readiness, ownership, priority/risk, verification/proof, dependencies, gate authority, source conflict, and product decision). Do not silently convert any such field.

If V1 material is present, the optional adapter must say exactly that it is `non_executable`, `read_only`, and requires V2 doctor, product review, held import, and fingerprint-bound Start before any execution. It may preserve source references and unconverted fields, but it must not create work records or execution authority.

## Final Check

Before returning, verify:

- every factual statement cites packet evidence or is labeled as an assumption/question;
- each proposed epic and gate has a source key;
- each emitted V2 plan has source-keyed epic/task/gate contracts, requirement refs, exact observable acceptance, exact verification, artifacts, and dependencies;
- no final Tusker IDs, lifecycle fields, fake verification, credentials, or executable parallel task list appears;
- the migration report enumerates unconverted legacy fields and states the held V2 execution route.
