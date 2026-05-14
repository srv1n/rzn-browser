import { afterEach, describe, expect, it, vi } from 'vitest';
import { getActiveBrowserTabId } from './browserTabs';

function installChromeMock(overrides: {
  getLastFocused?: ReturnType<typeof vi.fn>;
  getAll?: ReturnType<typeof vi.fn>;
  query?: ReturnType<typeof vi.fn>;
}) {
  (globalThis as any).chrome = {
    windows: {
      WINDOW_ID_NONE: -1,
      getLastFocused: overrides.getLastFocused ?? vi.fn(),
      getAll: overrides.getAll ?? vi.fn(),
    },
    tabs: {
      query: overrides.query ?? vi.fn(),
    },
  };
}

describe('browser tab resolver', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    delete (globalThis as any).chrome;
  });

  it('falls back when Chrome has no current window in the service worker', async () => {
    const query = vi.fn(async (options: any) => {
      if (options.lastFocusedWindow) {
        return [{ id: 42, active: true, windowId: 7, url: 'https://example.com' }];
      }
      return [];
    });

    installChromeMock({
      getLastFocused: vi.fn(async () => {
        throw new Error('No current window');
      }),
      query,
    });

    await expect(getActiveBrowserTabId('test action')).resolves.toBe(42);
    expect(query).not.toHaveBeenCalledWith(expect.objectContaining({ currentWindow: true }));
  });

  it('uses populated normal windows if last-focused tab query is unavailable', async () => {
    installChromeMock({
      getLastFocused: vi.fn(async () => {
        throw new Error('No current window');
      }),
      query: vi.fn(async () => {
        throw new Error('No current window');
      }),
      getAll: vi.fn(async () => [
        {
          id: 9,
          focused: true,
          tabs: [
            { id: 51, active: true, windowId: 9, url: 'https://example.com' },
          ],
        },
      ]),
    });

    await expect(getActiveBrowserTabId('test action')).resolves.toBe(51);
  });
});

