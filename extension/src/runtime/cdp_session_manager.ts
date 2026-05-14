import { frameRouter } from '../cdp/frameRouter';
import { isExpectedCdpLifecycleError } from '../cdp/errors';

type SessionKey = string;

export interface CdpSessionHandle {
  sessionId: string;
  tabId: number;
  sendCommand<T = any>(method: string, params?: any): Promise<T>;
  release(): Promise<void>;
}

type CdpSessionRecord = {
  sessionId: string;
  tabId: number;
  acquiredAtMs: number;
  lastUsedAtMs: number;
};

function normalizeSessionId(raw: unknown): string {
  if (typeof raw === 'string' && raw.trim()) {
    return raw.trim();
  }
  return 'default';
}

function formatChromeDebuggerError(error: chrome.runtime.LastError | undefined): string {
  const message = error?.message;
  return message && message.trim() ? message.trim() : 'unknown chrome.debugger error';
}

export class CdpSessionManager {
  private records = new Map<SessionKey, CdpSessionRecord>();
  private tabOwners = new Map<number, Set<string>>();

  async acquire(sessionIdRaw: unknown, tabId: number): Promise<CdpSessionHandle> {
    const sessionId = normalizeSessionId(sessionIdRaw);
    const key = this.key(sessionId, tabId);

    await frameRouter.attachToTab(tabId);

    const now = Date.now();
    const existing = this.records.get(key);
    if (existing) {
      existing.lastUsedAtMs = now;
    } else {
      this.records.set(key, {
        sessionId,
        tabId,
        acquiredAtMs: now,
        lastUsedAtMs: now,
      });
      const owners = this.tabOwners.get(tabId) ?? new Set<string>();
      owners.add(sessionId);
      this.tabOwners.set(tabId, owners);
    }

    return {
      sessionId,
      tabId,
      sendCommand: async <T = any>(method: string, params?: any) => {
        this.touch(sessionId, tabId);
        return await this.sendCommand<T>(tabId, method, params);
      },
      // The worker owns session lifetime; per-action handles deliberately do not detach.
      release: async () => {},
    };
  }

  async releaseSession(sessionIdRaw: unknown): Promise<number[]> {
    const sessionId = normalizeSessionId(sessionIdRaw);
    const affectedTabs = new Set<number>();

    for (const [key, record] of Array.from(this.records.entries())) {
      if (record.sessionId !== sessionId) continue;
      this.records.delete(key);
      affectedTabs.add(record.tabId);

      const owners = this.tabOwners.get(record.tabId);
      if (owners) {
        owners.delete(sessionId);
        if (owners.size === 0) {
          this.tabOwners.delete(record.tabId);
        }
      }
    }

    const detachedTabs: number[] = [];
    for (const tabId of affectedTabs) {
      if (this.tabOwners.has(tabId)) continue;
      await frameRouter.detachFromTab(tabId).catch((error) => {
        console.warn(`[CdpSessionManager] Failed to detach tab ${tabId}:`, error);
      });
      detachedTabs.push(tabId);
    }

    return detachedTabs;
  }

  async releaseTab(tabId: number): Promise<void> {
    for (const [key, record] of Array.from(this.records.entries())) {
      if (record.tabId === tabId) {
        this.records.delete(key);
      }
    }
    this.tabOwners.delete(tabId);
    await frameRouter.detachFromTab(tabId).catch((error) => {
      console.warn(`[CdpSessionManager] Failed to detach tab ${tabId}:`, error);
    });
  }

  snapshot(): Array<CdpSessionRecord & { key: string }> {
    return Array.from(this.records.entries()).map(([key, value]) => ({ key, ...value }));
  }

  private key(sessionId: string, tabId: number): SessionKey {
    return `${sessionId}:${tabId}`;
  }

  private touch(sessionId: string, tabId: number): void {
    const record = this.records.get(this.key(sessionId, tabId));
    if (record) {
      record.lastUsedAtMs = Date.now();
    }
  }

  private async sendCommand<T = any>(tabId: number, method: string, params?: any): Promise<T> {
    return await new Promise<T>((resolve, reject) => {
      chrome.debugger.sendCommand({ tabId }, method, params || {}, (result) => {
        const error = chrome.runtime.lastError;
        if (error) {
          const formatted = formatChromeDebuggerError(error);
          const commandError = new Error(`CDP command failed: ${formatted}`);
          if (isExpectedCdpLifecycleError(formatted)) {
            frameRouter.markTabDetached(tabId, formatted);
            (commandError as any).code = 'CDP_TARGET_DETACHED';
          } else {
            (commandError as any).code = 'CDP_COMMAND_FAILED';
          }
          reject(commandError);
          return;
        }
        resolve(result as T);
      });
    });
  }
}

export const cdpSessionManager = new CdpSessionManager();
