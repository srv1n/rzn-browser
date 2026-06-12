// CDP Input Rung - Chrome DevTools Protocol (works everywhere)
// Most powerful, handles cross-origin, but potentially more detectable

import { ResolvedElement, parseEncodedId } from '../../types/targets';
import { InputAction } from '../ladder';
import { frameRouter } from '../../cdp/frameRouter';
import { CDPClient } from '../../cdp/cdpClient';

type Point = { x: number; y: number };
type ActionWait = 'navigation' | 'networkIdle' | 'selector' | undefined;

interface ExecuteResult {
  success: boolean;
  error?: string;
  meta?: any;
}

// CDP client interface (would be injected or imported)
interface CDPClient {
  sendCommand<T = any>(method: string, params?: any): Promise<T>;
  isConnected(): boolean;
}

// Global CDP client (would be initialized elsewhere)
declare global {
  interface Window {
    __RZN_CDP_CLIENT__?: CDPClient;
  }
}

export class CDPInputExecutor {
  private cdp = new CDPClient();

  constructor() {
    // CDP client is injected via frameRouter
  }

  /**
   * Check if CDP can be used for this element/action combination
   */
  canExecute(element: ResolvedElement, action: InputAction): boolean {
    // Only allow CDP in contexts where chrome.debugger is available (background/service worker)
    const canUseDebugger = typeof chrome !== 'undefined' && !!(chrome as any).debugger && typeof (chrome as any).debugger.sendCommand === 'function';
    if (!canUseDebugger) return false;
    return ['click', 'fill', 'key', 'hover', 'scroll', 'type_and_submit', 'batch_actions'].includes(action.type);
  }

  /**
   * Execute batch actions using Chrome DevTools Protocol
   */
  async executeBatchActions(startFrameId: string, steps: any[]): Promise<ExecuteResult> {
    const { sessionId } = frameRouter.routeForFrame(startFrameId);
    
    try {
      // Enable required domains for the entire batch
      await this.cdp.enableDomains({ sessionId }, ["Page", "Input", "DOM", "Runtime"]);
      
      // Process each step in sequence
      for (let i = 0; i < steps.length; i++) {
        const step = steps[i];
        const op = step.op as string;
        
        console.log(`[CDPRung] Executing batch step ${i + 1}/${steps.length}: ${op}`);
        
        let targetEl: ResolvedElement | null = null;
        
        // Resolve target element if needed
        if (step.encodedId) {
          // Parse frameId:backendNodeId format
          const parts = String(step.encodedId).split(':');
          if (parts.length === 2) {
            const frameId = parts[0];
            const backendNodeId = Number(parts[1]);
            targetEl = {
              frameId,
              backendNodeId,
              encoded_id: step.encodedId
            } as ResolvedElement;
          }
        } else if (step.selector) {
          // Import and use ElementResolver for deep selector resolution
          const { ElementResolver } = await import('../../resolver/elementResolver');
          const resolver = new ElementResolver();
          try {
            targetEl = await resolver.resolve({
              css: step.selector,
              frame_ordinal: startFrameId === 'main' ? 0 : 1
            });
          } catch (error) {
            console.warn(`[CDPRung] Failed to resolve selector ${step.selector}:`, error);
            // Continue with next step rather than failing entire batch
            continue;
          }
        }
        
        // Execute the operation
        switch (op) {
          case 'click': {
            if (!targetEl) {
              console.warn(`[CDPRung] Batch step ${i + 1}: click requires selector or encodedId`);
              continue;
            }
            const coords = await this.centerOf(targetEl);
            await this.executeCDPClick(coords, { 
              type: 'click', 
              frameId: targetEl.frameId || startFrameId 
            } as InputAction);
            break;
          }
          
          case 'insert_text': {
            if (!targetEl) {
              console.warn(`[CDPRung] Batch step ${i + 1}: insert_text requires selector or encodedId`);
              continue;
            }
            const text = step.text || '';
            await this.executeCDPFill(targetEl, { 
              type: 'fill', 
              text, 
              value: text,
              frameId: targetEl.frameId || startFrameId 
            } as InputAction);
            break;
          }
          
          case 'press_key': {
            const key = step.key || 'Enter';
            await this.executeCDPKey({ frameId: startFrameId } as ResolvedElement, { 
              type: 'key', 
              key,
              frameId: startFrameId 
            } as InputAction);
            break;
          }
          
          case 'wait_selector': {
            const waitSelector = step.waitSelector;
            if (waitSelector) {
              await this.waitForSelector(sessionId, waitSelector, 8000);
            }
            break;
          }
          
          case 'scroll_by': {
            const dx = Number(step.dx ?? 0);
            const dy = Number(step.dy ?? 400);
            await this.cdp.sendCommand({ sessionId }, 'Input.dispatchMouseEvent', {
              type: 'mouseWheel',
              x: window.innerWidth / 2,
              y: window.innerHeight / 2,
              deltaX: dx,
              deltaY: dy
            });
            break;
          }
          
          default:
            console.warn(`[CDPRung] Unsupported batch operation: ${op}`);
            continue;
        }
        
        // Small delay between steps for more natural interaction
        await new Promise(r => setTimeout(r, 50 + Math.random() * 100));
      }
      
      return { 
        success: true, 
        meta: { 
          stepsExecuted: steps.length,
          batchMode: true 
        } 
      };
      
    } catch (error) {
      console.error('[CDPRung] Batch execution error:', error);
      return { 
        success: false, 
        error: String(error) 
      };
    }
  }

  /**
   * Execute action using Chrome DevTools Protocol
   */
  async execute(element: ResolvedElement, action: InputAction): Promise<boolean> {
    try {
      // Route to specific action handler
      switch (action.type) {
        case 'click': {
          const coords = await this.centerOf(element);
          const result = await this.executeCDPClick(coords, action);
          return result.success;
        }
        case 'fill': {
          const result = await this.executeCDPFill(element, action);
          return result.success;
        }
        case 'key': {
          const result = await this.executeCDPKey(element, action);
          return result.success;
        }
        case 'type_and_submit': {
          const result = await this.executeTypeAndSubmit(element, action);
          return result.success;
        }
        default:
          console.warn(`[CDPRung] Unsupported action type: ${action.type}`);
          return false;
      }
    } catch (error) {
      console.error('[CDPRung] Execution error:', error);
      return false;
    }
  }

  /**
   * Compute center point of element via DOM.getBoxModel
   */
  async centerOf(element: ResolvedElement): Promise<Point> {
    const { sessionId } = frameRouter.routeForFrame(element.frameId);
    const { frameOrdinal, backendNodeId } = parseEncodedId(element.encoded_id);
    
    const model = await this.cdp.sendCommand<any>({ sessionId }, 'DOM.getBoxModel', {
      backendNodeId: backendNodeId
    });
    
    const quad = (model?.model?.content as number[]) ?? model?.model?.border;
    if (!quad || quad.length < 8) {
      throw new Error('No box model for element');
    }
    
    const xs = [quad[0], quad[2], quad[4], quad[6]];
    const ys = [quad[1], quad[3], quad[5], quad[7]];
    const x = (Math.min(...xs) + Math.max(...xs)) / 2;
    const y = (Math.min(...ys) + Math.max(...ys)) / 2;
    
    return { x, y };
  }

  /**
   * The new macro: focus → insertText → optional Enter → wait
   */
  async executeTypeAndSubmit(element: ResolvedElement, action: InputAction): Promise<ExecuteResult> {
    const { sessionId } = frameRouter.routeForFrame(element.frameId);
    
    try {
      // Enable minimal domains needed
      await this.cdp.enableDomains({ sessionId }, ['Page', 'Input', 'DOM']);

      // 1) Focus by trusted click
      const center = await this.centerOf(element);
      await this.executeCDPClick(center, { ...action, frameId: element.frameId });

      // 2) Insert text quickly using Input.insertText
      const text = action.text ?? action.value ?? '';
      if (text.length > 0) {
        await this.cdp.sendCommand({ sessionId }, 'Input.insertText', { text });
      }

      // 3) Submit if requested (default true for Search mode)
      const doSubmit = action.submit !== false;
      if (doSubmit) {
        await this.cdp.sendCommand({ sessionId }, 'Input.dispatchKeyEvent', {
          type: 'keyDown',
          key: 'Enter',
          code: 'Enter',
          windowsVirtualKeyCode: 13,
          nativeVirtualKeyCode: 13
        });
        
        await new Promise(r => setTimeout(r, 40));
        
        await this.cdp.sendCommand({ sessionId }, 'Input.dispatchKeyEvent', {
          type: 'keyUp',
          key: 'Enter',
          code: 'Enter',
          windowsVirtualKeyCode: 13,
          nativeVirtualKeyCode: 13
        });
      }

      // 4) Wait according to policy
      const wait: ActionWait = action.wait ?? 'navigation';
      await this.waitAccordingToPolicy({ sessionId }, wait, action.waitSelector);

      return { success: true, meta: { submitted: doSubmit } };
    } catch (error) {
      console.error('[CDPRung] Type and submit error:', error);
      return { success: false, error: String(error) };
    }
  }

  async executeCDPClick(coords: Point, action: InputAction): Promise<ExecuteResult> {
    const { sessionId } = frameRouter.routeForFrame(action.frameId);
    
    try {
      const button = action.options?.button || 'left';
      const modifiers = this.getModifierMask(action.options?.modifiers || []);

      // Move to position first for more natural interaction
      await this.cdp.sendCommand({ sessionId }, 'Input.dispatchMouseEvent', {
        type: 'mouseMoved',
        x: coords.x,
        y: coords.y,
        buttons: 1
      });

      // Press
      await this.cdp.sendCommand({ sessionId }, 'Input.dispatchMouseEvent', {
        type: 'mousePressed',
        x: coords.x,
        y: coords.y,
        button: button,
        clickCount: 1,
        modifiers: modifiers
      });

      // Small delay for realism
      await new Promise(r => setTimeout(r, 40 + Math.random() * 40));

      // Release
      await this.cdp.sendCommand({ sessionId }, 'Input.dispatchMouseEvent', {
        type: 'mouseReleased',
        x: coords.x,
        y: coords.y,
        button: button,
        clickCount: 1,
        modifiers: modifiers
      });

      return { success: true };
    } catch (error) {
      console.error('[CDPRung] Click execution error:', error);
      return { success: false, error: String(error) };
    }
  }

  async executeCDPFill(element: ResolvedElement, action: InputAction): Promise<ExecuteResult> {
    const { sessionId } = frameRouter.routeForFrame(element.frameId);
    
    try {
      // Focus by trusted click at center
      const center = await this.centerOf(element);
      await this.executeCDPClick(center, { ...action, frameId: element.frameId });

      // Insert text fast using Input.insertText (single CDP call)
      const text = action.value || action.text || '';
      if (text.length > 0) {
        await this.cdp.sendCommand({ sessionId }, 'Input.insertText', { text });
      }
      
      return { success: true };
    } catch (error) {
      console.error('[CDPRung] Fill execution error:', error);
      return { success: false, error: String(error) };
    }
  }

  async executeCDPKey(element: ResolvedElement, action: InputAction): Promise<ExecuteResult> {
    const { sessionId } = frameRouter.routeForFrame(element.frameId);
    
    try {
      const key = action.key || 'Enter';
      const code = this.getKeyCode(key);
      const modifiers = this.getModifierMask(action.options?.modifiers || []);
      const vk = key === 'Enter' ? 13 : key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0;

      // Focus element first
      const center = await this.centerOf(element);
      await this.executeCDPClick(center, { ...action, frameId: element.frameId });

      // Dispatch key events
      await this.cdp.sendCommand({ sessionId }, 'Input.dispatchKeyEvent', {
        type: 'keyDown',
        key,
        code,
        windowsVirtualKeyCode: vk,
        nativeVirtualKeyCode: vk,
        modifiers: modifiers
      });

      await new Promise(r => setTimeout(r, 40));

      await this.cdp.sendCommand({ sessionId }, 'Input.dispatchKeyEvent', {
        type: 'keyUp',
        key,
        code,
        windowsVirtualKeyCode: vk,
        nativeVirtualKeyCode: vk,
        modifiers: modifiers
      });

      return { success: true };
    } catch (error) {
      console.error('[CDPRung] Key execution error:', error);
      return { success: false, error: String(error) };
    }
  }

  private async executeCDPHover(coordinates: {x: number; y: number}, action: InputAction): Promise<boolean> {
    if (!this.cdpClient) return false;

    try {
      // Move mouse to element with realistic path
      const steps = 5;
      const startX = coordinates.x - 100 + Math.random() * 200;
      const startY = coordinates.y - 100 + Math.random() * 200;

      for (let i = 0; i <= steps; i++) {
        const progress = i / steps;
        const currentX = startX + (coordinates.x - startX) * progress;
        const currentY = startY + (coordinates.y - startY) * progress;

        await this.cdpClient.sendCommand('Input.dispatchMouseEvent', {
          type: 'mouseMoved',
          x: currentX,
          y: currentY
        });

        await this.delay(50 + Math.random() * 50);
      }

      return true;
    } catch (error) {
      console.error('[CDPRung] Hover execution error:', error);
      return false;
    }
  }

  private async executeCDPScroll(element: ResolvedElement, action: InputAction): Promise<boolean> {
    if (!this.cdpClient) return false;

    try {
      const coordinates = await this.getElementCoordinates(element);
      if (!coordinates) return false;

      // Calculate scroll delta to bring element into view
      const viewportHeight = window.innerHeight;
      const targetY = coordinates.y - viewportHeight / 2;
      const currentScrollY = window.scrollY;
      const deltaY = targetY - currentScrollY;

      // Dispatch wheel events to scroll
      const scrollSteps = Math.max(5, Math.abs(deltaY) / 100);
      const stepSize = deltaY / scrollSteps;

      for (let i = 0; i < scrollSteps; i++) {
        await this.cdpClient.sendCommand('Input.dispatchMouseEvent', {
          type: 'mouseWheel',
          x: coordinates.x,
          y: coordinates.y,
          deltaX: 0,
          deltaY: -stepSize // Negative for scrolling down
        });

        await this.delay(16); // ~60fps
      }

      return true;
    } catch (error) {
      console.error('[CDPRung] Scroll execution error:', error);
      return false;
    }
  }

  // Helper methods

  private mapButtonToCDP(button: string): string {
    switch (button) {
      case 'left': return 'left';
      case 'right': return 'right';
      case 'middle': return 'middle';
      default: return 'left';
    }
  }

  private getModifierMask(modifiers: string[]): number {
    let mask = 0;
    if (modifiers.includes('alt')) mask |= 1;
    if (modifiers.includes('ctrl')) mask |= 2;
    if (modifiers.includes('meta')) mask |= 4;
    if (modifiers.includes('shift')) mask |= 8;
    return mask;
  }

  private getKeyCode(key: string): string {
    // Map common keys to their codes
    const keyCodeMap: Record<string, string> = {
      'Enter': 'Enter',
      'Tab': 'Tab',
      'Escape': 'Escape',
      ' ': 'Space',
      'Delete': 'Delete',
      'Backspace': 'Backspace',
      'ArrowUp': 'ArrowUp',
      'ArrowDown': 'ArrowDown',
      'ArrowLeft': 'ArrowLeft',
      'ArrowRight': 'ArrowRight'
    };

    if (keyCodeMap[key]) {
      return keyCodeMap[key];
    }

    // For letter keys
    if (/^[a-zA-Z]$/.test(key)) {
      return `Key${key.toUpperCase()}`;
    }

    // For number keys
    if (/^[0-9]$/.test(key)) {
      return `Digit${key}`;
    }

    return key;
  }

  /**
   * Wait according to action policy
   */
  private async waitAccordingToPolicy(target: { sessionId: string }, wait: ActionWait, selector?: string) {
    switch (wait) {
      case 'networkIdle':
        await this.waitForNetworkIdle(target.sessionId, 500, 5000);
        break;
      case 'selector':
        if (selector) await this.waitForSelector(target.sessionId, selector, 5000);
        break;
      case 'navigation':
      default:
        await this.waitForLoadOrDOMContent(target.sessionId, 8000);
    }
  }

  private async waitForLoadOrDOMContent(sessionId: string, timeoutMs: number) {
    let resolved = false;
    const timer = setTimeout(() => { if (!resolved) resolved = true; }, timeoutMs);
    
    // Simple poll for document.readyState via Runtime.evaluate
    while (!resolved) {
      try {
        const res = await this.cdp.sendCommand<any>({ sessionId }, 'Runtime.evaluate', {
          expression: 'document.readyState',
          returnByValue: true
        });
        const state = res?.result?.value;
        if (state === 'interactive' || state === 'complete') {
          resolved = true;
          break;
        }
      } catch (e) {
        // Ignore errors, continue polling
      }
      await new Promise(r => setTimeout(r, 150));
    }
    clearTimeout(timer);
  }

  private async waitForNetworkIdle(sessionId: string, idleMs: number, capMs: number) {
    let lastActivity = Date.now();
    
    try {
      await this.cdp.sendCommand({ sessionId }, 'Network.enable', {});
      
      const end = Date.now() + capMs;
      while (Date.now() < end) {
        if (Date.now() - lastActivity >= idleMs) break;
        await new Promise(r => setTimeout(r, 100));
      }
      
      await this.cdp.sendCommand({ sessionId }, 'Network.disable', {});
    } catch (e) {
      console.warn('Network idle wait failed:', e);
    }
  }

  private async waitForSelector(sessionId: string, selector: string, timeoutMs: number) {
    const end = Date.now() + timeoutMs;
    while (Date.now() < end) {
      try {
        const found = await this.cdp.sendCommand<any>({ sessionId }, 'Runtime.evaluate', {
          expression: `!!document.querySelector(${JSON.stringify(selector)})`,
          returnByValue: true
        });
        if (found?.result?.value === true) return;
      } catch (e) {
        // Ignore errors, continue polling
      }
      await new Promise(r => setTimeout(r, 100));
    }
    throw new Error(`Timeout waiting for selector: ${selector}`);
  }

  private delay(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
  }
}
