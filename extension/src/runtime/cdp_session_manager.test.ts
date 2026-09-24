import { afterEach, describe, expect, test, vi } from 'vitest';

describe('CdpSessionManager lifecycle', () => {
  afterEach(() => vi.unstubAllGlobals());

  test('drops session records when the tab closes or the debugger detaches', async () => {
    const onRemoved: Array<(tabId: number) => void> = [];
    const onDetach: Array<(source: { tabId?: number }) => void> = [];
    vi.stubGlobal('chrome', {
      runtime: { id: 'extension-under-test' },
      tabs: { get: async () => ({ url: 'https://example.test/' }), onRemoved: { addListener: (l: any) => onRemoved.push(l) } },
      debugger: {
        attach: async () => {},
        detach: async () => {},
        onEvent: { addListener: () => {}, removeListener: () => {} },
        onDetach: { addListener: (l: any) => onDetach.push(l) },
        sendCommand: (_t: any, _m: string, _p: any, callback: (result?: any) => void) => callback({}),
      },
    });
    const { CdpSessionManager } = await import('./cdp_session_manager');
    const manager = new CdpSessionManager();
    await manager.acquire('a', 1);
    await manager.acquire('b', 2);
    expect(manager.snapshot()).toHaveLength(2);

    onRemoved.forEach(listener => listener(1));
    expect(manager.snapshot().map(record => record.tabId)).toEqual([2]);
    onDetach.forEach(listener => listener({ tabId: 2 }));
    expect(manager.snapshot()).toEqual([]);
  });
});
