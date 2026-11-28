import { friendly } from '../ui/errors';
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

/** One title and one plain sentence; the raw detail goes to the console (see errors.ts). */
export function problem(root: HTMLElement, error: unknown): void {
  const { title, line } = friendly(error, 'RZN settings');
  root.innerHTML = `<section class="narrow"><div class="empty"><h1>${esc(title)}</h1><p class="lede">${esc(line)}</p></div></section>`;
}

export const unavailable = problem;
export const applicationError = problem;
