import { afterEach, describe, expect, it, vi } from 'vitest';
import { TypeTextAction } from './type_text';

afterEach(() => vi.unstubAllGlobals());

describe('CDP type_text cancellation', () => {
  it('does not insert another character after its lease is cancelled', async () => {
    let cancelled = false;
    const sendCommand = vi.fn(async () => {
      cancelled = true;
    });
    vi.stubGlobal('chrome', { debugger: { sendCommand } });

    await expect(new TypeTextAction().execute(7, 'abc', {
      manageDebuggerLifecycle: false,
      assertActive: () => {
        if (cancelled) throw new Error('cancelled');
      },
    })).rejects.toThrow('cancelled');
    expect(sendCommand).toHaveBeenCalledTimes(1);
    expect(sendCommand).toHaveBeenCalledWith({ tabId: 7 }, 'Input.insertText', { text: 'a' });
  });
});
