import { DashboardRoute, esc, RpcClient } from '../shared';

export async function mountLogs(root: HTMLElement, call: RpcClient, route: DashboardRoute): Promise<() => void> {
  let level = ''; let component = ''; let runId = route.query.get('run') || ''; let auto = false; let pausedForScroll = false; let timer: ReturnType<typeof setInterval> | undefined;
  const load = async (): Promise<void> => {
    const response = await call<any>('logs.tail', { limit: 500, ...(level && { level }), ...(component && { component }), ...(runId && { run_id: runId }) });
    const target = root.querySelector<HTMLElement>('[data-log-lines]');
    if (target) target.textContent = (response.entries || []).slice(-500).map((entry: any) => `${entry.ts || entry.at || ''} ${entry.level || ''} ${entry.component || ''} ${entry.run_id || ''} ${entry.message || ''}`.trim()).join('\n');
    return response;
  };
  const render = async (): Promise<void> => {
    root.innerHTML = `<section><header class="page-header"><div><h1>Logs</h1><p>Supervisor and extension diagnostics. Tokens and parameters are redacted.</p></div><button class="secondary" data-export>Export diagnostics</button></header><div class="filters"><label class="control"><span>Level</span><input data-level value="${esc(level)}" placeholder="All levels"></label><label class="control"><span>Component</span><input data-component value="${esc(component)}" placeholder="All components"></label><label class="control search"><span>Run ID</span><input data-run-id value="${esc(runId)}" placeholder="Filter by run"></label><label><input data-auto type="checkbox" ${auto ? 'checked' : ''}> Auto-refresh every 2s</label></div><p data-export-result class="muted"></p><pre data-log-lines aria-label="Log entries"></pre></section>`;
    await load();
    root.querySelector<HTMLInputElement>('[data-level]')!.addEventListener('change', event => { level = (event.target as HTMLInputElement).value; void load(); });
    root.querySelector<HTMLInputElement>('[data-component]')!.addEventListener('change', event => { component = (event.target as HTMLInputElement).value; void load(); });
    root.querySelector<HTMLInputElement>('[data-run-id]')!.addEventListener('change', event => { runId = (event.target as HTMLInputElement).value; void load(); });
    root.querySelector<HTMLInputElement>('[data-auto]')!.addEventListener('change', event => { auto = (event.target as HTMLInputElement).checked; if (timer) clearInterval(timer); timer = auto ? setInterval(() => { if (!pausedForScroll) void load(); }, 2_000) : undefined; });
    root.querySelector<HTMLElement>('[data-log-lines]')!.addEventListener('scroll', event => { const element = event.currentTarget as HTMLElement; pausedForScroll = element.scrollTop + element.clientHeight < element.scrollHeight; });
    root.querySelector('[data-export]')!.addEventListener('click', async () => { const result = root.querySelector<HTMLElement>('[data-export-result]')!; try { const response: any = await call('diagnostics.export', {}); result.innerHTML = `Saved to <code>${esc(response.path)}</code> <button data-copy-path>Copy</button>`; result.querySelector('[data-copy-path]')?.addEventListener('click', () => { void navigator.clipboard?.writeText(response.path); }); } catch (cause) { result.textContent = cause instanceof Error ? cause.message : String(cause); } });
  };
  await render();
  return () => { if (timer) clearInterval(timer); };
}
