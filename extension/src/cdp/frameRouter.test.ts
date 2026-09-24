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

  test('restricted URLs fail with a non-retryable RESTRICTED_URL error', async () => {
    tabUrl = 'chrome://extensions/';
    await expect(new FrameRouter().attachToTab(3)).rejects.toMatchObject({ code: 'RESTRICTED_URL', retryable: false });
  });
});
