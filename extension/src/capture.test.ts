import { describe, expect, it, vi } from 'vitest';
import { captureSitesOf, onPageLoaded, siteOf, switchedOn, type CaptureDeps } from './capture';

function deps(sites: string[], workflowTabs: number[] = []) {
  const call = vi.fn(async () => ({ read: true }));
  const d: CaptureDeps = {
    sites: () => sites,
    isWorkflowTab: (tabId) => workflowTabs.includes(tabId),
    browserInstanceId: async () => '11111111-2222-4333-8444-555555555555',
    call,
  };
  return { d, call };
}

const page = { tabId: 7, frameId: 0, url: 'https://www.voicebraindump.com/blog/offline' };

describe('capture of opened pages', () => {
  it('reads nothing while the switch is off (the default)', async () => {
    const { d, call } = deps([]);
    expect(await onPageLoaded(page, d)).toBe(false);
    const other = deps(['lambda.ai']);
    expect(await onPageLoaded(page, other.d)).toBe(false);
    expect(call).not.toHaveBeenCalled();
    expect(other.call).not.toHaveBeenCalled();
  });

  it('reports a page of a switched-on site once, with its tab', async () => {
    const { d, call } = deps(['voicebraindump.com']);
    expect(await onPageLoaded(page, d)).toBe(true);
    expect(call).toHaveBeenCalledTimes(1);
    expect(call).toHaveBeenCalledWith('capture.page', {
      url: page.url,
      tab_ref: 'rzn://browser/11111111-2222-4333-8444-555555555555/tab/7',
    });
  });

  it('leaves frames, workflow tabs and non-web pages alone', async () => {
    const { d, call } = deps(['voicebraindump.com'], [7]);
    expect(await onPageLoaded(page, d)).toBe(false);
    expect(await onPageLoaded({ ...page, tabId: 8, frameId: 3 }, d)).toBe(false);
    expect(await onPageLoaded({ ...page, tabId: 8, url: 'chrome://extensions' }, d)).toBe(false);
    expect(call).not.toHaveBeenCalled();
  });

  it('matches a site and its subdomains, never a suffix', () => {
    expect(siteOf('https://WWW.Example.com/a#b')).toBe('example.com');
    expect(siteOf('file:///tmp/a.html')).toBeNull();
    expect(switchedOn(['example.com'], 'blog.example.com')).toBe(true);
    expect(switchedOn(['ample.com'], 'example.com')).toBe(false);
  });

  it('reads the site list from settings', () => {
    expect(captureSitesOf({ capture_sites: ['a.com', 3] })).toEqual(['a.com']);
    expect(captureSitesOf(null)).toEqual([]);
  });
});
