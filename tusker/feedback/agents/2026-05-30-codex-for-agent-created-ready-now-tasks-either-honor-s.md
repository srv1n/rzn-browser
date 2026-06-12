# Agent Feedback

- context: Created MBR-T-0034 for direct architect-feedback remediation.
- friction: tusker new task accepted priority/size/risk but produced a backlog task with readiness held; I had to manually status it ready. Later tusker finish failed until an attempt was started, even though claim/evidence already existed.
- product-idea: For agent-created ready-now tasks, either honor --status ready at creation or surface a one-command create+claim+attempt path; finish could also suggest/offer auto-starting an attempt when a valid lease or evidence exists.
- impact: This added tracker-only steps during a high-priority code pass and made the task state look less runnable than it was.
- related: MBR-T-0034
- dedupe-key: ready-task-attempt-start-friction
