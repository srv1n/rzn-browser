import { describe, expect, it } from 'vitest';
import {
  closeExactWorkflowTab,
  exactTabMissingError,
  isExactTabMissingError,
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
