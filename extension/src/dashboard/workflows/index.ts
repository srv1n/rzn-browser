import { DashboardRoute, dot, esc, isPaused, relativeTime, RpcClient } from '../shared';

const healthSentence = (health: any): string => {
  const dominant = health?.dominant_fingerprint;
  if (!dominant) return '';
  const date = new Date(dominant.first_seen_at).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  return `failed ${dominant.count}× at step ${(dominant.step_index ?? 0) + 1} (${dominant.error_class}) since ${date}`;
};

export async function mountWorkflows(root: HTMLElement, call: RpcClient, _route: DashboardRoute, options: { navigate: (hash: string) => void }): Promise<void> {
  root.innerHTML = '<p>Loading workflows…</p>';
  const [response, snapshot] = await Promise.all([call<any>('workflows.list', {}), call<any>('status.snapshot', {}).catch(() => ({ paused: false }))]);
  const paused = isPaused(snapshot);
  const workflows = response.workflows || [];
  root.innerHTML = `<section><h1>Workflows</h1>${workflows.length ? workflows.map((workflow: any, index: number) => {
    const flag = workflow.health?.flag || 'healthy'; const sentence = healthSentence(workflow.health);
    return `<article class="workflow" data-workflow="${esc(workflow.workflow_id)}"><h2>${esc(workflow.name || workflow.workflow_id)}</h2><code>${esc(workflow.workflow_id)}</code> <span class="chip">${esc(workflow.source)}</span> <span class="chip">${esc((workflow.workflow_hash || '').slice(0, 8))}</span> <span class="chip flag-${esc(flag)}">${dot(flag)} ${esc(flag)}</span><p>Last run ${relativeTime(workflow.last_run_at)}</p>${sentence ? `<p class="health-sentence">${esc(sentence)}</p>` : ''}<button data-run-now="${index}" ${paused ? 'disabled title="Automation is paused."' : ''}>Run now</button><form class="run-form" data-form="${index}" hidden><label><input data-raw-json type="checkbox" checked> Raw JSON</label><label data-json-params>Parameters <textarea data-params placeholder='{"key":"value"}'>{}</textarea></label><div data-key-values hidden><label>Key <input data-param-key></label><label>Value <input data-param-value></label><label>Key <input data-param-key></label><label>Value <input data-param-value></label></div><p class="error" data-error></p><button type="submit">Start run</button></form></article>`;
  }).join('') : '<div class="empty">No workflows are available on this device.</div>'}</section>`;
  root.querySelectorAll<HTMLButtonElement>('[data-run-now]').forEach(button => button.addEventListener('click', () => { const form = root.querySelector<HTMLFormElement>(`[data-form="${button.dataset.runNow}"]`)!; form.hidden = !form.hidden; }));
  root.querySelectorAll<HTMLInputElement>('[data-raw-json]').forEach(toggle => toggle.addEventListener('change', () => {
    const form = toggle.closest<HTMLFormElement>('form')!;
    form.querySelector<HTMLElement>('[data-json-params]')!.hidden = !toggle.checked;
    form.querySelector<HTMLElement>('[data-key-values]')!.hidden = toggle.checked;
  }));
  root.querySelectorAll<HTMLFormElement>('form.run-form').forEach(form => form.addEventListener('submit', async event => {
    event.preventDefault();
    const textarea = form.querySelector<HTMLTextAreaElement>('[data-params]')!;
    const error = form.querySelector<HTMLElement>('[data-error]')!;
    let params: unknown;
    try {
      const raw = form.querySelector<HTMLInputElement>('[data-raw-json]')!;
      const keys = form.querySelectorAll<HTMLInputElement>('[data-param-key]');
      const values = form.querySelectorAll<HTMLInputElement>('[data-param-value]');
      params = raw.checked ? JSON.parse(textarea.value || '{}') : Object.fromEntries([...keys].map((key, index) => [key.value, values[index]?.value]).filter(([key]) => key));
      if (!params || Array.isArray(params) || typeof params !== 'object') throw new Error('Parameters must be a JSON object.');
    }
    catch (cause) { error.textContent = cause instanceof Error ? `Invalid JSON: ${cause.message}` : 'Invalid JSON.'; return; }
    try { const run: any = await call('runs.start', { workflow_id: (form.closest<HTMLElement>('[data-workflow]')!).dataset.workflow!, params: params as Record<string, unknown> }); options.navigate(`#runs/${encodeURIComponent(run.run_id || run.record?.run_id)}`); }
    catch (cause) { error.textContent = cause instanceof Error ? cause.message : String(cause); }
  }));
}
