import { describe, expect, it } from 'vitest';
import {
  closeExactWorkflowTab,
  exactTabMissingError,
  isExactTabMissingError,
  liveTabOrDefer,
} from './workflowTabLifecycle';

describe('closeExactWorkflowTab', () => {
  it('removes only the supplied tab ID', async () => {
    const removed: number[] = [];
    const result = await closeExactWorkflowTab(42, async (tabId) => {
      removed.push(tabId);
    });

    expect(result).toEqual({ closed: true, alreadyMissing: false });
    expect(removed).toEqual([42]);
  });

  it('makes an already-missing exact tab an idempotent no-op', async () => {
    const result = await closeExactWorkflowTab(42, async () => {
      throw new Error('No tab with id: 42');
    });

    expect(result).toEqual({ closed: false, alreadyMissing: true });
  });

  it('preserves a missing exact tab as TAB_MISSING for caller fallback', () => {
    const error = exactTabMissingError(42, new Error('No tab with id: 42'));

    expect(isExactTabMissingError(error)).toBe(true);
    expect(error.message).toContain('42');
  });
});

describe('liveTabOrDefer', () => {
  // Chrome's wording when a tab id outlived its tab (e.g. a parked tab after an extension reload).
  const gone = async () => {
    throw new Error('No tab with given id 639035019.');
  };

  it('keeps a live session tab', async () => {
    await expect(liveTabOrDefer(7, false, async () => ({}))).resolves.toBe(7);
  });

  it('defers to the navigation path when the session tab is gone', async () => {
    await expect(liveTabOrDefer(639035019, false, gone)).resolves.toBeUndefined();
  });

  it('reports a gone exact tab ref as TAB_MISSING so the runner opens a fresh tab', async () => {
    await expect(liveTabOrDefer(639035019, true, gone)).rejects.toMatchObject({ code: 'TAB_MISSING' });
  });

  it('waits for a tab when the session has none', async () => {
    await expect(liveTabOrDefer(undefined, true, gone)).resolves.toBeUndefined();
  });
});
