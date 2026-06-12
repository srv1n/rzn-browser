# CDP Integration Guide - FSM-Driven First-Class Key Input

## Overview

RZN implements **first-class CDP integration** as a core component of the FSM-driven architecture. Unlike optional fallback systems, CDP is used strategically for reliable keyboard and mouse input that can't be detected or blocked by modern web applications.

**Key Principle: CDP provides trusted, undetectable input events when static actions aren't sufficient.**

## Architecture Integration with FSM

### CDP in FSM Context

The FSM system uses CDP for specific tool implementations:

```rust
// FSM allows press_key in Search mode
PlannerMode::Search => vec!["type", "press_key", "wait"]

// press_key action uses CDP for maximum reliability  
match action.cmd.as_str() {
    "press_key" if key == "Enter" && fsm.mode == Search => {
        // Use CDP for trusted Enter key event
        fsm.transition(Results);
    }
}
```

### Three-Tier Action System

1. **Static Actions (CSP-Safe, Default)** - DOM manipulation without JavaScript execution
2. **Enhanced JavaScript Actions** - Better event simulation with human-like behavior  
3. **CDP Actions (First-Class)** - Chrome DevTools Protocol for trusted events

### When CDP is Used

CDP engages for specific actions requiring trust:
- **press_key**: All keyboard input uses CDP for reliability
- **Complex mouse interactions**: When pixel-perfect timing is needed
- **Cross-origin frame access**: OOPIF (Out-of-Process iframes) support
- **File upload scenarios**: When trusted events are required

## Implementation Details

### CDP-Based Key Input (IMPLEMENTED)

```typescript
// extension/src/actions/press_key.ts

export async function press_key_cdp(key: string): Promise<ActionResult> {
    const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
    const tabId = tabs[0].id!;
    const debuggee = { tabId };
    
    // Attach debugger for CDP access
    await chrome.debugger.attach(debuggee, '1.3');
    
    try {
        const mapping = getKeyMapping(key);
        
        // Dispatch trusted keyDown event via CDP
        await chrome.debugger.sendCommand(debuggee, 'Input.dispatchKeyEvent', {
            type: 'keyDown',
            key: mapping.key,
            code: mapping.code,
            windowsVirtualKeyCode: mapping.keyCode,
            nativeVirtualKeyCode: mapping.keyCode,
            modifiers: 0
        });
        
        // Small delay for realistic timing
        await new Promise(resolve => setTimeout(resolve, 50));
        
        // Dispatch trusted keyUp event via CDP  
        await chrome.debugger.sendCommand(debuggee, 'Input.dispatchKeyEvent', {
            type: 'keyUp',
            key: mapping.key,
            code: mapping.code,
            windowsVirtualKeyCode: mapping.keyCode,
            nativeVirtualKeyCode: mapping.keyCode,
            modifiers: 0
        });
        
        return { success: true, message: `Pressed key: ${key}` };
        
    } finally {
        // Always detach debugger to maintain stealth
        await chrome.debugger.detach(debuggee);
    }
}
```

### Background Script Integration

```typescript
// extension/src/background.ts

chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
    if (request.action === 'press_key_cdp') {
        press_key_cdp(request.key)
            .then(result => sendResponse(result))
            .catch(error => sendResponse({ 
                success: false, 
                error: error.message 
            }));
        return true; // Will respond asynchronously
    }
});
```

### Content Script CDP Handler

```typescript
// extension/src/contentScript.ts

const actions = {
    // Enhanced press_special_key handler routes to CDP
    press_special_key_enhanced: async (step: any) => {
        const key = step.key || 'Enter';
        const response = await chrome.runtime.sendMessage({
            action: 'press_key_cdp',
            key: key
        });
        
        if (response.success) {
            return {
                success: true,
                message: `Key pressed: ${key}`,
                action_taken: `press_key_cdp_${key}`
            };
        } else {
            throw new Error(`CDP key press failed: ${response.error}`);
        }
    }
};
```

## Key Features

### 1. Unified Frame Tree (UFT)
Solves the iframe problem by building a complete tree across all frames:
- Assigns unique EncodedIds (`frameOrdinal:backendNodeId`)
- Works across cross-origin boundaries
- Maintains element stability during DOM changes

### 2. Accessibility-First Context
Reduces DOM to semantic information for LLM:
- Role, name, state from AX tree
- Minimal geometry for interaction
- 2-8KB context windows instead of raw HTML

### 3. Auto-Escalation
Automatically escalates tiers on failure:
```
Pure JS → Enhanced JS → CDP Fallback
```
Never escalates unless necessary.

### 4. Site-Specific Overrides
```typescript
// Configure per-site behavior
strategy.updateConfig({
  siteOverrides: new Map([
    ['*.google.com', { 
      features: { cdpForTrustedEvents: true } 
    }],
    ['*.stripe.com', { 
      tier: ExecutionTier.CDPFallback 
    }]
  ])
});
```

## Stealth Considerations

### What We Do
- CDP via `chrome.debugger` API (not remote debugging port)
- Attach only when needed, detach immediately
- Never enable Console domain (fingerprint risk)
- Use real user profile with cookies/extensions
- Humanized delays and mouse paths

### What We Don't Do
- ❌ No `--remote-debugging-port` flag
- ❌ No HeadlessChrome UA
- ❌ No WebDriver properties
- ❌ No persistent CDP sessions
- ❌ No Console domain unless debugging

## Performance

- **Pure JS**: ~10ms per action, 0% detection risk
- **Enhanced JS**: ~50ms per action, minimal detection risk  
- **CDP Fallback**: ~100ms per action (attach/detach overhead)
- **CDP Always**: ~30ms per action (session kept alive)

## Debugging

```typescript
// Enable debug logging
import { cdp } from './cdp/cdpHelper';

// Check if CDP is attached
console.log('CDP attached:', cdp.isAttached(tabId));

// View current strategy
import { strategy } from './cdp/executionStrategy';
console.log(strategy.exportConfig());

// Force CDP for testing
strategy.updateConfig({ 
  tier: ExecutionTier.CDPAlways,
  stealth: { cdpOnDemandOnly: false }
});
```

## Migration from Pure Actions

Existing RZN actions work unchanged. The system automatically:
1. Maps action types to the new tiered system
2. Attempts pure JS first
3. Escalates only if needed
4. Reports which tier succeeded

```json
// Response includes execution details
{
  "success": true,
  "data": {
    "tier": "enhanced-js",
    "method": "enhanced-events",
    "escalated": true  // Started with pure-js, escalated
  }
}
```

## Best Practices

1. **Always start with Pure JS** - Let auto-escalation handle edge cases
2. **Use CDP sparingly** - Only for cross-origin frames and trusted events
3. **Monitor escalations** - If a site always escalates, add a site override
4. **Test both modes** - Ensure workflows work with and without CDP
5. **Keep sessions short** - Attach/detach quickly to minimize exposure

## FAQ

**Q: Will this make RZN detectable?**
A: No. CDP is off by default and only activates briefly when needed. The chrome.debugger API is less detectable than remote debugging ports.

**Q: Do I need to change my workflows?**
A: No. Existing workflows continue using pure JS. CDP only engages for failures.

**Q: What about performance?**
A: Pure JS mode is fastest. CDP adds ~100ms when it attaches, but this only happens for complex cases.

**Q: Can I disable CDP completely?**
A: Yes. Don't grant the debugger permission, or set `tier: ExecutionTier.PureJS` with `autoEscalate: false`.

## Summary

The CDP integration is designed to be **invisible by default** while providing a safety net for complex automation scenarios. It follows the principle of "escalate only when necessary" to maintain RZN's stealth-first approach while solving real-world automation challenges like cross-origin iframes and trusted event requirements.