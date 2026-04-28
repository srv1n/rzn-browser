# RZN Browser Explainer Video

Standalone Remotion package for a 25-second product explainer covering what RZN Browser is,
why it exists, and how the runtime escalates from DOM-first control to CDP only when needed.

## Scenes

1. Product hook: real Chrome session reuse instead of a throwaway bot browser.
2. Local runtime path: task -> runtime -> native host -> extension -> live Chrome.
3. Escalation ladder: DOM-first, scripted events, short CDP attach.
4. Two entry points: workflows and `llm-auto` converging on one execution stack.
5. Closing frame: the honest positioning for local, reality-proof browser automation.

## Commands

```bash
cd examples/rzn_tool_explainer
npm i
npm run dev
```

Render a still:

```bash
npm run still
```

Render the full video:

```bash
npm run render
```
