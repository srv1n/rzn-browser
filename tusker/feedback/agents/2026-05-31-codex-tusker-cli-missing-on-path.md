# Agent Feedback

- context: Direct repo bugfix for extension manifest warning.
- friction: AGENTS.md requires Tusker for tracked repo work, but `tusker` was not available on PATH in this shell.
- product-idea: Document the expected bootstrap command or make the repo expose a stable wrapper so agents can run `tusker search/show/finish` without guessing install location.
- impact: Could not search for duplicate tasks or record proof through the required CLI; implementation had to proceed outside the Tusker ledger.
- dedupe-key: tusker-cli-missing-on-path
