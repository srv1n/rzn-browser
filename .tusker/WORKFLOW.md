---
agents:
    default: codex_acp
    enabled:
        - codex_acp
        - codex_exec
        - claude-code
    max_concurrent_agents: 2
    max_concurrent_agents_by_state:
        rework: 1
automation_enabled: false
claude:
    command: claude -p --output-format stream-json --input-format stream-json --permission-mode bypassPermissions
codex:
    approval_policy: on-request
    command: codex exec --json --skip-git-repo-check -
    max_turns: 1
    read_timeout_ms: 30000
    stall_timeout_ms: 120000
    thread_sandbox: workspace-write
    turn_sandbox_policy: workspace-write
    turn_timeout_ms: 600000
codex_cloud:
    apply_mode: ""
    collect_command: ""
    command: ""
    environment_id: ""
    pr_mode: ""
    status_command: ""
extensions:
    allow_tusker_read_tools: false
    allowed_mcps: []
    allowed_tools: []
    enabled: false
external_loop:
    maxcycles: 3
    maxexternalthreads: 5
    maxrepaircontinuations: 2
    wallclocktimeouthours: 8
fanout:
    allowed_child_types: []
    enabled: false
    max_children: 0
    merge_rule: manual_review
hooks:
    after_workspace_create: []
    before_workspace_remove: []
retry:
    backoff_ms:
        - 30000
        - 120000
        - 600000
    max_attempts: 3
reviewer:
    actor: agent:reviewer/codex
    auto_close_risks:
        - low
        - medium
        - high
        - critical
    max_cycles: 3
    prompt: |-
        You are the independent Tusker reviewer for {{ note.id }}.

        Review the task acceptance, proof, and gates. Tusker does not control repository operations. Your only Tusker lifecycle output is one typed result submitted with `tusker review submit`.

        Task:
        - ID: {{ note.id }}
        - Title: {{ note.title }}
        - Risk: {{ note.risk }}
        - Status: {{ note.status }}
        - Attempt: {{ attempt.id }}
        - Workspace: {{ workspace.path }}
        - Vault: {{ vault.path }}

        Policy:
        - Reviewer actor: {{ reviewer.actor }}

        Checklist:
        1. Read the task acceptance contract, proof mode, verification rows, evidence cards, and gates.
        2. Run the smallest verification needed to prove the acceptance contract.
        3. Risk alone does not justify a human gate. Create or honor one only for a named capability, external authority, unresolved product fact, or contractually subjective acceptance; do not re-approve choices already settled by the task/spec.
        4. Record any acceptance gap as an actionable typed finding.

        Submit exactly one result for the injected review attempt: `tusker review submit {{ note.id }} --attempt {{ attempt.id }} --task-rev {{ review.task_rev }} --source-sha {{ review.source_sha }} --work-rev {{ review.work_rev }} --proof-fingerprint {{ review.proof_fingerprint }} --gate-fingerprint {{ review.gate_fingerprint }} --verdict pass|changes_requested|blocked --covers <acceptance-ids> --summary "<bounded summary>"`. A pass requires complete objective proof and satisfied gates; changes_requested needs an actionable finding; blocked needs a machine, infrastructure, or genuine-human blocker.

        Explicit blocking gates must be reported in the typed result; do not change gate or task state.
    runner: codex_acp
runners:
    claude-code:
        command: claude -p --output-format stream-json --input-format stream-json --permission-mode bypassPermissions
        kind: claude-code
    codex_exec:
        command: codex exec --json --skip-git-repo-check -
        kind: codex_exec
runtime:
    budget:
        daily_input_tokens: 20000000
        daily_output_tokens: 1000000
        enabled: false
        per_attempt_input_tokens: 2000000
        per_attempt_output_tokens: 100000
        per_task_input_tokens: 6000000
        per_task_output_tokens: 300000
    lease_ttl_ms: 900000
    max_active_runs_per_project: 1
    max_continuation_retries: 3
    poll_interval_ms: 60000
    sentinel:
        checks:
            - held_lease_dispatch_eligible
            - attempt_count_within_caps
            - fresh_heartbeat_pid_live
            - unique_active_lease_per_task
            - last_poll_advanced
        fresh_heartbeat_ms: 120000
    serve:
        addr: 127.0.0.1:7420
scheduled_promotion:
    mode: disabled
    version: 1
tracker:
    dispatch_states:
        - ready
        - rework
    kind: tusker_vault
    review_states:
        - review
    terminal_states:
        - done
        - cancelled
        - superseded
tracker_schema_version: 7
workflow_version: 1
workspace:
    root: .
    strategy: shared
# Declared proof policy for this repo. These are the defaults Tusker already
# applies at task-create time (defaultV7ProofMode / defaultV7EvidenceBudget);
# this stanza records the repo's policy for humans and agents to read and is not
# itself consulted at runtime, so editing it does not change resolution.
# Evidence-by-risk-class: inline is the floor; only evidence-bearing modes
# attach files.
proof:
  # proof_mode is the default proof depth Tusker assigns a new task that does
  # not declare its own: inline for every risk class EXCEPT critical, which
  # defaults to audit. inline records verification rows (command + PASS/FAIL)
  # directly on the task and writes no evidence files.
  proof_mode: inline
  # proof_mode for risk=critical tasks; audit adds independent_review and
  # evidence files on top of the inline test proof.
  proof_mode_critical: audit
  # evidence_budget caps the evidence files a task may attach. 0 keeps inline
  # tasks file-free; only the evidence-bearing modes below raise this.
  evidence_budget: 0
  # Evidence files are required ONLY for these evidence-bearing proof modes.
  # Every other mode (inline, focused_test, broad_test, ...) proves inline with
  # no files.
  evidence_bearing_modes:
    - card
    - artifact
    - audit
orchestration:
  # branch_age_warning_hours warns when a task branch outlives this many hours.
  branch_age_warning_hours: 48
  batch_gate:
    # enabled turns on the periodic wave-boundary batch gate.
    enabled: false
    # period_hours is the batch-gate cycle length when no windows are set.
    period_hours: 24
    # max_repairs caps repair continuations Tusker attempts per batch cycle.
    max_repairs: 3
  # orchestration.gate is this project's gate contract. The floor values below
  # ship COMMENTED OUT as placeholders: uncomment and set them to the project's
  # real, measured toolchain values before relying on the gate. Nothing here
  # inherits an unmeasured floor.
  gate:
    # profile is the canonical gate profile name. Replace this placeholder; a
    # run requesting a different profile is refused rather than discarding the
    # warm build.
    profile: default
    # harvest_commands is the runner's no-fail-fast test/build form, e.g.
    # "go test ./..." or "cargo nextest run --no-fail-fast". Defaults to the
    # batch gate's commands when empty.
    harvest_commands:
      - make test
    # min_free_disk_gb MUST be MEASURED against this project's real peak build
    # footprint, never guessed, before you uncomment it. On 2026-07-20 an
    # unmeasured guess of 15 GB authorized a doomed run: it died on a full disk
    # mid-gate, and its recovery deleted the build cache the next run needed.
    # Measure the peak footprint and set the floor above it.
    # min_free_disk_gb: <measured-peak-build-gb>
    # defect_target_regex has exactly one capture group naming the failing
    # target, e.g. "^--- FAIL: (\S+)" for Go.
    defect_target_regex: '^--- FAIL: (\S+)'
    # defect_line_limit caps each harvested defect excerpt.
    defect_line_limit: 12
    # scopes enable the Stage 1 per-change gate ("tusker gate --changed"): map an
    # area of the repo to the harvest commands that cover it, and a change is
    # gated on only the scopes it touched. A touched path that no scope owns fails
    # closed to the full harvest_commands set above rather than being skipped, so
    # scopes narrow proof cost without ever narrowing coverage. Uncomment and set
    # to this project's real areas; leave empty to only ever run the whole gate.
    # scopes:
    #   - name: api
    #     paths:
    #       - internal/api
    #     commands:
    #       - go test ./internal/api/...
    #   - name: store
    #     paths:
    #       - internal/store
    #     commands:
    #       - go test ./internal/store/...
# runner escalation reasons: system_error|security_concern|unresolvable_conflict|stuck_loop
---

## Routing

Use Tusker only for task tracking.

## Prompt

Work on {{ note.id }} using the user request and repository rules. Tusker only tracks the task contract, status, proof, and gates; it does not control repository operations.

Item: {{ note.title }}
Record: {{ note.record_id }}
Type: {{ note.type }}
Vault: {{ vault.path }}

Inspect only the task context needed, perform the authorized work, and record the smallest truthful task update through the Tusker CLI. If Tusker is broken, report that separately without blocking otherwise-authorized work.

## Retry policy

Retry only failed task-tracking operations when state changed.

## Human override policy

The user owns authority outside Tusker task records.
