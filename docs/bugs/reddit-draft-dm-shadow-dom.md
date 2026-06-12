# Bug: `reddit-draft-dm.json` silently fails to type into chat

## Summary

`reddit-draft-dm.json` v1.6.0 reports all steps as `[OK]` but the message never appears in the chat textarea. The chat sidebar opens correctly, but `wait_for_element` (s4d) and `fill_input_field` (s7) can't reach the textarea because it lives inside nested shadow DOM.

## Root Cause

Reddit's chat sidebar renders inside nested Web Components. The textarea is 4 levels deep in shadow DOM:

```
document
  → SHREDDIT-APP (shadowRoot)
    → ... (shadowRoot)
      → ... (shadowRoot)
        → ... (shadowRoot)
          → textarea[name="message"][placeholder="Message"][aria-label="Write message"]
```

Standard `document.querySelector()` cannot pierce shadow boundaries. The `wait_for_element` and `fill_input_field` steps use top-level CSS selectors, so they either:
- Match nothing and silently succeed (if the step has a fallback/timeout behavior that resolves as OK)
- Match the hidden reCAPTCHA textarea (`g-recaptcha-response`) which is the only `<textarea>` in the main DOM

Either way, the visible chat textarea is never found or typed into.

## Reproduction

```bash
rzn-browser run workflows/reddit/reddit-draft-dm.json \
  --param "recipient=Areuregarded" \
  --param "message_body=Hello"
```

All steps report `[OK]`, but no text appears in the chat sidebar on the profile page.

## Proof: the textarea IS there, just in shadow DOM

This script finds it:

```javascript
function findChatTA(root, depth) {
  if (depth > 10) return null;
  for (const n of root.querySelectorAll('*')) {
    if (n.tagName === 'TEXTAREA' && n.name === 'message' && n.offsetWidth > 0) return n;
    if (n.shadowRoot) {
      const f = findChatTA(n.shadowRoot, depth + 1);
      if (f) return f;
    }
  }
  return null;
}
const ta = findChatTA(document, 0);
// Returns: textarea[name="message"], visible, at ~(1346, 928), depth 4
```

## Working workaround

Replace `wait_for_element` + `fill_input_field` with two `execute_javascript` steps:

**Step 1 — Find and focus the textarea through shadow DOM:**

```javascript
function findChatTA(root, d) {
  if (d > 10) return null;
  for (const n of root.querySelectorAll('*')) {
    if (n.tagName === 'TEXTAREA' && n.name === 'message' && n.offsetWidth > 0) return n;
    if (n.shadowRoot) { const f = findChatTA(n.shadowRoot, d+1); if (f) return f; }
  }
  return null;
}
const ta = findChatTA(document, 0);
if (!ta) throw new Error('chat textarea not found in shadow DOM');
ta.focus();
ta.click();
window.__rzn_chat_ta = ta;
```

**Step 2 — Set value using native setter (React controlled component):**

```javascript
const ta = window.__rzn_chat_ta;
if (!ta) throw new Error('no textarea ref');
ta.focus();
const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
setter.call(ta, MESSAGE_TEXT);
ta.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
ta.dispatchEvent(new Event('change', { bubbles: true, composed: true }));
```

The `composed: true` flag is important — it lets the event cross shadow DOM boundaries so Reddit's React handlers pick it up.

## Suggested fix options

### Option A: Shadow-aware `fill_input_field` (engine-level)

Add a `pierce_shadow: true` flag to `fill_input_field` and `wait_for_element` that recursively searches through shadow roots. This is the cleanest fix and would help any site using Web Components (Reddit, GitHub, etc).

### Option B: Workflow-level JS steps (quick fix)

Replace s4d + s7 in `reddit-draft-dm.json` with the two `execute_javascript` steps from the workaround above. The native setter + composed event dispatch is needed because Reddit uses React controlled components.

### Option C: CDP `DOM.describeNode` with `pierce` flag

Chrome DevTools Protocol's `DOM.querySelector` supports `{ pierce: true }` which traverses shadow DOM. If the engine already uses CDP for `use_cdp: true`, this could be exposed as a selector option.

## Additional notes

- The `force_same_tab: true` on the click step (s4b) correctly prevents navigation to `chat.reddit.com` and keeps the chat sidebar on the profile page. This part works fine.
- The chat sidebar also appears when navigating to `https://chat.reddit.com/user/{recipient}/` but that URL redirects to `https://www.reddit.com/chat/user/{recipient}/` and may not open the correct conversation (it opened the wrong user's chat in testing).
- Quoting hazard: if `{message_body}` contains apostrophes and gets substituted into a JS string literal, it will cause a `SyntaxError`. The engine should either JSON-escape the substitution or provide a `window.__rzn_params.message_body` variable so scripts don't need inline substitution.
