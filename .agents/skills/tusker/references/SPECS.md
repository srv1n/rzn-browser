# Documentation and spec habits

Use the repository's one documentation corpus for humans and agents. Before
reading or writing a doc or spec, run `tusker docs find <query>` so the current
answer and its subject are known. Create new material with `tusker docs new`
only; update an existing subject in place instead of making a versioned copy.

Canonical docs live in `docs/system/`, specs in `.tusker/specs/`, and decision
logs in `.tusker/specs/decisions/`. Keep front matter complete, connect each
node to its parent, and run `tusker docs map` followed by `tusker validate`
after changing the corpus. Generated indexes, diagrams, and graph artifacts
are outputs, never hand-maintained source.

When a spec locks decisions, its `updates:` targets must land with the change
or have an explicit doc-update task in the owning epic. Do not silently leave
legacy copies competing with the canonical answer; use `tusker docs adopt`
for brownfield triage and approve the complete proposal as one batch.
