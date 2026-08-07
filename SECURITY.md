# Security Policy

## Reporting A Vulnerability

Report privately through GitHub Security Advisories:
[srv1n/rzn-browser/security/advisories/new](https://github.com/srv1n/rzn-browser/security/advisories/new).

Please do not open a public issue for a security problem.

Include what you would need yourself to reproduce it: version, OS, the workflow or command
involved, and the smallest sequence of steps that triggers the behaviour. Expect a first reply
within a week. If a fix is warranted, we will coordinate disclosure with you before publishing.

## Scope

RZN Browser is a local tool that drives the operator's own Chrome profile through a native
messaging host and an extension. That shapes what counts as a vulnerability.

In scope:

- Privilege escalation beyond what the operator asked for — a workflow, page, or remote site
  reaching capabilities it was never granted.
- Anything that lets a visited web page drive the runtime, reach the native host, or execute
  local commands.
- Credential, cookie, or session data leaving the machine other than to the site the workflow is
  operating on.
- Workflows performing side effects they do not declare in their manifest (writes, downloads,
  network access, auth changes).
- Flaws in the native messaging host, extension message handling, or CDP fallback path.

Out of scope:

- The tool acting on a signed-in session. That is the entire point: RZN drives a browser you are
  already authenticated in, with your own cookies, and a workflow you run can do anything you
  could do by hand on that site. Read a workflow's declared side effects before running it.
- Automating a third-party site against that site's terms of service. That is between you and
  the site.
- Vulnerabilities in Chrome, in the sites being automated, or in third-party LLM providers.
- Findings that require an attacker to already have local code execution or filesystem access as
  your user.

## Running Untrusted Workflows

Workflow JSON is executable input. Treat a workflow from a stranger the way you would treat a
shell script from a stranger: read it, check its declared side effects with
`rzn-browser workflow inspect`, and run it against a session you are willing to expose to it.
