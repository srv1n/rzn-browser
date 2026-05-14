// CDP Frame Router - CRITICAL infrastructure for cross-origin iframe support
// Based on Sam's feedback: Target.setAutoAttach with flatten=true enables OOPIF support
import { isExpectedCdpLifecycleError } from './errors';
import { getActiveBrowserTabId } from '../browserTabs';

export interface FrameInfo {
  frameId: string;
  sessionId?: string;
  parentFrameId?: string;
  url?: string;
  securityOrigin?: string;
  mimeType?: string;
}

export interface TargetInfo {
  targetId: string;
  type: string;
  title?: string;
  url?: string;
  attached?: boolean;
  canAccessOpener?: boolean;
  openerFrameId?: string;
  browserContextId?: string;
  openerId?: string;
}

interface FrameRoute {
  sessionId: string;
  targetId: string;
  parentSessionId?: string;
  frameInfo?: FrameInfo;
}

interface ExecutionContext {
  id: number;
  origin: string;
  name: string;
  uniqueId: string;
  auxData?: any;
}

interface SessionInfo { 
  sessionId: string; 
  targetId: string; 
  parentSessionId?: string; 
  type?: string; 
}

export class FrameRouter {
  private routes = new Map<string, FrameRoute>(); // frameId -> route
  private targetSessions = new Map<string, string>(); // targetId -> sessionId
  private contextMap = new Map<number, ExecutionContext>(); // contextId -> context
  private attachedTabs = new Set<number>();
  private tabSessions = new Map<number, string>(); // tabId -> root sessionId
  private sessions = new Map<string, SessionInfo>(); // sessionId -> info
  private frameToSession = new Map<string, string>(); // frameId -> sessionId
  
  // Event listeners for cleanup
  private eventListeners = new Map<number, (source: any, method: string, params: any) => void>();
  private readonly debuggerDetachListener = (source: chrome.debugger.Debuggee, reason: string) => {
    if (source.tabId === undefined) return;
    this.markTabDetached(source.tabId, reason || 'debugger detached');
  };

  constructor() {
    if (typeof chrome !== 'undefined' && chrome.debugger?.onDetach) {
      chrome.debugger.onDetach.addListener(this.debuggerDetachListener);
    }
  }

  /**
   * Attach CDP to tab with OOPIF support
   * KEY: Target.setAutoAttach with flatten=true for cross-origin frames
   */
  async attachToTab(tabId: number): Promise<void> {
    if (this.attachedTabs.has(tabId)) {
      console.log(`[FrameRouter] Tab ${tabId} already attached`);
      return;
    }

    console.log(`[FrameRouter] Attaching to tab ${tabId}`);
    
    try {
      // Avoid attaching to internal / restricted pages (CDP attach will fail).
      // This commonly happens if the active tab is chrome://extensions or a newtab page.
      try {
        const tab = await chrome.tabs.get(tabId);
        const url = tab?.url || '';
        if (/^(chrome|edge|about|devtools|chrome-extension):\/\//.test(url)) {
          console.warn(`[FrameRouter] Skipping CDP attach for restricted URL: ${url}`);
          return;
        }
      } catch {
        // If tab lookup fails, we still attempt to attach; chrome.debugger will provide a clear error.
      }

      // Attach to debugger
      await chrome.debugger.attach({ tabId }, '1.3');
      
      // Create root session tracking
      const rootSessionId = `root:${tabId}`;
      this.tabSessions.set(tabId, rootSessionId);
      this.sessions.set(rootSessionId, { 
        sessionId: rootSessionId, 
        targetId: `tab:${tabId}` 
      });
      
      // Set up event listener for this tab
      const eventListener = this.createEventListener(tabId);
      chrome.debugger.onEvent.addListener(eventListener);
      this.eventListeners.set(tabId, eventListener);
      
      // CRITICAL: Enable auto-attach with flatten=true for OOPIF support
      await this.sendCommand(tabId, 'Target.setAutoAttach', {
        autoAttach: true,
        waitForDebuggerOnStart: false,
        flatten: true // This is the key for cross-origin iframe support
      });
      
      // Enable required domains (Target.enable is optional in some protocol versions)
      try { await this.sendCommand(tabId, 'Page.enable', {}); } catch {}
      try { await this.sendCommand(tabId, 'Runtime.enable', {}); } catch {}
      
      this.attachedTabs.add(tabId);
      console.log(`[FrameRouter] Successfully attached to tab ${tabId}`);
      
    } catch (error: any) {
      const msg = (error && error.message) ? String(error.message) : String(error);
      // If another debugger is attached, do not loop/throw; mark as not attached and return
      if (msg.includes('Another debugger')) {
        console.warn(`[FrameRouter] CDP attach unavailable for tab ${tabId}: ${msg}`);
        this.markTabDetached(tabId, msg);
        return;
      }
      // Common non-fatal case: trying to attach to internal pages (chrome://, etc.)
      if (msg.includes('chrome://') || msg.includes('Cannot access a chrome://')) {
        console.warn(`[FrameRouter] CDP attach skipped for tab ${tabId}: ${msg}`);
        this.markTabDetached(tabId, msg);
        return;
      }
      // If protocol method not found (e.g., Target.enable), we already skipped it above
      if (msg.includes("wasn't found") || msg.includes('not found')) {
        console.warn(`[FrameRouter] CDP attach skipped for tab ${tabId}: ${msg}`);
        this.markTabDetached(tabId, msg);
        return;
      }
      if (isExpectedCdpLifecycleError(msg)) {
        console.warn(`[FrameRouter] CDP attach lost target for tab ${tabId}: ${msg}`);
        this.markTabDetached(tabId, msg);
        return;
      }
      console.error(`[FrameRouter] Failed to attach to tab ${tabId}:`, msg);
      this.markTabDetached(tabId, msg);
      throw error;
    }
  }

  /**
   * Ensure CDP is attached for a specific frame
   */
  async ensureAttachedForFrame(frameId?: string): Promise<void> {
    const tabId = await getActiveBrowserTabId('frame attachment');
    await this.attachToTab(tabId);
    
    // If frameId is provided, ensure we have routing for it
    if (frameId && !this.frameToSession.has(frameId)) {
      // Frame mapping will be populated by CDP events
      console.log(`[FrameRouter] Frame ${frameId} not yet mapped, will be populated by events`);
    }
  }

  /**
   * Detach CDP from tab and clean up resources
   */
  async detachFromTab(tabId: number): Promise<void> {
    if (!this.attachedTabs.has(tabId)) {
      return;
    }

    console.log(`[FrameRouter] Detaching from tab ${tabId}`);

    try {
      // Clean up event listener
      const eventListener = this.eventListeners.get(tabId);
      if (eventListener) {
        chrome.debugger.onEvent.removeListener(eventListener);
        this.eventListeners.delete(tabId);
      }
      
      // Detach debugger
      await chrome.debugger.detach({ tabId });

    } catch (error: any) {
      const msg = (error && error.message) ? String(error.message) : String(error);
      const benign =
        msg.includes('Debugger is not attached') ||
        msg.includes('No tab with given id') ||
        msg.includes('Cannot access a chrome://') ||
        msg.includes('Detached while handling command');
      if (!benign) {
        console.error(`[FrameRouter] Error detaching from tab ${tabId}:`, error);
      }
    } finally {
      // Even if Chrome already dropped the debugger session, clear our local state so the
      // next attach starts cleanly and reload-time cleanup does not surface noisy errors.
      this.clearTabRoutes(tabId);
      this.attachedTabs.delete(tabId);
      this.tabSessions.delete(tabId);
    }
  }

  markTabDetached(tabId: number, reason = 'detached'): void {
    console.warn(`[FrameRouter] Marking tab ${tabId} detached: ${reason}`);

    const eventListener = this.eventListeners.get(tabId);
    if (eventListener && typeof chrome !== 'undefined' && chrome.debugger?.onEvent) {
      try {
        chrome.debugger.onEvent.removeListener(eventListener);
      } catch {}
    }
    this.eventListeners.delete(tabId);
    this.clearTabRoutes(tabId);
    this.attachedTabs.delete(tabId);
    this.tabSessions.delete(tabId);
  }

  /**
   * Get sessionId for routing commands to specific frame
   * This is the core routing functionality
   */
  routeForFrame(frameId?: string): { sessionId: string } {
    if (!frameId) {
      // Return root session for main frame
      // This is a synchronous fallback - in practice, ensureAttachedForFrame should be called first
      return { sessionId: 'root:unknown' };
    }
    
    const sessionId = this.frameToSession.get(frameId);
    if (sessionId) {
      console.log(`[FrameRouter] Routing frameId ${frameId} -> sessionId ${sessionId}`);
      return { sessionId };
    }
    
    // Fallback to root session if frame not mapped yet
    console.warn(`[FrameRouter] No route found for frameId: ${frameId}, using root session`);
    return { sessionId: 'root:unknown' };
  }

  /**
   * Get all frames with their routing information
   */
  async getFrameTree(tabId: number): Promise<FrameInfo[]> {
    if (!this.attachedTabs.has(tabId)) {
      throw new Error(`Tab ${tabId} not attached to CDP`);
    }

    try {
      const result = await this.sendCommand(tabId, 'Page.getFrameTree', {});
      const frames: FrameInfo[] = [];
      
      // Traverse frame tree and collect frame info
      const traverse = (node: any, parentFrameId?: string) => {
        const frame: FrameInfo = {
          frameId: node.frame.id,
          parentFrameId,
          url: node.frame.url,
          securityOrigin: node.frame.securityOrigin,
          mimeType: node.frame.mimeType,
          sessionId: this.routes.get(node.frame.id)?.sessionId
        };
        
        frames.push(frame);
        
        // Process child frames
        if (node.childFrames) {
          for (const child of node.childFrames) {
            traverse(child, node.frame.id);
          }
        }
      };
      
      traverse(result.frameTree);
      return frames;
      
    } catch (error) {
      console.error(`[FrameRouter] Failed to get frame tree for tab ${tabId}:`, error);
      throw error;
    }
  }

  /**
   * Check if tab is attached to CDP
   */
  isAttachedToTab(tabId: number): boolean {
    return this.attachedTabs.has(tabId);
  }

  /**
   * Get all attached tabs
   */
  getAttachedTabs(): number[] {
    return Array.from(this.attachedTabs);
  }

  /**
   * Get frame sessions for a specific tab (for AX slice iteration)
   */
  getFrameSessionsForTab(tabId: number): Array<{ frameId: string; sessionId: string }> {
    const result: Array<{ frameId: string; sessionId: string }> = [];
    
    // Add root session first
    const rootSession = this.tabSessions.get(tabId);
    if (rootSession) {
      result.push({ frameId: 'main', sessionId: rootSession });
    }
    
    // Add frame-specific sessions
    for (const [frameId, sessionId] of this.frameToSession.entries()) {
      // Filter to sessions belonging to this tab (rough heuristic)
      const sessionInfo = this.sessions.get(sessionId);
      if (sessionInfo && (
        sessionInfo.targetId.includes(`tab:${tabId}`) || 
        sessionId === rootSession ||
        sessionInfo.parentSessionId === rootSession
      )) {
        result.push({ frameId, sessionId });
      }
    }
    
    // Remove duplicates based on sessionId
    const seen = new Set<string>();
    return result.filter(item => {
      if (seen.has(item.sessionId)) {
        return false;
      }
      seen.add(item.sessionId);
      return true;
    });
  }

  /**
   * Send CDP command with optional session routing
   */
  private async sendCommand<T = any>(
    tabId: number,
    method: string,
    params: any = {},
    sessionId?: string
  ): Promise<T> {
    return new Promise((resolve, reject) => {
      const commandParams = sessionId ? { ...params, sessionId } : params;
      
      chrome.debugger.sendCommand(
        { tabId },
        method,
        commandParams,
        (result) => {
          const error = chrome.runtime.lastError;
          if (error) {
            reject(new Error(`CDP command failed: ${error.message}`));
          } else {
            resolve(result as T);
          }
        }
      );
    });
  }

  /**
   * Create event listener for a specific tab
   */
  private createEventListener(tabId: number) {
    return (source: any, method: string, params: any) => {
      // Only handle events from our attached tab
      if (source.tabId !== tabId) return;
      
      this.handleCDPEvent(tabId, method, params);
    };
  }

  /**
   * Handle CDP events to maintain frame routing
   */
  private handleCDPEvent(tabId: number, method: string, params: any): void {
    console.log(`[FrameRouter] Event: ${method}`, params);
    
    switch (method) {
      case 'Target.attachedToTarget':
        this.handleTargetAttached(params, tabId);
        break;
        
      case 'Target.detachedFromTarget':
        this.handleTargetDetached(params);
        break;
        
      case 'Runtime.executionContextCreated':
        this.handleExecutionContextCreated(params);
        break;
        
      case 'Runtime.executionContextDestroyed':
        this.handleExecutionContextDestroyed(params);
        break;
        
      case 'Page.frameAttached':
      case 'Page.frameNavigated':
        this.handleFrameEvent(params, tabId);
        break;
        
      case 'Page.frameDetached':
        this.handleFrameDetached(params);
        break;
    }
  }

  /**
   * Handle Target.attachedToTarget - critical for OOPIF routing
   */
  private handleTargetAttached(params: any, tabId: number): void {
    const { sessionId, targetInfo } = params;
    
    console.log(`[FrameRouter] Target attached: ${targetInfo.targetId} -> session ${sessionId}`);
    
    // Store session info
    this.sessions.set(sessionId, {
      sessionId,
      targetId: targetInfo.targetId,
      parentSessionId: params.sessionId,
      type: targetInfo.type
    });
    
    // Map targetId to sessionId
    this.targetSessions.set(targetInfo.targetId, sessionId);
    
    // If this is a frame target, we need to map it to frameId when we get frame info
    if (targetInfo.type === 'page' || targetInfo.type === 'iframe') {
      console.log(`[FrameRouter] Frame target detected: ${targetInfo.url}`);
    }
  }
  
  /**
   * Handle Page.frameAttached and Page.frameNavigated
   */
  private handleFrameEvent(params: any, tabId: number): void {
    const frameId = params.frame?.id ?? params.frameId;
    if (!frameId) return;
    
    // Map frame to session (use root session for main frame, or specific session for OOPIF)
    const sessionId = params.sessionId || this.tabSessions.get(tabId) || `root:${tabId}`;
    
    console.log(`[FrameRouter] Mapping frame ${frameId} -> session ${sessionId}`);
    this.frameToSession.set(frameId, sessionId);
    
    // Update routes map as well for compatibility
    this.routes.set(frameId, {
      sessionId,
      targetId: `frame:${frameId}`,
      frameInfo: {
        frameId,
        url: params.frame?.url,
        securityOrigin: params.frame?.securityOrigin
      }
    });
  }

  /**
   * Handle Target.detachedFromTarget
   */
  private handleTargetDetached(params: any): void {
    const { sessionId, targetId } = params;
    
    console.log(`[FrameRouter] Target detached: ${targetId || 'unknown'} (session ${sessionId})`);
    
    // Remove from target sessions
    if (targetId) {
      this.targetSessions.delete(targetId);
    }
    
    // Find and remove routes using this sessionId
    for (const [frameId, route] of this.routes.entries()) {
      if (route.sessionId === sessionId) {
        console.log(`[FrameRouter] Removing route for frame ${frameId}`);
        this.routes.delete(frameId);
        this.frameToSession.delete(frameId);
      }
    }
  }

  /**
   * Handle Runtime.executionContextCreated - maps contexts to frames
   */
  private handleExecutionContextCreated(params: any): void {
    const { context } = params;
    
    // Store execution context
    this.contextMap.set(context.id, context);
    
    // If this context has auxData with frameId, create the route
    if (context.auxData && context.auxData.frameId) {
      const frameId = context.auxData.frameId;
      const sessionId = params.sessionId; // This comes from the session that created the context
      
      if (sessionId) {
        console.log(`[FrameRouter] Mapping frame ${frameId} -> session ${sessionId}`);
        
        this.routes.set(frameId, {
          sessionId,
          targetId: context.auxData.targetId || 'unknown',
          frameInfo: {
            frameId,
            url: context.origin,
            securityOrigin: context.origin
          }
        });
      }
    }
  }

  /**
   * Handle Runtime.executionContextDestroyed
   */
  private handleExecutionContextDestroyed(params: any): void {
    const { executionContextId } = params;
    this.contextMap.delete(executionContextId);
  }

  /**
   * Handle Page.frameNavigated
   */
  private handleFrameNavigated(params: any): void {
    const { frame } = params;
    
    // Update frame info in existing route
    const route = this.routes.get(frame.id);
    if (route && route.frameInfo) {
      route.frameInfo.url = frame.url;
      route.frameInfo.securityOrigin = frame.securityOrigin;
      route.frameInfo.mimeType = frame.mimeType;
    }
  }

  /**
   * Handle Page.frameDetached
   */
  private handleFrameDetached(params: any): void {
    const { frameId } = params;
    
    console.log(`[FrameRouter] Frame detached: ${frameId}`);
    this.routes.delete(frameId);
    this.frameToSession.delete(frameId);
  }

  /**
   * Clear all routes for a tab (cleanup helper)
   */
  private clearTabRoutes(tabId: number): void {
    // Clear frame mappings for this tab
    const rootSession = this.tabSessions.get(tabId);
    if (rootSession) {
      this.sessions.delete(rootSession);
    }
    
    // Remove frame mappings that belong to this tab's sessions
    const tabSessions = new Set([rootSession]);
    for (const [sessionId, info] of this.sessions.entries()) {
      if (info.targetId.startsWith(`tab:${tabId}`) || tabSessions.has(info.parentSessionId)) {
        tabSessions.add(sessionId);
        this.sessions.delete(sessionId);
      }
    }
    
    // Clean up frame-to-session mappings
    for (const [frameId, sessionId] of this.frameToSession.entries()) {
      if (tabSessions.has(sessionId)) {
        this.frameToSession.delete(frameId);
        this.routes.delete(frameId);
      }
    }
    
    // Clean up other maps
    for (const [targetId, sessionId] of this.targetSessions.entries()) {
      if (tabSessions.has(sessionId)) {
        this.targetSessions.delete(targetId);
      }
    }
    
    this.contextMap.clear();
  }
}

// Singleton instance
export const frameRouter = new FrameRouter();
