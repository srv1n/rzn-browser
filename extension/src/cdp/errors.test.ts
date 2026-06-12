import { describe, expect, it } from 'vitest';
import {
  cdpErrorText,
  isExecutionContextDestroyedCdpError,
  isExpectedCdpLifecycleError,
} from './errors';

describe('CDP error helpers', () => {
  it('formats chrome debugger lastError objects with useful metadata', () => {
    expect(cdpErrorText({ message: 'Runtime.evaluate failed', code: -32000, data: 'lost' }))
      .toBe('Runtime.evaluate failed code=-32000 data=lost');
  });

  it('treats debugger detachment during a command as a lifecycle error', () => {
    const message = 'Detached while handling command';

    expect(isExpectedCdpLifecycleError(message)).toBe(true);
    expect(isExecutionContextDestroyedCdpError(message)).toBe(true);
  });

  it('does not classify ordinary protocol failures as lifecycle churn', () => {
    const message = 'Invalid parameters for DOM.querySelector';

    expect(isExpectedCdpLifecycleError(message)).toBe(false);
    expect(isExecutionContextDestroyedCdpError(message)).toBe(false);
  });
});
