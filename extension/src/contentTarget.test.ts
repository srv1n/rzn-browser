import { describe, expect, it } from 'vitest';
import {
  describeContentScriptTarget,
  invalidContentScriptTargetMessage,
  isContentScriptTargetUrl,
  tabContentTargetUrl,
} from './contentTarget';

describe('content script target URLs', () => {
  it('allows http and https pages', () => {
    expect(isContentScriptTargetUrl('https://example.com')).toBe(true);
    expect(isContentScriptTargetUrl('http://localhost:3000')).toBe(true);
  });

  it('rejects browser-owned pages with useful labels', () => {
    expect(isContentScriptTargetUrl('chrome://extensions')).toBe(false);
    expect(describeContentScriptTarget('chrome://extensions')).toBe('Chrome system pages');
    expect(describeContentScriptTarget('edge://extensions')).toBe('Edge system pages');
    expect(describeContentScriptTarget('chrome-extension://abc/options.html')).toBe('extension pages');
  });

  it('uses pendingUrl when url is not populated yet', () => {
    expect(tabContentTargetUrl({ pendingUrl: 'https://example.com/loading' })).toBe('https://example.com/loading');
  });

  it('builds the invalid target broker message text', () => {
    expect(invalidContentScriptTargetMessage('edge://extensions', 'inspect DOM')).toBe(
      'Cannot inspect DOM on Edge system pages. Please navigate to an http(s) website first.'
    );
  });
});
