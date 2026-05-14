---
workflow_version: 1
tracker_schema_version: 6
tracker:
    kind: tusker_vault
    active_states:
        - active
        - rework
    review_states:
        - review
    terminal_states:
        - done
        - cancelled
agents:
    default: codex
    enabled:
        - sarav
        - codex
        - claude-code
        - gemini
    max_concurrent_agents: 3
    max_concurrent_agents_by_state:
        rework: 1
runtime:
    poll_interval_ms: 5000
    lease_ttl_ms: 900000
    max_active_runs_per_project: 1
workspace:
    root: workspaces
    strategy: worktree
retry:
    max_attempts: 3
    backoff_ms:
        - 30000
        - 120000
        - 600000
codex:
    command: codex app-server
    approval_policy: on-request
    thread_sandbox: workspace-write
    turn_sandbox_policy: workspace-write
    turn_timeout_ms: 600000
    read_timeout_ms: 30000
    stall_timeout_ms: 120000
    max_turns: 1
reviewer:
    enabled: true
    runner: codex
    actor: agent-reviewer
    auto_close_risks:
        - low
        - medium
    human_required_risks:
        - high
        - critical
    prompt: |-
        You are the independent Tusker reviewer for {{ note.id }}.

        Review only. Do not edit implementation files. If the work needs changes, mark the task `rework` with a specific reason instead of fixing it yourself.

        Check acceptance, scope, evidence, verification, and knowledge resolution. Low/medium risk work may be verified and closed by the configured reviewer after gates pass; high/critical work remains human-gated.
claude:
    command: claude -p --output-format stream-json --input-format stream-json --permission-mode bypassPermissions
extensions:
    enabled: false
    allowed_tools: []
    allowed_mcps: []
    allow_tusker_read_tools: false
hooks:
    after_workspace_create: []
    before_workspace_remove: []
---

## Routing

You are working on {{ note.id }} for {{ project.name }}. Dispatch only makes sense because this task is currently {{ note.status }} and the workspace is ready at {{ workspace.path }}.

## Prompt

Use the installed Tusker skill bundle for durable task semantics, evidence, and verification discipline. Work inside {{ workspace.path }}. Treat {{ repo.root }} as the source repository root for context only unless the task explicitly requires comparing against it.

Item: {{ note.title }}
Record: {{ note.record_id }}
Type: {{ note.type }}
Attempt: {{ attempt.number }}
Workflow: {{ workflow.path }}
Vault: {{ vault.path }}

## Completion contract

When the work is demonstrably ready for verification, move the task to `review`. If the work is blocked, set status to `blocked` with a concrete blocker instead of exiting cleanly. If the task remains active after a turn, the daemon will continue or retry the same session.

## Retry policy

Retry only transient infrastructure failures. Human-directed rework creates a new active task revision.

## Human override policy

Humans may edit tasks directly, but runtime state belongs to the daemon store.
