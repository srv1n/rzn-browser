# Agent Feedback

- context: BRR-T-0009 bugfix task creation
- friction: A freshly created agent-owned task appeared as readiness held, so runnable work required an extra status transition before implementation.
- product-idea: When new task --epic is created with next_owner agent and a concrete summary, default readiness could be ready or the CLI could print the exact status command to make it runnable.
- impact: Small but repetitive setup tax for urgent CI-fix tasks.
- related: BRR-T-0009
- dedupe-key: new-agent-task-held-readiness
