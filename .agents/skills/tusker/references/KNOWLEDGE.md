# Knowledge

Canonical repo knowledge is one visible corpus, shared by humans and agents — never a separate agents fork:

| Node | Lives in | Answers |
|---|---|---|
| Canonical doc | `docs/system/` | how it works today |
| Spec | `.tusker/specs/` | what is changing and why |
| Decision log | `.tusker/specs/decisions/` | what was said, what got locked |

Docs are canonical: an answer comes from a doc read this run, or it is "not in canon" plus an offer to write it. If a doc contradicts the code, current behavior belongs to the code — report the drift and fix the doc in the same change.

## Answer a question

```bash
tusker docs find <query>
```

Deterministic keyword routing over front-matter and headings, ranked canonical doc → spec → decision log. Read the top hit(s) whose `read_when` matches; answer citing paths. One question costs one or two file reads, not a scan.

## Front-matter contract

Every doc's front-matter is its context pointer:

```yaml
subject: token-refresh          # unique key across the corpus
keywords: [tokens, 401, expiry] # aliases docs find matches
part_of: auth                   # parent subject — the DAG edge
describes: [internal/auth/]     # code paths this doc tells the truth about
read_when: "token refresh, session expiry, 401 retry"
skip_when: "login UI -> serve-ui"
last_verified: 2026-08-18       # bump when read against code and found true
```

Specs add `updates:` (docs they will obsolete) and `sources:`; decision logs add `decides_for:`. Maps, indexes, and diagrams are generated output — a hand-drawn map is another document that rots.

## Write or update

Create only through the CLI — it refuses duplicate subjects and points at the file to update in place:

```bash
tusker docs new <subject>
```

- One subject, one doc. Replacement is explicit: the old file becomes a tombstone (`status: superseded`, `superseded_by:`) so stale keywords resolve forward. Version-suffix filenames fail validation.
- Behavior changed means the owning doc changes in the same task; a task diff touching a doc's `describes:` paths owes a doc edit or a waiver row.
- Body: dense prose stating current behavior and why — paths, commands, invariants, the gotcha no config confesses. What one `--help` lookup shows stays out.
- Run `tusker validate --json` after knowledge changes.
