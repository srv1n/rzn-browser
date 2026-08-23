---
name: spec
description: Run a grill-style spec session that ends in a canonical spec doc, a decision log, and emitted Tusker tasks. Use when the user wants to spec out a feature, product, or system change, says "/spec", "spec this out", "let's design", or starts a design discussion that should become tracked work.
---

# Spec session: from discussion to dispatched work

You are running a spec session for this repository. The output is never just a
conversation — it is three artifacts in the repo, and the session is not done
until all three exist.

Before reading or writing any doc/spec, run `tusker docs find <query>`; create
canonical docs only through `tusker docs new`.

## The grill protocol (how to run the discussion)

Interview the operator relentlessly until shared understanding, walking each
branch of the decision tree and resolving dependencies between decisions
one by one.

- Ask ONE question at a time. Wait for the answer before the next. Multiple
  questions at once are bewildering.
- For every question, lead with your recommended answer and why.
- If a FACT can be found in the environment (filesystem, code, vault, web),
  look it up — never ask. DECISIONS belong to the operator — always ask,
  never assume.
- Do not act until the operator confirms shared understanding.
- Questions walk the layers in order: customer story → product invariants →
  architecture → program design (schemas, API contracts, types, signatures) →
  vertical slices. Trade-offs (cost, latency, performance) are framed for the
  problem domain at hand — a front-end, a backend, an ETL job, and a mobile
  app each get different questions.

## Artifact 1 — the canonical spec

Write or update `.tusker/specs/<subject>.md`:

- Front-matter: `title`, `status: canonical`, `read_when`, `skip_when`,
  `sources`, `decisions_locked`.
- One canonical document per subject. No versions — update in place.
- Structure: why → plain-language product context (PM-readable) → audience
  branches as needed (dev: schemas/contracts; ops; marketing) → explicit
  "Deferred (not now)" list.
- Obsidian-style `[[backlinks]]` to related specs, used sparingly.
- Plain language throughout; a non-engineer must be able to read the top half.

## Artifact 2 — the decision log

Write `.tusker/specs/decisions/<date>-<subject>-grill.md`:

- One entry per decision: the question asked, options offered with your
  recommendation, what the operator ACTUALLY said (condensed but faithful —
  keep their stories and incidents, they carry the why), and what got locked.
- Backlink it from the spec's sources; front-matter `read_when` points readers
  who want the why here, and `skip_when` points decision-only readers back to
  the spec.

## Artifact 3 — the canonical system docs stay true

The living reference docs in `docs/system/` describe how the system works
TODAY (features, diagrams, tables — the newcomer's map). If the decisions
locked in this session change any documented behavior, updating the affected
`docs/system/*.md` files is part of the session's emitted work — either edit
them directly for small deltas or cut an explicit doc-update task into the
epic. A spec that changes a design without truing up the canonical doc is
incomplete.

## Artifact 4 — the emitted work

End the session by cutting Tusker tasks:

- `tusker new epic` / `tusker new task` per the tusker skill; set real
  `dependencies:` edges for ordering.
- Every task: two-layer contract — plain-language top (what/why/done-as-
  artifacts, NO symbols or paths; the validate lint enforces this), then an
  `## Implementation notes` appendix with the file map (mark best guesses as
  such), then acceptance + verification tables with exact commands and
  pre-named test substrings.
- Every non-trivial task carries `spec_refs` pointing at the spec file.
- Acceptance is artifact-first: UI → before/after screenshots; performance →
  benchmark before/after; security → posture artifact.
- Human gates only for: missing credentials, unclear spec, or replacement
  decisions. Never for code review.

## House rules that bind every spec session here

- Plain language everywhere, including proposed identifiers.
- The operator reviews specs, schemas, and artifacts — never plumbing code.
- Implementation/review dispatches run on Opus with an explicit model
  override; spec work stays in the planner session.
- No AI attribution anywhere; commits author as the configured git user.
