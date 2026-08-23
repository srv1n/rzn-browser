# Run

Run one task to completion under human control, resolve its gates, and watch it. `automation.enabled` gates autonomous daemon pickup only; a run directive is deliberate human authority and dispatches even while automation is off.

## Preconditions

```bash
tusker projects add --repo . --vault ./.tusker   # registers disabled: daemon observes, never auto-dispatches
tusker daemon status --json
```

Dispatch needs the resident daemon alive. Starting it (`tusker daemon service start`, or `tusker daemon run` in a dedicated terminal) is the operator's action in their own shell — an agent session reports the status and the command, and implements requested work itself instead of spawning workers.

## Dispatch one task

The task must be `ready` or `rework`. The one-shot dispatch is the Serve play button: `tusker serve`, open the project board, press Run on the task. It queues a run directive the daemon consumes once, and the directive bypasses `automation.enabled` — `tusker automation dispatch <TASK-ID>` also dispatches but passes the same eligibility checks as polling, automation flag included. `tusker automation plan` and `explain` answer "why won't this dispatch" read-only.

## Watch

```bash
tusker runs inspect <TASK-ID>
tusker runs logs <TASK-ID> --lines 50      # --follow to tail
tusker next                                 # what the daemon would pick
```

Serve shows the same live: run display is liveness-derived, so `running` means a held lease with a fresh heartbeat — a stale badge is a stale run, not a UI bug. `tusker runs interrupt <TASK-ID>` stops a live run; `tusker runs retire` clears a settled failed one.

## Gates and human waits

```bash
tusker gate list --json
tusker gate satisfy <GATE-ID> --by human:<name> --evidence "<how it was met>"
tusker gate waive <GATE-ID> --by human:<name> --reason "<why waived>"
```

Satisfy and waive carry human authority: run them on explicit human instruction, naming that human in `--by`. A run parked on `waiting_on_human` names its exact unblocking action in the task capsule — report it and stop.

## After the run

The daemon hands finished work to `review`. Verify and close through `TRACK.md`'s lifecycle: inspect the diff, record proof, `tusker close`.
