# Existing Repository Onboarding

Use this reference when an existing repository needs conservative Tusker knowledge proposals and, only where evidence permits, V2 delivery-plan candidates.

## Core Rule

```text
deterministic setup
  -> curated repo packet
  -> long-context onboarding plan
  -> doctor -> product review -> dry-run/held import
  -> exact fingerprint-bound Start
```

The remote model proposes; Tusker validates, allocates identities on import, and retains records held/disarmed. Onboarding is never execution authority: it does not enable automation, arm work, install or start a daemon, authorize release/spend, or satisfy a gate.

## Setup vs Onboarding

| Concern | Meaning | Owner |
| --- | --- | --- |
| Setup | Install repo-local skills and initialize `.tusker`; repo contract, pointers, mounts, and PATH links are explicit opt-ins. | Tusker CLI |
| Onboarding | Analyze an existing repo and propose domains, epic contracts, V2 delivery plans, gates, and decisions. | Packet + remote model + Tusker import |
| Product review and Start | Review a doctor-valid plan, then explicitly authorize its exact fingerprint. | Human + Tusker |

Global install may install user-level skills and the binary. It must not mutate the current repo unless the user passes an explicit repo target.

## Storage Boundary

Machine-shared runtime and portable project truth have different owners:

| Location | Contents |
| --- | --- |
| `~/Library/Application Support/tusker/` on macOS | Daemon database, project registry, global limits, logs, runtime workspaces, and the managed daemon binary. |
| `<repo>/.tusker/` | `WORKFLOW.md`, task contracts, gates, curated evidence, project knowledge, and generated project views. |
| `<repo>/tusker.yaml` | Repo-level agent and runner configuration where present. |

Do not copy a project's `WORKFLOW.md` into shared state. All Tusker CLI users observe the same repo-local contract.

On macOS, prefer `~/Developer`, `~/Code`, or `~/Projects` for repositories.
LaunchAgents may not inherit Terminal access to `Desktop`, `Documents`,
`Downloads`, or iCloud Drive. Project add and enable warn for those protected
roots. `tusker daemon service install|start` refuses before launch unless the
project is moved, or the operator grants Full Disk Access to the reported
service executable and explicitly passes `--allow-protected-projects`. Other
cloud, network, and removable volumes may also require access; the service
startup health check remains authoritative.

## Setup Commands

Installation is conservative by default: no PATH symlink, no user-level
skill refresh; repo-local skills install when a repo target is supplied.

```bash
tusker purge --repo . --only-tusker-state
tusker install --repo .
tusker init --yes
tusker validate --vault .tusker
tusker skill doctor --strict
```

Opt into extra repo files explicitly:

```bash
tusker init --yes --with-pointers --with-contract
```

Opt in with `tusker install --bin` (PATH symlink) or
`tusker install --all-user-skills` (user skill refresh).

For a clean reset of old Tusker state, run the purge plan, then apply it
and initialize:

```bash
tusker purge --repo . --only-tusker-state --yes
tusker init --yes --fresh --purge-state
tusker skill sync --repo . --mode symlink --source /path/to/tusker
```

`purge` is scoped to generated Tusker state and managed Tusker pointers; it
must not remove product code. `--source` keeps repo-local skill symlinks
pointed at canonical Tusker source even from another repository. Fresh
initialization defaults `automation.enabled` to `false`.

Audit an existing setup without mutating repository or runtime state:

```bash
tusker setup doctor --repo . --source /path/to/tusker --json
```

Apply only deterministic local repairs, then rerun until the report is stable:

```bash
tusker setup repair --repo . --source /path/to/tusker --json
```

For status-only daily use, register the project with `tusker projects add
--repo . --vault ./.tusker`; registry enablement lets the daemon observe state
while repo automation prevents dispatch. Use the explicit Serve automation
control only when the project is ready for unattended work.

The doctor recognizes the canonical Tusker package by its manifest contract,
not merely by finding a `SKILL.md`. For ChatGPT handoff it validates the
`rzn.chatgpt_handoff.config/v1` shape, nested `zip.artifacts_dir` and
`zip.pattern` selector, and the installed `chatgpt/send`, `chatgpt/read`, and
`chatgpt/projects` workflow input contracts. It does not access browser state,
credentials, or the network. Repair may correct a registered legacy vault root,
canonical skill links, and a zip selector inferred from a real local artifact;
project routing and provider workflow refresh remain explicit operator actions.

## Existing Repo Packet

Do not upload the whole repository. Build a task-scoped packet:

```bash
tusker skill pack <TASK-ID> --budget 120000 --for agent
```

A packet contains high-signal facts (README, guidance, manifests, build/CI,
schemas, public APIs, representative tests, summaries, redaction and
skipped-file reports) and excludes generated output, caches, secrets, raw
logs, binaries, and Tusker runtime/generated paths.

Packet contents:

```text
repo-profile.json
file-tree.txt
file-stats.json
language-summary.json
entrypoints.md
build-and-test.md
ci-summary.md
public-api-summary.md
routes-summary.md
data-model-summary.md
test-surface-summary.md
docs-summary.md
existing-agent-guidance.md
selected-excerpts/
skipped-files.md
redaction-report.md
```

Include exact content for small, high-signal files: README, contributing docs,
agent guidance, package manifests, build files, CI, schemas, examples, routes,
CLI maps, public APIs, and representative tests. Exclude generated output,
dependency caches, secrets, raw logs, binary/media files, and Tusker
runtime/generated paths.

## Long-Context Output Contract

The prompt source is `assets/templates/onboard-prompt.md`. It produces a versioned `tusker.onboard_plan/v2` with:

```text
onboarding-plan/
  manifest.json
  report.md
  domains.yaml
  epic-contracts.yaml
  delivery-plans/                 # zero or more tusker.delivery-plan/v2 files
  gates.yaml
  decisions.yaml
  docs-map.yaml
  assumptions.md
  open-questions.md
  migration-report.md
  legacy-compatibility.yaml       # optional, non-executable/read-only adapter
```

The output has observations/report, domains, source-keyed proposed epic contracts, source-keyed human gates, decisions, documentation map, assumptions/questions, and a migration report. It never emits an executable parallel task-list format.

An ordinary V2 delivery plan is optional. Emit one only when packet evidence supports source-keyed `epic_contract`, source-keyed tasks, requirement refs, exact observable acceptance, exact verification command, artifact, and dependencies. It contains no final Tusker IDs or lifecycle fields. Unknown commands, ownership, credentials, product choices, and source conflicts remain assumptions, questions, or source-keyed gates—not fake verification or dispatchable placeholders.

Use the explicit proposal fields `epic_contract.source_key`, task `source_key`,
and human-gate `source_key`. These are stable import keys, never final Tusker
IDs; held import allocates final identities locally.

## Import, Review, and Start Boundaries

All commands below are live in the installed CLI:

```bash
tusker delivery doctor --plan <plan.yaml> --json
tusker delivery review --plan <plan.yaml>
tusker delivery import --plan <plan.yaml> --dry-run --json
tusker delivery import --plan <plan.yaml> --json
tusker delivery start --plan <plan.yaml> --confirm <plan-fingerprint> --by human:<name>
```

Doctor and review are read-only. Dry-run is read-only. Held import may allocate/reconcile IDs but leaves all resulting records held and the wave disarmed. The exact fingerprint-bound Start is the only execution route, and it revalidates the plan and preflight. It cannot enable project automation, install/start a daemon, change runner permissions, satisfy gates, authorize release/spend, or include unrelated work.

## Legacy V1 Readability and Migration

V1 packets/plans remain readable through an explicitly non-executable, read-only legacy compatibility adapter. The adapter preserves references and labels unconverted data; it never materializes records or grants authority.

`migration-report.md` must enumerate fields that cannot be converted without human intent: ambiguous epic mapping, final identities, lifecycle/readiness, ownership, priority/risk, verification/proof, dependencies, gate authority, source conflicts, and unresolved product decisions. Legacy material must pass the same V2 doctor, product review, held import, and fingerprint-bound Start boundaries; it never bypasses them.

## Human Review

After held import, review in this order:

1. `report.md` for broad wrong assumptions.
2. Domain canon for unsupported claims.
3. Source-keyed gates and open questions for true human blockers.
4. V2 delivery-plan acceptance, verification, artifacts, and dependencies.
5. Proposed epic contracts for useful backlog shape.

A product reviewer may choose to resolve a real gate or explicitly Start a
doctor-valid plan; they do not promote guesses into verification.

## Skill Integration

Do not create a separate global Tusker onboarding skill. The canonical operator reference and external prompt are:

```text
skills/tusker/references/REPO_ONBOARDING.md
skills/tusker/assets/templates/onboard-prompt.md
```

The ChatGPT handoff package remains transport only. It must not write Tusker state; feed returned output through the review and held-import boundaries above.

Before browser transport, run the combined local setup diagnostic:

```bash
tusker setup doctor --repo . --source /path/to/canonical/tusker --json
tusker setup repair --repo . --source /path/to/canonical/tusker --dry-run --json
```

The repair command handles deterministic vault/workflow pointers, generated
skill links, and a missing zip attachment default. It deliberately does not
invent ChatGPT Project routing, credentials, or claim that a stale browser
workflow was refreshed; those findings include the exact external action.

Use ChatGPT handoff to submit and collect only:

```bash
chatgpt-handoff doctor
chatgpt-handoff submit --kind architect --brief-file prompt.md --zip repo-packet.zip
chatgpt-handoff collect <job-id> --force
chatgpt-handoff fetch <job-id> --out-dir .tusker/scratch/onboarding/chatgpt-<job-id>
```

If `chatgpt-handoff doctor` reports stale browser workflows, refresh the
installed workflow catalog from the canonical browser source directory:

```bash
rzn-browser workflow pull --repo-root /path/to/rzn-browser
```
