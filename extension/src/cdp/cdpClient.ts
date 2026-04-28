// CDP Client - Thin wrapper for chrome.debugger commands
// Provides type-safe CDP command execution with session routing

import { frameRouter } from './frameRouter';

export interface CDPCommand {
  method: string;
  params?: any;
  sessionId?: string;
}

export interface CDPResult<T = any> {
  result: T;
  error?: {
    code: number;
    message: string;
    data?: any;
  };
}

export interface CDPTarget {
  tabId?: number;
  sessionId?: string;
}

/**
 * Type-safe CDP client with automatic frame routing
 */
export class CDPClient {
  private domainRefs = new Map<string, Map<string, number>>(); // sessionId -> domain -> refcount

  private formatCommandError(error: unknown): string {
    if (typeof error === 'string') {
      return error;
    }
    if (!error || typeof error !== 'object') {
      return String(error);
    }

    const anyError = error as {
      message?: unknown;
      code?: unknown;
      data?: unknown;
      stack?: unknown;
    };
    const parts: string[] = [];

    if (typeof anyError.message === 'string' && anyError.message.trim()) {
      parts.push(anyError.message.trim());
    }
    if (typeof anyError.code === 'number') {
      parts.push(`code=${anyError.code}`);
    }
    if (anyError.data !== undefined) {
      try {
        const dataText =
          typeof anyError.data === 'string' ? anyError.data : JSON.stringify(anyError.data);
        if (dataText) {
          parts.push(`data=${dataText}`);
        }
      } catch {}
    }

    if (!parts.length) {
      try {
        return JSON.stringify(error);
      } catch {
        return String(error);
      }
    }

    return parts.join(' ');
  }

  /**
   * Send CDP command to target with automatic session routing
   */
  async sendCommand<T = any>(
    target: CDPTarget,
    method: string,
    params?: any,
    options?: {
      frameId?: string;
      sessionId?: string;
      timeout?: number;
    }
  ): Promise<T> {
    const { tabId, sessionId: targetSessionId } = target;
    const { frameId, sessionId: explicitSessionId, timeout = 30000 } = options || {};
    
    // Determine session routing
    let sessionId = explicitSessionId || targetSessionId;
    if (!sessionId && frameId) {
      const route = frameRouter.routeForFrame(frameId);
      sessionId = route.sessionId;
    }
    
    console.log(`[CDPClient] Sending ${method}${sessionId ? ` (session: ${sessionId})` : ''}`);
    
    return new Promise<T>((resolve, reject) => {
      const timeoutId = setTimeout(() => {
        reject(new Error(`CDP command timeout: ${method}`));
      }, timeout);
      
      // Prepare command parameters
      const commandParams = sessionId ? { ...params, sessionId } : params;
      
      // Determine which debugger target to use
      const debuggerTarget = tabId ? { tabId } : { extensionId: chrome.runtime.id };
      
      chrome.debugger.sendCommand(
        debuggerTarget,
        method,
        commandParams || {},
        (result) => {
          clearTimeout(timeoutId);
          
          const error = chrome.runtime.lastError;
          if (error) {
            const formatted = this.formatCommandError(error);
            console.error(`[CDPClient] Command failed: ${method}: ${formatted}`);
            reject(new Error(`CDP command failed: ${formatted}`));
          } else {
            console.log(`[CDPClient] Command succeeded: ${method}`);
            resolve(result as T);
          }
        }
      );
    });
  }

  /**
   * Enable CDP domains with ref-counting
   */
  async enableDomains(target: CDPTarget, domains: string[], frameId?: string): Promise<void> {
    const sessionId = target.sessionId ?? `tab:${target.tabId}`;
    console.log(`[CDPClient] Enabling domains: ${domains.join(', ')} for session ${sessionId}`);
    
    if (!this.domainRefs.has(sessionId)) {
      this.domainRefs.set(sessionId, new Map());
    }
    const refs = this.domainRefs.get(sessionId)!;
    
    // Never enable Console domain for stealth
    const filteredDomains = domains.filter(d => d !== 'Console');
    
    for (const domain of filteredDomains) {
      const count = (refs.get(domain) ?? 0) + 1;
      refs.set(domain, count);
      
      if (count === 1) {
        await this.sendCommand(target, `${domain}.enable`, {}, { frameId });
      }
    }
  }

  /**
   * Disable CDP domains with ref-counting
   */
  async disableDomains(target: CDPTarget, domains: string[], frameId?: string): Promise<void> {
    const sessionId = target.sessionId ?? `tab:${target.tabId}`;
    console.log(`[CDPClient] Disabling domains: ${domains.join(', ')} for session ${sessionId}`);
    
    const refs = this.domainRefs.get(sessionId);
    if (!refs) return;
    
    for (const domain of domains) {
      const count = (refs.get(domain) ?? 0) - 1;
      
      if (count <= 0) {
        refs.delete(domain);
        try {
          await this.sendCommand(target, `${domain}.disable`, {}, { frameId });
        } catch (error) {
          console.warn(`[CDPClient] Failed to disable ${domain}:`, error);
        }
      } else {
        refs.set(domain, count);
      }
    }
  }

  /**
   * Get document node for DOM operations
   */
  async getDocument(target: CDPTarget, options?: {
    depth?: number;
    pierce?: boolean;
    frameId?: string;
  }): Promise<any> {
    const { depth = -1, pierce = true, frameId } = options || {};
    
    return this.sendCommand(
      target,
      'DOM.getDocument',
      { depth, pierce },
      { frameId }
    );
  }

  /**
   * Query selector in specific frame
   */
  async querySelector(target: CDPTarget, nodeId: number, selector: string, frameId?: string): Promise<any> {
    return this.sendCommand(
      target,
      'DOM.querySelector',
      { nodeId, selector },
      { frameId }
    );
  }

  /**
   * Query all selectors in specific frame
   */
  async querySelectorAll(target: CDPTarget, nodeId: number, selector: string, frameId?: string): Promise<any> {
    return this.sendCommand(
      target,
      'DOM.querySelectorAll',
      { nodeId, selector },
      { frameId }
    );
  }

  /**
   * Get box model for element
   */
  async getBoxModel(target: CDPTarget, nodeId: number, frameId?: string): Promise<any> {
    return this.sendCommand(
      target,
      'DOM.getBoxModel',
      { nodeId },
      { frameId }
    );
  }

  /**
   * Get outer HTML of element
   */
  async getOuterHTML(target: CDPTarget, nodeId: number, frameId?: string): Promise<any> {
    return this.sendCommand(
      target,
      'DOM.getOuterHTML',
      { nodeId },
      { frameId }
    );
  }

  /**
   * Describe DOM node
   */
  async describeNode(target: CDPTarget, nodeId: number, options?: {
    depth?: number;
    pierce?: boolean;
    frameId?: string;
  }): Promise<any> {
    const { depth = 0, pierce = false, frameId } = options || {};
    
    return this.sendCommand(
      target,
      'DOM.describeNode',
      { nodeId, depth, pierce },
      { frameId }
    );
  }

  /**
   * Push backend node IDs to frontend
   */
  async pushNodesByBackendIds(target: CDPTarget, backendNodeIds: number[], frameId?: string): Promise<any> {
    return this.sendCommand(
      target,
      'DOM.pushNodesByBackendIdsToFrontend',
      { backendNodeIds },
      { frameId }
    );
  }

  /**
   * Get accessibility tree
   */
  async getAccessibilityTree(target: CDPTarget, options?: {
    nodeId?: number;
    backendNodeId?: number;
    objectId?: string;
    fetchRelatives?: boolean;
    frameId?: string;
  }): Promise<any> {
    const { nodeId, backendNodeId, objectId, fetchRelatives = false, frameId } = options || {};
    
    const params: any = { fetchRelatives };
    if (nodeId !== undefined) params.nodeId = nodeId;
    if (backendNodeId !== undefined) params.backendNodeId = backendNodeId;
    if (objectId !== undefined) params.objectId = objectId;
    
    return this.sendCommand(
      target,
      'Accessibility.getPartialAXTree',
      params,
      { frameId }
    );
  }

  /**
   * Get full accessibility tree
   */
  async getFullAccessibilityTree(target: CDPTarget, frameId?: string): Promise<any> {
    return this.sendCommand(
      target,
      'Accessibility.getFullAXTree',
      {},
      { frameId }
    );
  }

  /**
   * Get page frame tree
   */
  async getFrameTree(target: CDPTarget): Promise<any> {
    return this.sendCommand(target, 'Page.getFrameTree');
  }

  /**
   * Get layout metrics
   */
  async getLayoutMetrics(target: CDPTarget, frameId?: string): Promise<any> {
    return this.sendCommand(
      target,
      'Page.getLayoutMetrics',
      {},
      { frameId }
    );
  }

  /**
   * Take screenshot
   */
  async captureScreenshot(target: CDPTarget, options?: {
    format?: 'jpeg' | 'png' | 'webp';
    quality?: number;
    clip?: {
      x: number;
      y: number;
      width: number;
      height: number;
      scale?: number;
    };
    fromSurface?: boolean;
    captureBeyondViewport?: boolean;
  }): Promise<any> {
    return this.sendCommand(target, 'Page.captureScreenshot', options);
  }

  /**
   * Execute JavaScript in specific context
   */
  async evaluate(target: CDPTarget, expression: string, options?: {
    objectGroup?: string;
    includeCommandLineAPI?: boolean;
    silent?: boolean;
    contextId?: number;
    returnByValue?: boolean;
    generatePreview?: boolean;
    userGesture?: boolean;
    awaitPromise?: boolean;
    throwOnSideEffect?: boolean;
    timeout?: number;
    disableBreaks?: boolean;
    replMode?: boolean;
    allowUnsafeEvalBlockedByCSP?: boolean;
    uniqueContextId?: string;
    frameId?: string;
  }): Promise<any> {
    const params: any = { 
      expression,
      returnByValue: true,
      awaitPromise: true,
      ...options 
    };
    
    return this.sendCommand(
      target,
      'Runtime.evaluate',
      params,
      { frameId: options?.frameId }
    );
  }

  /**
   * Click element at coordinates
   */
  async click(target: CDPTarget, x: number, y: number, options?: {
    button?: 'left' | 'right' | 'middle';
    clickCount?: number;
    modifiers?: number;
    timestamp?: number;
  }): Promise<void> {
    const { button = 'left', clickCount = 1, modifiers = 0 } = options || {};
    
    // Convert button to CDP format
    const buttonMap = { left: 'left', right: 'right', middle: 'middle' };
    
    await this.sendCommand(target, 'Input.dispatchMouseEvent', {
      type: 'mousePressed',
      x,
      y,
      button: buttonMap[button],
      clickCount,
      modifiers
    });
    
    await this.sendCommand(target, 'Input.dispatchMouseEvent', {
      type: 'mouseReleased',
      x,
      y,
      button: buttonMap[button],
      clickCount,
      modifiers
    });
  }

  /**
   * Type text
   */
  async type(target: CDPTarget, text: string): Promise<void> {
    for (const char of text) {
      await this.sendCommand(target, 'Input.dispatchKeyEvent', {
        type: 'char',
        text: char
      });
    }
  }

  /**
   * Press key
   */
  async pressKey(target: CDPTarget, key: string, options?: {
    modifiers?: number;
    timestamp?: number;
  }): Promise<void> {
    const { modifiers = 0 } = options || {};
    
    await this.sendCommand(target, 'Input.dispatchKeyEvent', {
      type: 'keyDown',
      key,
      modifiers
    });
    
    await this.sendCommand(target, 'Input.dispatchKeyEvent', {
      type: 'keyUp',
      key,
      modifiers
    });
  }
}

// Singleton instance
export const cdpClient = new CDPClient();
