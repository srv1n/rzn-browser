// CDP Client - Thin wrapper for chrome.debugger commands
// Provides type-safe CDP command execution with session routing

import { frameRouter } from './frameRouter';
import { cdpErrorText, isExpectedCdpLifecycleError } from './errors';

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
  private domainRefs = new Map<string, Map<string, number>>(); // target/session -> domain -> refcount
  private domainOperations = new Map<string, Promise<void>>();

  private routeTarget(target: CDPTarget, frameId?: string, explicitSessionId?: string): CDPTarget {
    const sessionId = explicitSessionId || target.sessionId ||
      (frameId ? frameRouter.routeForFrame(frameId).sessionId : undefined);
    return { ...target, sessionId };
  }

  private domainKey(target: CDPTarget): string {
    return JSON.stringify([target.tabId ?? chrome.runtime.id, target.sessionId ?? null]);
  }

  // Serialize lease changes for one resolved session, not ordinary CDP commands
  // or other tabs. A failed operation must not poison the queue for later calls.
  private withDomainLock(key: string, run: () => Promise<void>): Promise<void> {
    const operation = (this.domainOperations.get(key) ?? Promise.resolve()).then(run);
    const tail = operation.then(() => {}, () => {});
    this.domainOperations.set(key, tail);
    void tail.then(() => {
      if (this.domainOperations.get(key) === tail) this.domainOperations.delete(key);
    });
    return operation;
  }

  private formatCommandError(error: unknown): string {
    return cdpErrorText(error);
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
    const { frameId, sessionId: explicitSessionId, timeout = 30000 } = options || {};
    const { tabId, sessionId } = this.routeTarget(target, frameId, explicitSessionId);

    console.log(`[CDPClient] Sending ${method}${sessionId ? ` (session: ${sessionId})` : ''}`);

    return new Promise<T>((resolve, reject) => {
      let settled = false;
      const timeoutId = setTimeout(() => {
        settled = true;
        reject(new Error(`CDP command timeout: ${method}`));
      }, timeout);

      // Flat-session routing belongs on DebuggerSession, not in CDP params.
      const debuggerTarget = {
        ...(tabId !== undefined ? { tabId } : { extensionId: chrome.runtime.id }),
        ...(sessionId ? { sessionId } : {}),
      };

      try {
        chrome.debugger.sendCommand(debuggerTarget, method, params || {}, (result) => {
          // Always consume lastError, even for late callbacks, to avoid Chrome's
          // unchecked-error warning. Late results must not invalidate a new lease.
          const error = chrome.runtime.lastError;
          if (settled) return;
          settled = true;
          clearTimeout(timeoutId);

          if (error) {
            const formatted = this.formatCommandError(error);
            const lifecycleError = isExpectedCdpLifecycleError(formatted);
            const message = `[CDPClient] Command failed: ${method}: ${formatted}`;
            if (lifecycleError) {
              console.warn(message);
              if (tabId !== undefined) {
                frameRouter.markTabDetached(tabId, formatted);
              }
            } else {
              console.error(message);
            }
            const commandError = new Error(`CDP command failed: ${formatted}`);
            (commandError as any).code = lifecycleError
              ? 'CDP_TARGET_DETACHED'
              : 'CDP_COMMAND_FAILED';
            reject(commandError);
          } else {
            console.log(`[CDPClient] Command succeeded: ${method}`);
            resolve(result as T);
          }
        });
      } catch (error) {
        settled = true;
        clearTimeout(timeoutId);
        reject(error);
      }
    });
  }

  /**
   * Enable CDP domains with ref-counting
   */
  async enableDomains(target: CDPTarget, domains: string[], frameId?: string): Promise<void> {
    // Never enable Console domain for stealth. Copy before entering the queue.
    const filteredDomains = domains.filter(d => d !== 'Console');
    if (!filteredDomains.length) return;
    // Resolve once: the accounting key and commands must address the same frame.
    const routed = this.routeTarget(target, frameId);
    const key = this.domainKey(routed);

    return this.withDomainLock(key, async () => {
      const refs = this.domainRefs.get(key) ?? new Map<string, number>();
      this.domainRefs.set(key, refs);
      const acquired: string[] = [];
      try {
        for (const domain of filteredDomains) {
          const count = refs.get(domain) ?? 0;
          if (count === 0) await this.sendCommand(routed, `${domain}.enable`, {});
          // Commit the reference only after Chrome confirms the enable.
          refs.set(domain, count + 1);
          acquired.push(domain);
        }
      } catch (error) {
        // A failed batch must release only the references it acquired, including
        // increments of domains that were already held by another caller.
        await this.releaseDomains(routed, refs, acquired.reverse());
        throw error;
      } finally {
        if (!refs.size) this.domainRefs.delete(key);
      }
    });
  }

  /**
   * Disable CDP domains with ref-counting
   */
  async disableDomains(target: CDPTarget, domains: string[], frameId?: string): Promise<void> {
    if (!domains.length) return;
    const routed = this.routeTarget(target, frameId);
    const key = this.domainKey(routed);
    const requested = domains.slice();
    return this.withDomainLock(key, async () => {
      const refs = this.domainRefs.get(key);
      if (!refs) return;
      await this.releaseDomains(routed, refs, requested);
      if (!refs.size) this.domainRefs.delete(key);
    });
  }

  private async releaseDomains(
    target: CDPTarget,
    refs: Map<string, number>,
    domains: string[],
  ): Promise<void> {
    for (const domain of domains) {
      const count = refs.get(domain);
      if (count === undefined) continue;
      if (count > 1) {
        refs.set(domain, count - 1);
      } else {
        refs.delete(domain);
        try {
          await this.sendCommand(target, `${domain}.disable`, {});
        } catch (error) {
          console.warn(`[CDPClient] Failed to disable ${domain}:`, error);
        }
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
    const { frameId, ...evaluateOptions } = options || {};
    const params: any = {
      expression,
      returnByValue: true,
      awaitPromise: true,
      ...evaluateOptions
    };
    
    return this.sendCommand(
      target,
      'Runtime.evaluate',
      params,
      { frameId, timeout: options?.timeout }
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
