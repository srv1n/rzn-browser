// CDP Helper for RZN - Minimal CDP access via chrome.debugger
// Inspired by the iframe handling found in public reference projects

type CDPEvent = { method: string; params: any; sessionId?: string };

type FrameRoute = {
  sessionId: string;             // CDP session to talk to this frame
  targetId: string;              // DevTools target owning the frame
  parentSessionId?: string;      // parent session (flattened tree)
};

export class CDP {
  private sessions = new Map<number, { attached: boolean; domains: Set<string> }>();
  private frameRoutes = new Map<string, FrameRoute>(); // frameId -> route
  private tabListeners = new Map<number, (source: any, method: string, params: any) => void>();

  // Attach to a tab on demand - stealth mode, no remote debugging port
  async attach(tabId: number) {
    if (this.sessions.get(tabId)?.attached) return;
    
    console.log(`[CDP] Attaching to tab ${tabId}`);
    await chrome.debugger.attach({ tabId }, '1.3');
    this.sessions.set(tabId, { attached: true, domains: new Set() });

    // Listen to events for OOPIF routing
    const onEvent = (_src: any, method: string, params: any) => {
      // Maintain Target auto-attach routing for cross-origin iframes
      if (method === 'Target.attachedToTarget') {
        const { sessionId, targetInfo } = params;
        this.frameRoutes.set(targetInfo.targetId, {
          sessionId,
          targetId: targetInfo.targetId,
        });
      }
      if (method === 'Target.detachedFromTarget') {
        const { sessionId } = params;
        for (const [k, v] of this.frameRoutes) {
          if (v.sessionId === sessionId) this.frameRoutes.delete(k);
        }
      }
    };

    chrome.debugger.onEvent.addListener(onEvent);
    this.tabListeners.set(tabId, onEvent as any);

    // Enable flattened auto-attach for OOPIF sessions
    await this.send(tabId, 'Target.setAutoAttach', {
      autoAttach: true,
      waitForDebuggerOnStart: false,
      flatten: true, // Critical for iframe handling
    });

    // Minimal defaults - only what we need
    await this.enable(tabId, ['Page', 'DOM', 'Accessibility']);
  }

  async detach(tabId: number) {
    const s = this.sessions.get(tabId);
    if (!s?.attached) return;
    
    console.log(`[CDP] Detaching from tab ${tabId}`);
    // Best-effort disable to reduce fingerprints
    try { 
      await this.disable(tabId, Array.from(s.domains)); 
    } catch {}
    
    try { 
      await chrome.debugger.detach({ tabId }); 
    } catch {}
    
    this.sessions.delete(tabId);

    const l = this.tabListeners.get(tabId);
    if (l) chrome.debugger.onEvent.removeListener(l);
    this.tabListeners.delete(tabId);
    this.frameRoutes.clear();
  }

  // Scoped attach/detach for minimal exposure
  async with<T>(tabId: number, fn: () => Promise<T>, keepAttachedMs = 0): Promise<T> {
    await this.attach(tabId);
    try {
      const result = await fn();
      if (keepAttachedMs > 0) {
        setTimeout(() => this.detach(tabId), keepAttachedMs);
      } else {
        await this.detach(tabId);
      }
      return result;
    } catch (e) {
      // Ensure detach on error
      try { await this.detach(tabId); } catch {}
      throw e;
    }
  }

  async enable(tabId: number, domains: string[]) {
    const s = this.sessions.get(tabId);
    if (!s) throw new Error('CDP not attached');
    
    for (const d of domains) {
      if (s.domains.has(d)) continue;
      // Skip Console domain to avoid detection
      if (d === 'Console') {
        console.warn('[CDP] Skipping Console domain for stealth');
        continue;
      }
      await this.send(tabId, `${d}.enable`, {});
      s.domains.add(d);
    }
  }

  async disable(tabId: number, domains: string[]) {
    for (const d of domains) {
      try { 
        await this.send(tabId, `${d}.disable`, {}); 
      } catch {}
    }
  }

  // Send command with optional session routing for frames
  async send<T = any>(
    tabId: number,
    method: string,
    params?: Record<string, any>,
    paramsEnvelope?: { sessionId?: string }
  ): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const commandParams = Object.assign({}, paramsEnvelope || {}, params || {});
      chrome.debugger.sendCommand(
        { tabId },
        method,
        commandParams,
        (result) => {
          const err = chrome.runtime.lastError;
          if (err) return reject(new Error(err.message));
          resolve(result as T);
        }
      );
    });
  }

  // Resolve session route for a frame
  routeForFrame(frameId?: string): { sessionId?: string } {
    if (!frameId) return {};
    const route = this.frameRoutes.get(frameId);
    return route ? { sessionId: route.sessionId } : {};
  }

  // Check if CDP is attached to a tab
  isAttached(tabId: number): boolean {
    return this.sessions.get(tabId)?.attached || false;
  }
}

export const cdp = new CDP();
