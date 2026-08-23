---
title: "Extension and native host"
subject: extension-native-host
keywords: [extension, native-host, bridge, IPC, CDP]
part_of: architecture
read_when: "You need to change or diagnose the browser bridge."
skip_when: "You only need a CLI command. Open cli-runtime."
---

# Extension and native host

## Connection path

```text
supervisor -- local JSON-RPC --> native host
native host -- Chrome native messaging --> extension service worker
service worker -- runtime messages --> content script
content script -- page bridge or CDP --> page
```

The browser starts the native host. The native host forwards frames and does
not own workflow policy. The supervisor owns local readiness and dispatch.
The source is in [`supervisor.rs`](../../crates/rzn_browser/src/supervisor.rs),
[`rzn_native_host/src/main.rs`](../../crates/rzn_native_host/src/main.rs), and
[`extension/src/background.ts`](../../extension/src/background.ts).

The local protocol is `rzn.local`. The supervisor accepts a token handshake
from normal clients and a `runtime.hello` bridge handshake from the native host.
The bridge accepts `native_host.extension_call` messages.

## Extension parts

The base manifest loads:

- a Manifest V3 service worker;
- an isolated content script in each frame;
- main-world page and shadow-DOM scripts;
- popup and dashboard pages.

The maintained source targets are Chrome, Edge, and Chromium.

The service worker owns native reconnects, bridge health, browser target
routing, workflow sessions, dashboard calls, and CDP actions. The content
script owns normal DOM reads and actions. The page bridge provides main-world
helpers. CDP is an explicit fallback for selected trusted-input, frame, and
accessibility cases.

## Frame limits

| Link | Limit |
| --- | ---: |
| Supervisor local frame | 16 MiB |
| Chrome to native host | 64 MiB |
| Native host to Chrome | 1 MiB |

Large responses can use a private local artifact. The shared framing code is in
[`rzn_core/src/framing.rs`](../../crates/rzn_core/src/framing.rs).

## Health

The service worker reconnects after missed heartbeats. The supervisor checks
bridge capabilities and browser target identity. Epoch checks drop replies from
a worker or bridge tied to another runtime root.

Use this order when you diagnose a bridge:

```sh
rzn-browser native-host doctor --browser chrome --json
rzn-browser supervisor status
rzn-browser browser targets
rzn-browser supervisor ensure-ready
```

`doctor` checks files and registration. It does not prove a live page action.

## Security boundary

The extension has broad page, tab, scripting, download, debugger, and native
messaging permissions. Treat it as trusted code for the signed-in browser
profile. The page bridge runs in the page world and is visible to page
JavaScript. Do not use it as a secret boundary.
