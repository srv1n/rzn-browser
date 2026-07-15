# Interactive automation boundary friction

- The installed Tusker operator skill currently requires `tusker automation plan <TASK-ID> --json` as its first action, but the interactive-session migration makes automation planning opt-in. The operator guidance should distinguish interactive task inspection from explicitly requested automation so it cannot reintroduce a recursive runner path.
