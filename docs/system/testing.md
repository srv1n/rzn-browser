---
title: "Testing"
subject: testing
keywords: [tests, validation, smoke, proof]
part_of: overview
read_when: "You need to select a check or report proof."
skip_when: "You need to change behavior. Open architecture first."
---

# Testing

## Choose a check

| Change | Check |
| --- | --- |
| Rust code | `make test` or `make rust ARGS='test -p <crate>'` |
| Extension TypeScript | `make test-ext-unit ARGS='src/example.test.ts'` |
| Browser extension behavior | `make test-ext-e2e` |
| Action schema or generated types | `make schema-check` |
| Workflow JSON | Strict validation and one safe smoke run |
| Native-host files and registration | `make doctor` |

Use the smallest check that can fail if the change breaks. Then run the wider
check that the change can affect.

## Runtime proof

Build and load the exact extension bundle before a browser smoke. Check the
native host. Run a read-only workflow before a write workflow.

`make test` does not prove Chrome, the native host, a signed-in site, or an
installed artifact. `make doctor` checks local files and registration; it does
not perform a browser action. Playwright uses local test pages. Native-host
smoke is opt-in. The fleet smoke script is not a substitute for browser proof.

Record the exact command, exit status, and useful error text. State whether a
failure is in changed code, host setup, browser state, or an unrelated baseline.

## Proof boundary

Keep these claims separate:

1. Focused source check.
2. Full source check.
3. Installed artifact check.
4. Live browser check.
5. Human acceptance.

Do not call a focused green test a release or listening gate.
