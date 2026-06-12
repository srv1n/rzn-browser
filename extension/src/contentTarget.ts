type TabLike = {
  url?: string;
  pendingUrl?: string;
};

export function tabContentTargetUrl(tab: TabLike | undefined | null): string {
  return tab?.url || tab?.pendingUrl || '';
}

export function isContentScriptTargetUrl(url: string): boolean {
  return /^https?:\/\//i.test(url);
}

export function describeContentScriptTarget(url: string): string {
  if (!url) {
    return 'invalid URL';
  }

  let protocol = '';
  try {
    protocol = new URL(url).protocol.toLowerCase();
  } catch {
    return 'invalid URL';
  }

  switch (protocol) {
    case 'chrome:':
      return 'Chrome system pages';
    case 'edge:':
      return 'Edge system pages';
    case 'chrome-extension:':
    case 'moz-extension:':
      return 'extension pages';
    case 'about:':
      return 'browser system pages';
    case 'devtools:':
      return 'DevTools pages';
    case 'file:':
      return 'local file pages';
    default:
      return 'non-http(s) pages';
  }
}

export function invalidContentScriptTargetMessage(url: string, operation = 'execute actions'): string {
  return `Cannot ${operation} on ${describeContentScriptTarget(url)}. Please navigate to an http(s) website first.`;
}
