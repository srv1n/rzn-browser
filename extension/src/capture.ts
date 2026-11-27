// Capture of pages the owner opens (BFL-T-0025). The owner switches sites on in the dashboard
// (supervisor setting `capture_sites`, default none). When a page of a switched-on site finishes
// loading in an ordinary tab (never a tab a workflow run owns), the extension asks the supervisor
// to read it in place (`capture.page`); the supervisor runs the site's read workflow on that tab
// with no navigation or scrolling and pushes the items, in batches, to the owner's private index.
// A site that is off is never reported: nothing about its pages leaves the extension.

export type PageLoad = { tabId: number; frameId: number; url: string };

export type CaptureDeps = {
  /** The owner's switched-on sites, as last read from the supervisor. */
  sites: () => readonly string[];
  /** True for a tab a workflow run owns; capture leaves those alone. */
  isWorkflowTab: (tabId: number) => boolean;
  browserInstanceId: () => Promise<string>;
  /** Supervisor RPC through the native host. */
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
};

/** The site of a web page: its host, lowercase, without `www.`; null for anything else. */
export function siteOf(url: string): string | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null;
  const host = parsed.hostname.toLowerCase();
  return host.startsWith('www.') ? host.slice(4) : host;
}

/** A site is on when it, or a domain it belongs to, is in the owner's list. */
export function switchedOn(sites: readonly string[], site: string): boolean {
  return sites.some((raw) => {
    const lower = String(raw).trim().toLowerCase();
    const on = lower.startsWith('www.') ? lower.slice(4) : lower;
    return on !== '' && (site === on || site.endsWith(`.${on}`));
  });
}

/** Report a finished page load to the supervisor when its site is on; true when reported. */
export async function onPageLoaded(load: PageLoad, deps: CaptureDeps): Promise<boolean> {
  if (load.frameId !== 0) return false;
  const site = siteOf(load.url);
  if (!site || !switchedOn(deps.sites(), site)) return false;
  if (deps.isWorkflowTab(load.tabId)) return false;
  const instance = await deps.browserInstanceId();
  const tabRef = `rzn://browser/${instance}/tab/${load.tabId}`;
  try {
    await deps.call('capture.page', { url: load.url, tab_ref: tabRef });
  } catch (error) {
    console.warn('[Capture] the supervisor did not take the page:', error);
    return false;
  }
  return true;
}

/** The capture site list from a `settings.get` answer; empty when absent or malformed. */
export function captureSitesOf(settings: unknown): string[] {
  const sites = (settings as { capture_sites?: unknown } | null)?.capture_sites;
  return Array.isArray(sites) ? sites.filter((s): s is string => typeof s === 'string') : [];
}
