import { afterEach, beforeEach, describe, expect, test } from 'vitest';
import { FrameRouter } from './frameRouter';

describe('FrameRouter lifecycle', () => {
  let listeners: Array<(source: any, method: string, params: any) => void>;
  let tabUrl: string;
  let savedChrome: PropertyDescriptor | undefined;

  beforeEach(() => {
    listeners = [];
    tabUrl = 'https://example.test/';
    savedChrome = Object.getOwnPropertyDescriptor(globalThis, 'chrome');
    Object.assign(globalThis, {
      chrome: {
        runtime: { id: 'extension-under-test', lastError: undefined },
        tabs: { get: async () => ({ url: tabUrl }) },
        debugger: {
          attach: async () => {},
          detach: async () => {},
          onEvent: {
            addListener: (listener: any) => listeners.push(listener),
            removeListener: (listener: unknown) => { listeners = listeners.filter(item => item !== listener); },
          },
          sendCommand: (_target: any, _method: string, _params: any, callback: (result?: any) => void) => callback({}),
        },
      },
    });
  });

  afterEach(() => {
    if (savedChrome) Object.defineProperty(globalThis, 'chrome', savedChrome);
    else Reflect.deleteProperty(globalThis, 'chrome');
  });

  test('detaching one tab keeps other tabs execution contexts', async () => {
    const router = new FrameRouter();
    await router.attachToTab(1);
    await router.attachToTab(2);
    const context = { id: 1, origin: 'https://example.test', name: '', uniqueId: 'u' };
    listeners.forEach(listener => listener({ tabId: 1 }, 'Runtime.executionContextCreated', { context }));
    listeners.forEach(listener => listener({ tabId: 2 }, 'Runtime.executionContextCreated', { context }));
    const contexts = (router as any).contextMap as Map<string, unknown>;
    expect(contexts.size).toBe(2);

    await router.detachFromTab(1);
    expect(Array.from(contexts.keys())).toEqual(['2::1']);
  });

  test('a held attachment survives detach requests until the last hold is released', async () => {
    const detached: number[] = [];
    let attached = true;
    (globalThis as any).chrome.debugger.detach = async ({ tabId }: { tabId: number }) => { detached.push(tabId); };
    (globalThis as any).chrome.debugger.attach = async () => {
      if (attached) throw new Error('Another debugger is already attached to the tab with id: 4.');
    };
    const router = new FrameRouter();
    router.holdAttachment(4);
    // Network capture attached first; the router reuses that attachment instead of replacing it.
    await router.attachToTab(4);
    await router.detachFromTab(4);
    expect(detached).toEqual([]);
    expect(await router.releaseAttachment(4)).toBe(false);
    expect(detached).toEqual([4]);

    attached = false;
    router.holdAttachment(5);
    expect(await router.releaseAttachment(5)).toBe(true);
  });

  test('restricted URLs fail with a non-retryable RESTRICTED_URL error', async () => {
    tabUrl = 'chrome://extensions/';
    await expect(new FrameRouter().attachToTab(3)).rejects.toMatchObject({ code: 'RESTRICTED_URL', retryable: false });
  });
});
