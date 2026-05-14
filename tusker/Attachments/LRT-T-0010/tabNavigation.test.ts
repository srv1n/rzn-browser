import { describe, expect, it } from 'vitest';
import { tabNavigationChanged, type TabNavigationState } from './tabNavigation';

function state(partial: Partial<TabNavigationState>): TabNavigationState {
  return {
    url: 'https://www.instagram.com/natgeo/',
    pendingUrl: '',
    status: 'complete',
    ...partial,
  };
}

describe('tab navigation detection', () => {
  it('ignores status-only loading transitions when the URL is unchanged', () => {
    expect(
      tabNavigationChanged(
        state({ status: 'complete' }),
        state({ status: 'loading' })
      )
    ).toBe(false);
  });

  it('detects pending URL changes', () => {
    expect(
      tabNavigationChanged(
        state({ status: 'complete' }),
        state({ pendingUrl: 'https://www.instagram.com/p/example/' })
      )
    ).toBe(true);
  });

  it('detects committed URL changes', () => {
    expect(
      tabNavigationChanged(
        state({ url: 'https://www.instagram.com/natgeo/' }),
        state({ url: 'https://www.instagram.com/p/example/' })
      )
    ).toBe(true);
  });
});
