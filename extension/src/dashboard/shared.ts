export type RpcClient = <T = any>(method: string, params?: Record<string, unknown>) => Promise<T>;

export type DashboardRoute = { tab: string; id?: string; query: URLSearchParams };

export const tabs = ['runs', 'workflows', 'fleet', 'logs', 'settings'];

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

export function unavailable(root: HTMLElement, error: unknown): void {
  root.innerHTML = `<section class="empty"><h1>Supervisor unreachable</h1><p>Reconnect the native host and retry.</p><pre>${esc(error instanceof Error ? error.message : error)}</pre></section>`;
}

export function applicationError(root: HTMLElement, error: unknown): void {
  root.innerHTML = `<section class="empty"><h1>Request failed</h1><pre>${esc(error instanceof Error ? error.message : error)}</pre></section>`;
}
