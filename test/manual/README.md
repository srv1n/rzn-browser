# Manual Test Harnesses

This directory holds the old loose root-level smoke assets in one place.

## Layout

- `browser/`: HTML pages you open directly in Chrome for extension diagnostics, including `diagnose.html`.
- `scripts/`: ad hoc shell harnesses for manual smoke tests and debugging.
- `workflows/`: small JSON workflows used by the manual scripts.
- `extension-test-instructions.md`: step-by-step extension verification.
- `google-search-test-steps.md`: focused Google workflow smoke steps.

## Quick Start

Run the generic manual harness from the repo root:

```bash
./test/manual/scripts/test.sh --list-workflows
./test/manual/scripts/test.sh -w workflows/google/google-search.json --param query="browser automation"
```

Useful one-offs:

```bash
./test/manual/scripts/test_broker_connection.sh
./test/manual/scripts/test_llm_minimal.sh 'your-api-key'
cargo run -p rzn-browser -- run test/manual/workflows/test_navigation.json
```

Manual result logs now land under `test-results/manual/` instead of polluting the repo root.
