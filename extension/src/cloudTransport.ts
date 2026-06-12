function isLoopbackHost(hostname: string): boolean {
  const host = hostname.toLowerCase().replace(/^\[|\]$/g, '');
  return host === 'localhost' || host === '127.0.0.1' || host === '::1';
}

export function normalizeCloudServerUrl(serverUrl: string): string {
  const url = new URL(serverUrl);
  if (url.protocol !== 'https:' && !(url.protocol === 'http:' && isLoopbackHost(url.hostname))) {
    throw new Error('Cloud server URL must use https unless it targets loopback');
  }
  if (url.pathname === '/') {
    url.pathname = '';
  }
  url.search = '';
  url.hash = '';
  return url.toString().replace(/\/$/, '');
}
