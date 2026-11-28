export type RpcClient = <T = any>(method: string, params?: Record<string, unknown>) => Promise<T>;

export type DashboardRoute = { tab: string; id?: string; query: URLSearchParams };

/** Pages in the side navigation. */
export const mainTabs = ['account', 'settings'];
/** Our own tools, reached from Settings › Advanced; not in the main navigation. */
export const advancedTabs = ['runs', 'workflows', 'logs'];
export const tabs = [...mainTabs, ...advancedTabs];
export const tabLabels: Record<string, string> = { account: 'Account', settings: 'Settings', runs: 'Runs', workflows: 'Workflows', logs: 'Logs' };

export const esc = (value: unknown) => String(value ?? '').replace(/[&<>"']/g, char => ({
  '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
}[char]!));

export function relativeTime(timestamp?: number, now = Date.now()): string {
  if (!timestamp) return '—';
  const seconds = Math.max(0, Math.floor((now - timestamp) / 1000));
  if (seconds < 60) return `${seconds}s ago`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86_400)}d ago`;
}

export function duration(started?: number, ended?: number): string {
  if (!started || !ended) return '—';
  const seconds = Math.max(0, Math.round((ended - started) / 1000));
  return seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

export function statusClass(status?: string): string {
  if (status === 'succeeded' || status === 'active' || status === 'healthy') return 'ok';
  if (status === 'failed' || status === 'broken' || status === 'revoked') return 'bad';
  if (status === 'degraded' || status === 'dormant') return 'warn';
  return 'muted';
}

export const dot = (status?: string) => `<span class="status-dot ${statusClass(status)}" title="${esc(status)}">●</span>`;

export function isPaused(snapshot: any): boolean { return Boolean(snapshot?.paused); }

const details = (error: unknown) => `<details class="details"><summary>Details</summary><pre>${esc(error instanceof Error ? error.message : error)}</pre></details>`;

export function unavailable(root: HTMLElement, error: unknown): void {
  root.innerHTML = `<section><div class="empty"><span class="label">Not connected</span><h1>RZN isn't <em>answering</em></h1><p>The RZN helper on this computer didn't respond. Make sure RZN is installed and running, then reload this page.</p>${details(error)}</div></section>`;
}

export function applicationError(root: HTMLElement, error: unknown): void {
  root.innerHTML = `<section><div class="empty"><span class="label">Something went wrong</span><h1>That didn't <em>work</em></h1>${details(error)}</div></section>`;
}
