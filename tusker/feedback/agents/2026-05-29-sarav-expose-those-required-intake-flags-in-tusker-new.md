# Agent Feedback

- context: Created MBR-T-0001 from an external story pack.
- friction: Creating a ready task failed because verification details are required, but tusker new task --help does not show the --intent/--acceptance/--verification flags named in the error hint.
- product-idea: Expose those required intake flags in tusker new task --help, or provide a one-command path for creating an implementation-ready task from an external story.
- impact: Agents fall back to backlog task creation and have to proceed with less precise acceptance wiring.
- related: MBR-T-0001
- dedupe-key: new-task-help-missing-verification-flags
