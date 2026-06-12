#+ CDP Frame Router

## Overview
- Goal: Provide robust, cross-origin frame control via Chrome DevTools Protocol (CDP) with OOPIF routing.
- Constraints: MV3 background service worker; multiple frames/targets; CSP-safe content scripts; minimal permissions.

## Flow Diagrams

- End-to-end routing
```
Background (SW) → frameRouter.attachToTab(tabId)
  ├─ Target.setAutoAttach(flatten=true)
  ├─ Page.enable, DOM.enable, Accessibility.enable
  └─ Session map: frameId → CDP sessionId
```

- Command dispatch
```
sendCommand(method, params, { frameId? })
  ↓ resolve sessionId via routeForFrame(frameId)
  ↓ cdpClient.sendCommand({ tabId }, method, params, { sessionId })
  ↓ chrome.debugger.sendCommand
```

## Call Graphs

- Attach
```
background.ts
  └─ frameRouter.attachToTab(tabId)
     ├─ chrome.debugger.attach({ tabId }, '1.3')
     ├─ enableDomains(['Target', 'Page'])
     ├─ autoAttach flatten=true, waitForDebuggerOnStart=false
     └─ onEvent listener → track frame attach/detach, sessions
```

- Send command
```
cdp/index.ts: sendCommand
  └─ cdpClient.sendCommand({ tabId }, method, params, { sessionId })
     ├─ resolve session via frameRouter.routeForFrame(frameId)
     └─ chrome.debugger.sendCommand
```

## Architecture
- `extension/src/cdp/frameRouter.ts`: Tracks frames, sessions, events; exposes `attachToTab`, `detachFromTab`, `routeForFrame`, `getFrameTree`.
- `extension/src/cdp/cdpClient.ts`: Typed `sendCommand`, domain enable helpers, AX tree, layout metrics, element pushing.
- `extension/src/background.ts`: Orchestrates attach/route, ensures content readiness.
- `extension/src/cdp/index.ts`: Consolidated API re-exports (legacy wrappers removed).

## Implementation Notes
- Always attach with `Target.setAutoAttach` and `flatten=true` for OOPIF.
- Maintain per-tab state: sessions, event listeners, frame tree.
- For AX/DOM queries, enable domains once per session.

## Tasks & Status
- [x] FrameRouter attach/detach/session map
- [x] sendCommand with routed `sessionId`
- [x] Accessibility/DOM helpers in cdpClient
- [ ] Expand coverage for media/input/target domains as needed

## What Works (Do Not Change)
- `flatten=true` attach with auto-attach
- Route lookup: frameId → sessionId map

## Tried & Didn’t Work
- Ad-hoc `chrome.debugger` calls without routing: breaks on OOPIF pages.
- Per-frame reattach on every call: brittle and slow.

