import { DashboardRoute, dot, duration, esc, isPaused, relativeTime, RpcClient, statusClass } from '../shared';

type Run = { run_id: string; workflow_id: string; origin: string; status: string; started_at: number; ended_at?: number; failing_step_index?: number; error_class?: string; fingerprint?: string; error_message?: string };
type MountOptions = { navigate: (hash: string) => void };
const pageSize = 50;

const originLabel = (origin: string) => origin.startsWith('fleet:') ? 'fleet' : origin;
const option = (value: string, current: string) => `<option value="${esc(value)}" ${value === current ? 'selected' : ''}>${esc(value || 'All')}</option>`;

export async function mountRuns(root: HTMLElement, call: RpcClient, route: DashboardRoute, options: MountOptions): Promise<void> {
  if (route.id) return mountDetail(root, call, route.id, options);
  let page = 0;
  let workflow = '';
  let status = '';
  let origin = '';
  let text = '';

  const render = async (): Promise<void> => {
    root.innerHTML = '<p>Loading runs…</p>';
    const [listed, health] = await Promise.all([
      call<any>('runs.list', { limit: pageSize, offset: page * pageSize, ...(workflow && { workflow_id: workflow }), ...(status && { status }), ...(origin && { origin }) }),
      call<any>('workflows.health', {}).catch(() => ({ workflows: [] })),
    ]);
    const runs: Run[] = listed.runs || [];
    const visible = runs.filter(run => `${run.workflow_id} ${run.origin} ${run.status} ${run.run_id}`.toLowerCase().includes(text.toLowerCase()));
    const workflows = [...new Set([...(health.workflows || []).map((item: any) => item.workflow_id), ...runs.map(run => run.workflow_id)])].sort();
    const statuses = [...new Set(runs.map(run => run.status))].sort();
    const origins = [...new Set(runs.map(run => originLabel(run.origin)))].sort();
    const total = Number(listed.total || 0);
    root.innerHTML = `<section><h1>Runs</h1><div class="filters">
      <label>Workflow <select data-filter="workflow">${option('', workflow)}${workflows.map(value => option(value, workflow)).join('')}</select></label>
      <label>Status <select data-filter="status">${option('', status)}${statuses.map(value => option(value, status)).join('')}</select></label>
      <label>Origin <select data-filter="origin">${option('', origin)}${origins.map(value => option(value, origin)).join('')}</select></label>
      <label>Find on this page <input data-filter="text" value="${esc(text)}" placeholder="workflow, origin, status"></label>
    </div>
    ${total === 0 ? '<div class="empty"><p>No runs yet.</p><a href="#workflows">Run a workflow</a></div>' : `<table><thead><tr><th></th><th>Workflow</th><th>Origin</th><th>Started</th><th>Duration</th><th>Failing step</th></tr></thead><tbody>${visible.map(run => `<tr data-run="${esc(run.run_id)}"><td>${dot(run.status)}</td><td>${esc(run.workflow_id)}</td><td><span class="chip">${esc(originLabel(run.origin))}</span></td><td>${relativeTime(run.started_at)}</td><td>${duration(run.started_at, run.ended_at)}</td><td>${run.status === 'failed' ? esc(run.failing_step_index ?? '—') : '—'}</td></tr>`).join('')}</tbody></table>`}
    <nav class="pagination"><button data-page="prev" ${page === 0 ? 'disabled' : ''}>Previous</button><span>Page ${page + 1} of ${Math.max(1, Math.ceil(total / pageSize))}</span><button data-page="next" ${(page + 1) * pageSize >= total ? 'disabled' : ''}>Next</button></nav></section>`;
    root.querySelectorAll<HTMLSelectElement>('select[data-filter]').forEach(select => select.addEventListener('change', () => {
      if (select.dataset.filter === 'workflow') workflow = select.value;
      if (select.dataset.filter === 'status') status = select.value;
      if (select.dataset.filter === 'origin') origin = select.value;
      page = 0; void render();
    }));
    root.querySelector<HTMLInputElement>('input[data-filter="text"]')?.addEventListener('input', event => { text = (event.target as HTMLInputElement).value; void render(); });
    root.querySelectorAll<HTMLTableRowElement>('tr[data-run]').forEach(row => row.addEventListener('click', () => options.navigate(`#runs/${encodeURIComponent(row.dataset.run!)}`)));
    root.querySelector('[data-page="prev"]')?.addEventListener('click', () => { page--; void render(); });
    root.querySelector('[data-page="next"]')?.addEventListener('click', () => { page++; void render(); });
  };
  await render();
}

async function mountDetail(root: HTMLElement, call: RpcClient, id: string, options: MountOptions): Promise<void> {
  root.innerHTML = '<p>Loading run…</p>';
  const decoded = decodeURIComponent(id);
  const [response, snapshot, history, workflows] = await Promise.all([
    call<any>('runs.get', { run_id: decoded }),
    call<any>('status.snapshot', {}).catch(() => ({ paused: false })),
    call<any>('runs.list', { limit: 200 }).catch(() => ({ runs: [] })),
    call<any>('workflows.list', {}).catch(() => ({ workflows: [] })),
  ]);
  if (!response.ok) { root.innerHTML = `<section class="empty"><h1>Run not found</h1><p>${esc(response.error)}</p></section>`; return; }
  const record: Run = response.record;
  const result = response.result || {};
  const context = await call<any>('runs.get_failure_context', { run_id: decoded }).catch(() => ({ capture_unavailable: 'Failure context could not be loaded.' }));
  const firstSeen = record.fingerprint && (history.runs || []).filter((run: Run) => run.fingerprint === record.fingerprint).sort((a: Run, b: Run) => a.started_at - b.started_at)[0]?.started_at;
  const available = (workflows.workflows || []).some((workflow: any) => workflow.workflow_id === record.workflow_id);
  const disabled = isPaused(snapshot) || !available;
  const disabledReason = isPaused(snapshot) ? 'Automation is paused.' : 'This workflow is no longer available.';
  const steps = result.steps || [];
  root.innerHTML = `<section><a href="#runs">← All runs</a><h1>Run ${esc(decoded)}</h1>
    <div class="run-meta">${dot(record.status)} ${esc(record.workflow_id)} <span class="chip">${esc(originLabel(record.origin))}</span></div>
    <h2>Steps</h2><ol class="timeline">${steps.map((step: any, index: number) => `<li class="${index === record.failing_step_index ? 'failed-step' : ''}">${dot(step.status)} <b>${esc(step.step_id || `Step ${index + 1}`)}</b> <small>${esc(step.duration_ms != null ? `${step.duration_ms}ms` : step.duration || '')}</small></li>`).join('') || '<li>No step timeline recorded.</li>'}</ol>
    ${record.error_class || record.fingerprint || record.error_message ? `<section class="error-block"><h2>Failure</h2><span class="chip ${statusClass('failed')}">${esc(record.error_class || 'unknown')}</span><p>${esc(record.error_message || result.error?.message || result.failure_summary?.message || '')}</p><code>${esc(record.fingerprint || result.failure_summary?.fingerprint || '')}</code>${firstSeen ? `<p>First seen ${relativeTime(firstSeen)}</p>` : ''}</section>` : ''}
    <section><h2>Failure context</h2>${context.console_tail ? `<details><summary>Console tail</summary><pre class="console">${esc(context.console_tail)}</pre></details>` : ''}${context.screenshot_b64 ? `<img class="failure-screenshot" alt="Failure screenshot" src="data:image/png;base64,${esc(context.screenshot_b64)}">` : ''}${context.dom_excerpt ? `<details><summary>DOM excerpt</summary><pre>${esc(context.dom_excerpt)}</pre></details>` : ''}${context.capture_unavailable ? `<p class="muted">${esc(context.capture_unavailable)}</p>` : ''}</section>
    <section><h2>Result</h2><button data-copy-result>Copy JSON</button><details><summary>Show result JSON</summary><pre data-result>${esc(JSON.stringify(result, null, 2))}</pre></details></section>
    <button data-rerun ${disabled ? 'disabled title="' + esc(disabledReason) + '"' : ''}>Re-run with same params</button><p data-action-error class="error"></p></section>`;
  root.querySelector('[data-copy-result]')?.addEventListener('click', async () => { await navigator.clipboard?.writeText(JSON.stringify(result, null, 2)); });
  root.querySelector('[data-rerun]')?.addEventListener('click', async () => {
    const error = root.querySelector<HTMLElement>('[data-action-error]')!;
    try { const next: any = await call('runs.replay', { run_id: decoded }); options.navigate(`#runs/${encodeURIComponent(next.run_id || next.record?.run_id)}`); }
    catch (cause) { error.textContent = cause instanceof Error ? cause.message : String(cause); }
  });
}
