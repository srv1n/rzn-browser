# RZN Browser

## Scope and context

Implement the requested work in the interactive session. Read the affected files
and follow relevant references; a small edit does not require a repository tour.
Preserve unrelated local changes and keep site-specific behavior in workflow data.

For instruction-only edits, update the affected text and stop. Do not build or run
tests unless requested. For code changes, use focused checks appropriate to the
changed behavior; repeat them when a fix or unresolved failure warrants it.

## Build commands

Use `make` for anything that can compile project code, in this checkout and all
worktrees. Prefer `make build`, `make build-rust`, `make build-ext`, or `make test`.
For focused Rust work, use `make rust ARGS='check -p <crate>'` or the needed Cargo
subcommand through `ARGS`; do not invoke Cargo or Bun directly.

Local Rust compilation requires `sccache`. Keep the Makefile and `.cargo/config.toml`
enforcement. CI and release runners intentionally set `RUSTC_WRAPPER: ""` because
sccache is absent there; preserve that exception.

## Skills

Keep skill descriptions short and specific to the task they handle. Put essential
constraints and completion criteria in `SKILL.md`; link detailed procedures only
where they are needed. Prefer current CLI help and nearby workflows over copied
command catalogs. Avoid automatic installation, forced discovery loops, or broad
validation for tasks that do not need them.

## Task tracking and automation

Use the repository tracker for relevant contracts, dependencies, proof, gates,
review, and lifecycle state. Read only the task context needed for the change.
An ordinary interactive request does not authorize a tracker daemon, dispatch,
unattended automation, or nested `codex exec` / `claude -p` workers.

## Git attribution

Use the human's configured Git identity unless they explicitly provide another.
Never add AI authors, committers, co-authors, or generated-by attribution to
commits, PR descriptions, issues, or comments. This applies to every agent.
