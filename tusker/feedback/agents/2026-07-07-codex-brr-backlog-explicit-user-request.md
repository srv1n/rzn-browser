---
date: 2026-07-07
agent: codex
area: task-selection
---

# Explicit user request matched a backlog task

`BRR-T-0007` matched the user's requested bridge reliability work, but `tusker automation plan BRR-T-0007 --json` returned `do_not_dispatch` because the project is unregistered and the task is `backlog`. There was no obvious task-control path for "human explicitly asked to do this now in chat" short of working outside Tusker. A documented adopt/override flow would reduce ambiguity for urgent product fixes that map to backlog contracts.
