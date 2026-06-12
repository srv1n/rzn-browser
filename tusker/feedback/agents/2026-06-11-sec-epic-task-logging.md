# SEC epic task logging (2026-06-11)

Logged SEC epic + SEC-T-0001..0014 (all `ready`) from the 2026-06-11 audit digest. Each contract carries: audit context with file:line anchors (marked re-verify-before-edit), step-by-step implementation plan, acceptance table, exact verification commands, non-goals with cross-task boundaries. Ordering/file-ownership constraints live in the SEC epic body.

Operator friction worth fixing or knowing:

1. `tusker new task --status ready` correctly rejects placeholder contracts, but the error doesn't say "create as backlog first" — the backlog→fill→reconcile→promote flow should be in the skill's COMMANDS.md.
2. Editing task bodies after creation trips CAS (`state_rev`) on the next control op; `tusker reconcile` repairs. Expected, but the create→fill flow makes it mandatory every time.
3. `tusker/WORKFLOW.md` still declared `tracker_schema_version: 6` + legacy `active` state; `automation plan` hard-errored. Bumped to 7 / `ready` this turn. If daemon dispatch misbehaves, look here first.
4. Pre-existing: `automation plan` reports "verification missing exact command or manual proof" for every ready task incl. MBR-T-0038 (accepted format) — checker appears to fire while inline results are `pending` or project unregistered; don't burn time "fixing" task bodies against it. `tusker next` likewise returns no pickable tasks; legacy `tusker/epics/WCP/*` v6 records fail `tusker validate`.
5. Project is not registered for daemon automation (`tusker projects`), and SEC-T-0001/0002 are risk=critical → explicit human dispatch required by policy even after registration.
