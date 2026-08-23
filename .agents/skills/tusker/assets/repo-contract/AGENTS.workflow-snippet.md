## Contribution workflow snippet

Keep `AGENTS.md` short.

Point contributors and agents to:
- the public issue templates for intake (a change issue maps to the contract sections of a Tusker task)
- the PR template for evidence
- the project repo docs for architecture or policy

For Tusker lookup, start with `tusker search`, `tusker list`, and exact task
paths. Do not search or read generated indexes, raw runner logs, scratch artifacts, or build logs unless the task is explicitly about evidence
forensics.

For token discipline, keep repo-specific command wrappers, build locks, and
forbidden expensive probes in `tusker/SKILL.md` or routed runbooks. Root
`AGENTS.md` should stay a bootstrap pointer, not a command diary.

Do not turn `AGENTS.md` into a giant encyclopedia.
