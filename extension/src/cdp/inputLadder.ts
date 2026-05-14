// Tiered Input Synthesis Ladder - Escalates only when necessary
import { cdp } from './cdpHelper';
import { frameRouter } from './frameRouter';
import { parseEncodedId } from '../types/targets';
import { strategy, ExecutionTier } from './executionStrategy';
import type { EncodedId } from './uft';

export interface ActionParams {
  type: 'click' | 'type' | 'setValue' | 'select' | 'scroll';
  selector?: string;
  encodedId?: EncodedId;
  text?: string;
  value?: string;
  options?: {
    trusted?: boolean; // Require trusted events
    crossOrigin?: boolean; // Target is in cross-origin frame
  };
}

export interface ActionResult {
  success: boolean;
  tier: ExecutionTier;
  method: string;
  error?: string;
  escalated?: boolean;
}

// Main entry point - automatically chooses best tier
export async function executeAction(
  tabId: number,
  params: ActionParams
): Promise<ActionResult> {
  const url = await getCurrentUrl(tabId);
  const currentTier = strategy.getTierForUrl(url);
  
  console.log(`[InputLadder] Executing ${params.type} with tier: ${currentTier}`);
  
  // Start with configured tier
  let result = await executeTier(tabId, params, currentTier);
  
  // Auto-escalate if failed and allowed
  if (!result.success && strategy.config.escalation.autoEscalate) {
    const tiers = getEscalationPath(currentTier);
    
    for (const nextTier of tiers) {
      console.log(`[InputLadder] Escalating to ${nextTier}`);
      result = await executeTier(tabId, params, nextTier);
      result.escalated = true;
      
      if (result.success) {
        // Record successful tier for future optimization
        console.log(`[InputLadder] Success with ${nextTier}`);
        break;
      }
    }
  }
  
  // Record failure if still unsuccessful
  if (!result.success && params.selector) {
    strategy.recordFailure(params.selector, result.tier);
  }
  
  return result;
}

async function executeTier(
  tabId: number,
  params: ActionParams,
  tier: ExecutionTier
): Promise<ActionResult> {
  // Apply delay for stealth
  const delay = strategy.getActionDelay();
  if (delay > 0) {
    await sleep(delay);
  }
  
  switch (tier) {
    case ExecutionTier.PureJS:
      return executePureJS(tabId, params);
      
    case ExecutionTier.EnhancedJS:
      return executeEnhancedJS(tabId, params);
      
    case ExecutionTier.CDPFallback:
      // Only use CDP if specific conditions met
      if (params.options?.crossOrigin || params.options?.trusted) {
        return executeCDP(tabId, params);
      }
      // Otherwise fall back to enhanced JS
      return executeEnhancedJS(tabId, params);
      
    case ExecutionTier.CDPAlways:
      return executeCDP(tabId, params);
      
    default:
      return executePureJS(tabId, params);
  }
}

// Tier 1: Pure JavaScript (most stealthy)
async function executePureJS(
  tabId: number,
  params: ActionParams
): Promise<ActionResult> {
  try {
    const results = await chrome.scripting.executeScript({
      target: { tabId, allFrames: true },
      func: (p: ActionParams) => {
        const el = p.selector 
          ? document.querySelector(p.selector) as HTMLElement
          : null;
          
        if (!el) return { ok: false, error: 'Element not found' };
        
        switch (p.type) {
          case 'click':
            el.click();
            return { ok: true };
            
          case 'type':
          case 'setValue':
            if ('value' in el) {
              (el as HTMLInputElement).value = p.text || p.value || '';
              el.dispatchEvent(new Event('input', { bubbles: true }));
              el.dispatchEvent(new Event('change', { bubbles: true }));
              return { ok: true };
            }
            return { ok: false, error: 'Not an input element' };
            
          case 'select':
            if (el instanceof HTMLSelectElement) {
              el.value = p.value || '';
              el.dispatchEvent(new Event('change', { bubbles: true }));
              return { ok: true };
            }
            return { ok: false, error: 'Not a select element' };
            
          case 'scroll':
            el.scrollIntoView({ behavior: 'smooth', block: 'center' });
            return { ok: true };
            
          default:
            return { ok: false, error: 'Unknown action type' };
        }
      },
      args: [params],
      world: 'MAIN'
    });
    
    const success = results.some(r => (r.result as any)?.ok);
    const error = results.find(r => (r.result as any)?.error)?.result?.error;
    
    return {
      success,
      tier: ExecutionTier.PureJS,
      method: 'content-script',
      error
    };
  } catch (e) {
    return {
      success: false,
      tier: ExecutionTier.PureJS,
      method: 'content-script',
      error: e instanceof Error ? e.message : 'Unknown error'
    };
  }
}

// Tier 2: Enhanced JavaScript with realistic event sequences
async function executeEnhancedJS(
  tabId: number,
  params: ActionParams
): Promise<ActionResult> {
  try {
    const results = await chrome.scripting.executeScript({
      target: { tabId, allFrames: true },
      func: (p: ActionParams) => {
        const el = p.selector 
          ? document.querySelector(p.selector) as HTMLElement
          : null;
          
        if (!el) return { ok: false, error: 'Element not found' };
        
        // Helper for realistic mouse events
        function simulateMouseSequence(element: HTMLElement) {
          const rect = element.getBoundingClientRect();
          const x = rect.left + rect.width / 2;
          const y = rect.top + rect.height / 2;
          
          const options = {
            bubbles: true,
            cancelable: true,
            view: window,
            clientX: x,
            clientY: y,
            button: 0
          };
          
          // Full sequence for maximum compatibility
          element.dispatchEvent(new MouseEvent('mouseover', options));
          element.dispatchEvent(new MouseEvent('mouseenter', options));
          element.dispatchEvent(new MouseEvent('mousemove', options));
          element.dispatchEvent(new MouseEvent('mousedown', options));
          element.dispatchEvent(new MouseEvent('mouseup', options));
          element.dispatchEvent(new MouseEvent('click', options));
        }
        
        // Helper for realistic keyboard events
        function simulateTyping(element: HTMLInputElement, text: string) {
          element.focus();
          
          // Clear existing value
          element.value = '';
          
          // Type character by character
          for (const char of text) {
            const keydownEvent = new KeyboardEvent('keydown', {
              key: char,
              code: `Key${char.toUpperCase()}`,
              bubbles: true,
              cancelable: true
            });
            
            const keypressEvent = new KeyboardEvent('keypress', {
              key: char,
              code: `Key${char.toUpperCase()}`,
              bubbles: true,
              cancelable: true
            });
            
            const inputEvent = new InputEvent('input', {
              data: char,
              inputType: 'insertText',
              bubbles: true,
              cancelable: true
            });
            
            element.dispatchEvent(keydownEvent);
            element.dispatchEvent(keypressEvent);
            element.value += char;
            element.dispatchEvent(inputEvent);
            
            const keyupEvent = new KeyboardEvent('keyup', {
              key: char,
              code: `Key${char.toUpperCase()}`,
              bubbles: true,
              cancelable: true
            });
            element.dispatchEvent(keyupEvent);
          }
          
          element.dispatchEvent(new Event('change', { bubbles: true }));
        }
        
        switch (p.type) {
          case 'click':
            simulateMouseSequence(el);
            return { ok: true };
            
          case 'type':
            if ('value' in el) {
              simulateTyping(el as HTMLInputElement, p.text || '');
              return { ok: true };
            }
            return { ok: false, error: 'Not an input element' };
            
          case 'setValue':
            if ('value' in el) {
              (el as HTMLInputElement).focus();
              (el as HTMLInputElement).value = p.value || '';
              el.dispatchEvent(new InputEvent('input', {
                data: p.value,
                inputType: 'insertText',
                bubbles: true
              }));
              el.dispatchEvent(new Event('change', { bubbles: true }));
              return { ok: true };
            }
            return { ok: false, error: 'Not an input element' };
            
          case 'select':
            if (el instanceof HTMLSelectElement) {
              el.focus();
              el.value = p.value || '';
              el.dispatchEvent(new Event('change', { bubbles: true }));
              return { ok: true };
            }
            return { ok: false, error: 'Not a select element' };
            
          case 'scroll':
            // Smooth human-like scroll
            el.scrollIntoView({ behavior: 'smooth', block: 'center' });
            window.scrollBy(0, -50 + Math.random() * 100); // Small random offset
            return { ok: true };
            
          default:
            return { ok: false, error: 'Unknown action type' };
        }
      },
      args: [params],
      world: 'MAIN'
    });
    
    const success = results.some(r => (r.result as any)?.ok);
    const error = results.find(r => (r.result as any)?.error)?.result?.error;
    
    return {
      success,
      tier: ExecutionTier.EnhancedJS,
      method: 'enhanced-events',
      error
    };
  } catch (e) {
    return {
      success: false,
      tier: ExecutionTier.EnhancedJS,
      method: 'enhanced-events',
      error: e instanceof Error ? e.message : 'Unknown error'
    };
  }
}

// Tier 3: CDP (only when necessary)
async function executeCDP(
  tabId: number,
  params: ActionParams
): Promise<ActionResult> {
  // Check if we should use CDP
  if (strategy.config.stealth.cdpOnDemandOnly) {
    const url = await getCurrentUrl(tabId);
    if (!strategy.shouldUseCDP('cdpForTrustedEvents', url)) {
      console.log('[InputLadder] CDP requested but disabled by strategy');
      return executeEnhancedJS(tabId, params);
    }
  }
  
  try {
    return await cdp.with(tabId, async () => {
      await cdp.enable(tabId, ['Input', 'DOM']);
      
      // Resolve nodeId via encodedId (preferred) or selector
      let nodeId: number | undefined;
      let sessionRoute: { sessionId?: string } = {};
      
      if (params.encodedId) {
        const { frameOrdinal, backendNodeId } = parseEncodedId(params.encodedId);
        const frameTree = await frameRouter.getFrameTree(tabId);
        const frameInfo = frameTree[frameOrdinal];
        if (!frameInfo) throw new Error('Invalid encodedId frame ordinal');
        sessionRoute = cdp.routeForFrame(frameInfo.frameId);
        
        const push = await cdp.send<any>(
          tabId,
          'DOM.pushNodesByBackendIdsToFrontend',
          { backendNodeIds: [backendNodeId] },
          sessionRoute
        );
        nodeId = push?.nodeIds?.[0];
        if (!nodeId) throw new Error('Failed to resolve node from encodedId');
      } else {
        // Fallback to selector within main document
        const doc = await cdp.send<any>(tabId, 'DOM.getDocument', { depth: -1 });
        if (!params.selector) {
          throw new Error('Selector or encodedId required for CDP execution');
        }
        const node = await cdp.send<any>(
          tabId,
          'DOM.querySelector',
          { nodeId: doc.root.nodeId, selector: params.selector }
        );
        nodeId = node?.nodeId;
        if (!nodeId) throw new Error('Element not found');
      }
      
      // Get box model for coordinates
      const box = await cdp.send<any>(
        tabId,
        'DOM.getBoxModel',
        { nodeId },
        sessionRoute
      );
      
      const quad = box.model?.content || box.model?.border;
      if (!quad || quad.length < 8) {
        throw new Error('Could not get element position');
      }
      
      const x = (quad[0] + quad[2]) / 2;
      const y = (quad[1] + quad[5]) / 2;
      
      // Execute action with CDP
      switch (params.type) {
        case 'click':
          await cdp.send(tabId, 'Input.dispatchMouseEvent', {
            type: 'mousePressed',
            x,
            y,
            button: 'left',
            clickCount: 1
          }, sessionRoute);
          await cdp.send(tabId, 'Input.dispatchMouseEvent', {
            type: 'mouseReleased',
            x,
            y,
            button: 'left',
            clickCount: 1
          }, sessionRoute);
          break;
          
        case 'type':
        case 'setValue':
          // Focus element first
          await cdp.send(tabId, 'DOM.focus', { nodeId }, sessionRoute);
          
          // Clear and type
          const text = params.text || params.value || '';
          for (const char of text) {
            await cdp.send(tabId, 'Input.dispatchKeyEvent', {
              type: 'keyDown',
              text: char,
              unmodifiedText: char
            }, sessionRoute);
            await cdp.send(tabId, 'Input.dispatchKeyEvent', {
              type: 'keyUp',
              text: char,
              unmodifiedText: char
            }, sessionRoute);
          }
          break;
          
        default:
          throw new Error(`CDP action ${params.type} not implemented`);
      }
      
      return {
        success: true,
        tier: ExecutionTier.CDPFallback,
        method: 'cdp-input'
      };
    });
  } catch (e) {
    return {
      success: false,
      tier: ExecutionTier.CDPFallback,
      method: 'cdp-input',
      error: e instanceof Error ? e.message : 'Unknown error'
    };
  }
}

// Helper functions
function getEscalationPath(fromTier: ExecutionTier): ExecutionTier[] {
  const allTiers = [
    ExecutionTier.PureJS,
    ExecutionTier.EnhancedJS,
    ExecutionTier.CDPFallback,
    ExecutionTier.CDPAlways
  ];
  
  const startIndex = allTiers.indexOf(fromTier);
  return allTiers.slice(startIndex + 1);
}

async function getCurrentUrl(tabId: number): Promise<string> {
  try {
    const tab = await chrome.tabs.get(tabId);
    return tab?.url || tab?.pendingUrl || '';
  } catch {
    return '';
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}
