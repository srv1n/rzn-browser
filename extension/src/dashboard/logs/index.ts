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
    root.innerHTML = `<section><h1>Logs</h1><div class="filters"><label>Level <input data-level value="${esc(level)}"></label><label>Component <input data-component value="${esc(component)}"></label><label>Run ID <input data-run-id value="${esc(runId)}"></label><label><input data-auto type="checkbox" ${auto ? 'checked' : ''}> Auto-refresh (2s)</label></div><button data-export>Export diagnostics</button><p data-export-result class="muted"></p><p class="muted">Diagnostics are token- and params-redacted.</p><pre data-log-lines></pre></section>`;
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
