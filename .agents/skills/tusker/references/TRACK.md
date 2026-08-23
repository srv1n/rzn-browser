# Track

Create, update, and close tracked work. Inspect before mutating:

```bash
tusker show <TASK-ID> --capsule      # contract, state, next action
tusker proof status <TASK-ID>
```

## Create

One bounded outcome is one task — a conversation without one is not a task yet. An epic exists only when several real tasks share a product outcome. Capture observable acceptance and non-goals. A genuinely unresolved decision stays an open question or a gate — placeholder acceptance and invented verification commands poison the contract.

```bash
tusker new epic --vault ./.tusker --acronym APP --title "App foundation"
tusker new task --vault ./.tusker --epic APP --title "Implement auth" \
  --priority p2 --size m --risk medium
```

## Lifecycle

Durable flow is `idea -> backlog -> ready -> review -> done`, always via CLI:

```bash
tusker status <TASK-ID> ready --reason "Contract is actionable."
tusker verify add <TASK-ID> --covers A1 --check "command: go test ./..." --result pass
tusker status <TASK-ID> review --reason "Acceptance proof recorded."
tusker close <TASK-ID>
```

`tusker discard <TASK-ID>` cancels; `tusker status <TASK-ID> rework --reason "<what changed>"` reopens. Runtime activity is not a durable status.

## Proof

The smallest verification set covering acceptance: exact command, PASS/FAIL, bounded note. Noisy output lives in `.tusker/scratch/<TASK-ID>/`; promote only what acceptance or a gate consumes. At tiers 2+, `close` refuses until proof satisfies the contract — that refusal is the product working, so supply the missing row rather than working around it.
Scratch is not durable: it is deleted when the task closes and swept after 14 days regardless.

At tier 1 (`tier: 1` in `.tusker/config.yaml`) create/status/close work as a plain tracker: no dispatch/proof contract check on ready, while a demanding task with no `spec_refs` emits the `TASK_SPEC_REF_REQUIRED` warning. There is no proof gate on close. Record what exists; ceremony arrives with higher tiers, not before.

## Gates

A gate records one missing human fact: authority, credential, unresolved product intent, or subjective acceptance (UX feel, brand, legal). Anything the task, spec, or a linked decision already settles is settled; risk alone is not a gate.

```bash
tusker new gate --vault ./.tusker --blocks <TASK-ID> --kind <KIND> \
  --owner human:<name> --action "<ACTION>" --verification "<PROOF>" \
  --why-agent-cannot "<BOUNDARY>"
```
