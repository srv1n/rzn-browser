# Extension hot-path efficiency and hardening review

Date: 2026-09-08  
Reviewed base: `dda372177202c56ed00a04d04176987bb4d49d9c`

## Decision and scope

Reduce duplicate work and repair failure-state accounting before changing timing,
concurrency limits, browser permissions, or input semantics. This first patch is
intentionally limited to the content snapshot module and the CDP client, with new
regression tests. It is not a complete audit of the Rust runtime, native transport,
workflow catalog, or every sensitive-data capture path.

The inspected call paths are active: `contentScript.ts` imports the snapshot
module, and `background.ts` imports the CDP integration layer. The source changes
leave workflow definitions, action APIs, normal form values, stable element IDs,
prompt references, input-event sequences, wait/retry budgets, dependencies, browser
permissions, and existing tests unchanged. The intentional data-output exception
is omission of password-input `value` attributes from this snapshot path.

## Findings and changes

### 1. A normal capture repeats a substantial amount of DOM work

Previously, `captureCurrentDOM()` called `buildSnapshot(120)` and then `domHash()`,
which independently called `buildSnapshot(50)`. Each retained element also had its
rectangle read once for visibility and again for spatial metadata.

The patch reuses the first 50 elements of captures whose requested limit is at
least 50, preserving the existing hash algorithm. Requests below 50 still perform
the independent 50-element hash capture. This matters: simply hashing a ten-element
result would silently weaken loop detection outside those ten elements.

A private visibility helper returns the rectangle already read. Offscreen and
zero-size elements can be rejected before fetching computed style. Interactive
priority, breadth-first fallback, snapshot limits, selectors, and ordering remain
unchanged.

### 2. Wide breadth-first queues do unnecessary work and retain visited nodes

The old generator used repeated `Array.shift()` and a strong `Set` of visited
nodes. It now advances through current/next frontier arrays and uses a `WeakSet`
for visited-node protection. It still reads children after yielding each element,
so consumers that mutate a live DOM keep the previous traversal semantics.
Already-visited detached nodes are no longer strongly retained by the visited set.
This is a structural retention improvement, not a measured browser-RSS claim.
An absent document body now produces an empty traversal rather than a crash.

### 3. Child CDP sessions are routed through the wrong argument

The wrapper previously inserted `sessionId` into the protocol command's params.
Chrome's flat-session API expects it on the `DebuggerSession` target passed as the
first argument to `chrome.debugger.sendCommand`.

The patch puts it on that target, preserves explicit-session precedence, supports
`tabId: 0`, and keeps `frameId` out of `Runtime.evaluate` protocol parameters.
Frame routing and domain accounting now use the same resolved target/session.
Flat-session support is a Chrome 125+ API; real-browser child-frame validation
remains a merge gate.

Reference: [Chrome debugger API: attach to related targets](https://developer.chrome.com/docs/extensions/reference/api/debugger#attach-to-related-targets).

### 4. Failed and overlapping domain leases can corrupt local state

Previously, a domain reference was incremented before `.enable` succeeded. A
failed enable could therefore make the next acquisition skip the actual enable.
A concurrent borrower could also finish before Chrome acknowledged the first
borrower's enable. Accounting ignored frame-derived child sessions, and an
unheld release could issue an unnecessary `.disable`.

The patch serializes domain lease changes per resolved target/session, commits
references only after acknowledgement, rolls back the references acquired by a
failed batch, skips unheld releases, and removes empty reference/operation maps.
Rollback preserves references owned by earlier borrowers and preserves the
original error if best-effort cleanup also fails. Duplicate-domain counts are
retained rather than silently deduplicated.

This is not global command serialization: ordinary CDP commands and other
sessions/tabs are not queued behind a lease operation. Console remains excluded.

### 5. Command watchdogs survive synchronous throws; late callbacks affect state

A synchronous `chrome.debugger.sendCommand` exception previously rejected the
promise but left its timeout alive. The patch clears watchdogs on every immediate
completion path. Once a timeout settles the request, late callbacks consume
`chrome.runtime.lastError` without logging success or marking a newer tab lease
detached.

A local timeout still does **not** cancel a command already executing in Chrome.
The patch does not introduce automatic retries of browser mutations.

### 6. Passive snapshots include password-input value attributes

The shared attribute builder now excludes only the `value` attribute of inputs
whose type is password, including case-insensitive type spellings. Those controls
remain in the snapshot with their selectors and identifying attributes; ordinary
input values are preserved. The page itself is not modified.

This reduces accidental disclosure through this snapshot JSON path. It is not a
claim that screenshots, explicit extraction, CDP/AX snapshots, or all logging
surfaces have a complete sensitive-data policy.

### 7. Follow-up: root bookkeeping was still being sent as a Chrome session

`FrameRouter` used `root:<tabId>` and `root:unknown` internally, then exposed
them as if they were child session IDs. The Chrome debugger API accepts only
Chrome-issued child IDs on `DebuggerSession`; root commands must address the
tab target with no `sessionId`. The event listener also discarded
`source.sessionId`, which is where Chrome identifies the child session that
emitted a frame or execution-context event.

The router now uses `source.sessionId` for child mappings and keeps tab ownership
with every child session. Its frame-session iterator returns a command target:
the root is `{ tabId }`, while a child is `{ tabId, sessionId }`. AX collection
and CDP evaluation consume that target directly, so protocol routing and domain
lease keys agree. A focused real-router/mock-transport test failed on the old
head with `root:7`/`root:unknown`, then passed with root, unknown, child,
explicit-session, and lease-accounting coverage.

## Measured evidence

The baseline copies of both modified modules were verified against their GitHub
blob hashes before comparison:

- `dom-capture.ts`: `56bd9025603022f152fdf43ebfdc7c6b1571d02f`
- `cdpClient.ts`: `3f6b2458d1ed9f7d149914619656dfa55b74465c`

### Deterministic capture fixture

Fixture: 200 visible interactive elements, unique IDs, fixed rectangles, default
120-element capture. The before/after capture objects matched after removing only
the varying timestamp, including element IDs, order, hash, prompt, and metadata.

| Browser-facing operation | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| Interactive selector scans | 2 | 1 | 50.0% |
| Rectangle reads | 340 | 120 | 64.7% |
| Computed-style reads | 170 | 120 | 29.4% |

These are operation counts in an instrumented fixture, not a claimed percentage
reduction in total browser CPU, memory, or workflow latency. The new unit tests
assert the counts without brittle wall-clock thresholds.

### Synthetic breadth-first traversal

Node v22.16.0/V8, a root with the indicated number of leaf children, two warmups,
then the median of five runs. Both implementations visited exactly the same nodes.

| Leaf children | Before | After |
| ---: | ---: | ---: |
| 10,000 | 2.507 ms | 2.209 ms |
| 50,000 | 144.624 ms | 11.722 ms |

This isolates generator behavior on a wide synthetic tree. It is not a browser
DOM benchmark, a normal 120-element capture benchmark, or an end-to-end speedup.
Real pages, renderer layout, native messaging, network waits, and browser-session
memory have not been measured here.

## Validation and merge gates

**Completed locally after the follow-up:** `make test-ext-unit` passed **31 test
files / 142 tests**, including the real-frame-router transport regression;
`make build-ext` and `make build-ext-release` built the Chrome, Edge, and Chromium
bundles. The targeted test was first run against the original PR head and failed
with root bookkeeping IDs on the debugger target, then passed after the repair.

Coverage includes capture/hash compatibility at small and large limits, stable
IDs, traversal order and mutation handling, absent body, visibility, password
redaction on both paths, routing precedence, tab zero, timer cleanup, late
callbacks, error codes, concurrent acquisitions, failed enable/retry, partial
rollback, rollback failure, release ordering, frame/tab isolation, unheld releases,
duplicate domains, and repeated bookkeeping cleanup.

**Browser proof remains blocked locally:** after installing Playwright Chromium,
`make test-ext-e2e` reached Playwright's runner but did not launch Chromium or
report a test; it was stopped rather than waiting out the CI budget. The workflow
had a separate baseline defect: it built with the test bridge disabled although
all Playwright tests wait for bridge-only APIs. The workflow and Make E2E target
now set `RZN_PAGE_TEST_BRIDGE_ENABLED=1`; this is a test-build-only setting, not
a production behavior change. New browser coverage exercises main and OOPIF child
targets through real Chrome, navigation and tab closure, ordinary typing/clicking
(existing action coverage), and snapshots at 20 and 80 elements. CI must execute
that coverage before merge.

**Observed GitHub CI after opening PR #1:** the Extension Build and Unit Tests
job passed for code commit `57757f5abb7178ea3714722ef69a99fec921d603` (GitHub
PR merge ref `9ded528125db681a4e15b02bb0b305d57243c380`). Actual Vitest 4.1.6
reported **30 test files and 141 tests passed**, including all 38 new cases.
Chrome, Edge, and Chromium bundles built successfully, and both ChatGPT workflow
contract checks passed. This is additional validation beyond the local adapter.
At the time that job was inspected, the separate Playwright workflow and remaining
Rust/security jobs were still running; their success is not implied.

Evidence: [extension build/unit job](https://github.com/srv1n/rzn-browser/actions/runs/34265784021/job/102194667971).

Before merging, require the remaining PR CI checks and browser smoke coverage.
The focused tests can also be reproduced with the normal repository toolchain:

```sh
make test-ext-unit ARGS='src/content/dom-capture.test.ts src/cdp/cdpClient.test.ts'
make test-ext-unit
make build-ext-release
make test-ext-e2e
```

Smoke-test same-process and out-of-process frames, navigation/tab closure during
CDP work, ordinary typing/clicking, and captures at limits below and above 50. A
failure in the existing suite must be investigated, not resolved by weakening its
assertions. Until those gates are observed, this is a draft PR.

## Next work, in priority order

1. **Fix selector identity separately.** `bestSelector()` interpolates IDs,
   classes, and attribute values without proper CSS escaping; fallback selectors
   need not be globally unique. `diff()` keys elements by selector, so duplicates
   can overwrite one another. These are concrete correctness risks, but changing
   selector/delta identity deserves its own compatibility tests rather than being
   hidden inside this optimization.
2. **Audit detach/reconnect generations and all sensitive-data paths.** Domain
   rollback is now safer, but external detach/reconnect lifecycle ownership still
   needs end-to-end tests; this patch is not a generation-aware lease redesign.
   Apply an explicit sensitive-data policy consistently across DOM, AX/CDP,
   screenshots, artifacts, and logs. Do not assume password-attribute filtering is
   a complete privacy boundary.
3. **Add measured performance budgets and assess browser-test coverage.** The
   main CI builds benchmark targets with `cargo bench --no-run`; it does not
   establish a measured performance budget. A separate `extension-e2e.yml`
   workflow already runs Playwright with Chromium and the extension on PRs; keep
   it, rather than adding a duplicate lane. Extend that existing coverage where
   needed for child-frame and detach/reconnect behavior. Add reproducible
   workflow/page fixtures and track p50/p95 capture/command latency, idle CPU,
   renderer/native-host memory, round trips, and leaked lease counts.
4. **Profile before splitting or replacing architecture.** The background and
   content scripts are large, but source-file size alone is not evidence of a
   runtime bottleneck. Measure startup/injection and active allocations before
   pursuing lazy loading, transport batching, or broader Rust changes. Keep
   deliberate keystroke behavior, approval gates, retries, and waits intact unless
   equivalent behavior is demonstrated with browser tests.

## Files

- [Snapshot implementation](../../extension/src/content/dom-capture.ts)
- [Snapshot regression tests](../../extension/src/content/dom-capture.test.ts)
- [CDP client implementation](../../extension/src/cdp/cdpClient.ts)
- [CDP client regression tests](../../extension/src/cdp/cdpClient.test.ts)
- [Existing CI](../../.github/workflows/ci.yml)
- [Existing Playwright CI](../../.github/workflows/extension-e2e.yml)
