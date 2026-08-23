# Operate

Diagnose tracker state read-only:

```bash
tusker capabilities --json
tusker show <TASK-ID> --capsule
tusker proof status <TASK-ID>
tusker closeout status <TASK-ID> --json
tusker execution show <EXECUTION-ID>   # one execution strand
tusker execution inbox                 # unbound execution strands
tusker execution list                  # execution graph
```

Provider observations remain authority-neutral; recovery procedures live in
`docs/runbooks/execution-observability.md`.

The smallest command that names the task, lifecycle state, open gate, or missing proof wins.

## Recovery boundary

Correct clearly-requested state through ordinary lifecycle commands. Setup repair, daemon, dispatch, service, and fleet operations are their own explicitly-requested tasks, never a side effect of task management. Tracker failure stays separate from implementation results, and Tusker repair becomes a task only when the user asks.
