---
title: "Security and privacy"
subject: security
keywords: [privacy, secrets, browser, side-effects]
part_of: overview
read_when: "You handle browser data or workflow side effects."
skip_when: "You need normal build steps. Open developer-guide."
---

# Security and privacy

## Browser access

The extension can read and change pages that the signed-in browser can access.
Treat a workflow as trusted code for that profile. The extension has native
messaging, tabs, scripting, downloads, debugger, and all-page permissions.

Keep cookies, tokens, credentials, page content, snapshots, and private logs out
of Git. Use ignored secret files. Read [`SECURITY.md`](../../SECURITY.md) for
private vulnerability reports.

## Side effects

Review the manifest and every action before you run a workflow that posts,
sends, uploads, votes, follows, or deletes. Use a draft or review step before
an irreversible action.

Side-effect declarations are policy checks, not a sandbox. `execute_javascript`
can run arbitrary page code. `same_origin_request` can use a mutating method.
Do not trust a read-only label without reading the step.

Approval behavior can change with environment settings such as
`RZN_POLICY_AUTO_APPROVE` and `RZN_APPROVAL_MODE=auto_continue`. Treat those
settings as unattended-automation switches.

## LLM data

The browser runs locally, but an LLM endpoint can receive bounded page and DOM
context. The runtime does not provide broad redaction for passwords, payment
data, or personal data. Use a local model endpoint when page text must stay on
the machine. Check the endpoint before using a private page.

## Page bridge

The main-world page bridge is visible to page JavaScript. It can execute
main-world helpers. It is not a secret boundary. Do not claim that page code
cannot access it.

## Before sharing output

- Remove page content, screenshots, tokens, and `.env` files.
- Check logs for headers and private text.
- Use a disposable browser profile for a smoke test.
- Check whether an LLM or report endpoint will receive the data.
