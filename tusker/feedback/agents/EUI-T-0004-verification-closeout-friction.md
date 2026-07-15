# EUI-T-0004 verification/closeout friction

- `tusker verify add` replaced the task's verification table and silently dropped the existing manual-proof `pending` row. It should preserve unmatched rows, or expose a supported `pending` result for human verification rows.
- `tusker finish --request-review` requires an attempt but only reports that after all proof work. Surface the requirement in the agent packet/default loop so an interactive agent starts the attempt before implementation.
