# Tool Explainer Video

## Overview
- Goal: ship a standalone Remotion explainer that shows what RZN Browser is, how the runtime works, and why the "real Chrome session first" posture matters.
- Constraints: keep the story honest to the product docs, avoid implying cloud browsers or WebDriver-launched sessions, and make the package runnable without touching the extension build.

## Flow Diagrams
- Story flow
```text
hook on real-session reuse
  -> local runtime path
  -> escalation ladder
  -> workflows vs llm-auto
  -> closing positioning
```

- Runtime architecture shown in-scene
```text
task -> RZN runtime -> native host -> extension -> existing Chrome -> page
```

## Decision Record
- Chosen: build a separate Remotion package in `examples/rzn_tool_explainer` instead of mixing video tooling into the root workspace. That keeps the Rust and extension build clean.
- Chosen: make this a motion-graphics explainer, not a fake product demo. The repo docs are strongest when they explain the architecture honestly.
- Chosen: use five short scenes with one composition rather than many tiny compositions. Easier to render, easier to hand off, less sidebar clutter.

## Architecture
- `examples/rzn_tool_explainer/src/Root.tsx`
  Registers the explainer composition.
- `examples/rzn_tool_explainer/src/Composition.tsx`
  Contains the full scene system, motion helpers, cards, flow arrows, and background treatment.
- `examples/rzn_tool_explainer/src/index.css`
  Minimal global reset and type defaults for Remotion Studio.
- `examples/rzn_tool_explainer/package.json`
  Dev, still-render, and final render commands.

## Implementation Notes
- Timing is frame-driven with `useCurrentFrame()`, `spring()`, and `interpolate()` only.
- The copy is sourced from the repo README and visual brief, especially the constraints around local execution and CDP as fallback.
- Each scene owns a narrow message:
  - intro = real-session positioning
  - runtime = local path
  - escalation = DOM-first to CDP ladder
  - modes = workflow vs agent convergence
  - closing = product fit
- The composition runs at `1920x1080`, `30fps`, `720` frames.

## Tasks & Status
- [x] Create standalone Remotion package
- [x] Replace blank scaffold with a full explainer composition
- [x] Add runnable still and render commands
- [ ] Add voiceover, captions, or soundtrack if the human wants a narrated version

## What Works (Do Not Change)
- Keep the product framing honest: local Chrome session, not a stealth-cloud fantasy.
- Keep CDP visually and verbally framed as fallback, not default runtime behavior.
- Keep the package standalone so it does not pollute root build steps.

## Tried & Didn’t Work
- Reusing the root workspace for Remotion: bad fit, because this repo is not already a video project.
- Making the video a literal browser screencast: weaker story, uglier pacing, and it would under-explain the architecture that actually matters.
