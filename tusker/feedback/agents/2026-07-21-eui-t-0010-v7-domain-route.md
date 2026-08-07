# V7 task domain routing rejects legacy project canon

- Context: Shaping and claiming `EUI-T-0010` in this repository.
- Friction: Declaring the existing `codebase` domain made `tusker packet` reject the task because V7 looked under `.tusker/knowledge/domains/`, while this repo's project skill routes to `tusker/domains/`. The task had to drop a valid domain tag after the canon was already read.
- Impact: Task packets cannot use the repository's existing domain knowledge during the V6-to-V7 layout transition, and the failure appears only after claim.
- Suggested fix: Resolve domain routes through `tusker/SKILL.md` during migration, or reject unsupported domain tags before a task becomes claimable.
