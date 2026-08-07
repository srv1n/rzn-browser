import { DashboardRoute, dot, esc, isPaused, relativeTime, RpcClient } from '../shared';

type ParamKind = 'string' | 'integer' | 'number' | 'boolean' | 'object' | 'array';
type ParamDef = {
  kind: ParamKind;
  required?: boolean;
  sensitive?: boolean;
  description?: string;
  default?: unknown;
  enum_values?: unknown[];
  min?: number;
  max?: number;
  min_length?: number;
  max_length?: number;
};
type ParamSchema = { properties?: Record<string, ParamDef>; additional_params?: boolean };

type Workflow = {
  workflow_id: string;
  name?: string;
  source?: string;
  workflow_hash?: string;
  last_run_at?: number;
  health?: { flag?: string; dominant_fingerprint?: Record<string, any> };
  params?: ParamSchema;
};

const groupNames: Record<string, string> = {
  chatgpt: 'ChatGPT', linkedin: 'LinkedIn', pubmed: 'PubMed', reddit: 'Reddit',
  youtube: 'YouTube', appstore: 'App Store', google_ads_transparency: 'Google Ads',
  apple_ads: 'Apple Ads', meta_ad_library: 'Meta Ad Library', hn: 'Hacker News',
  x: 'X', _smoke: 'Internal', generated: 'Generated',
};

export function workflowGroup(workflow: Workflow): { key: string; label: string } {
  const key = workflow.workflow_id.split('/')[0]?.trim() || 'other';
  const label = groupNames[key] || key.replace(/[-_]+/g, ' ').replace(/\b\w/g, letter => letter.toUpperCase());
  return { key, label };
}

export function filterWorkflows(workflows: Workflow[], query = '', health = '', source = ''): Workflow[] {
  const needle = query.trim().toLowerCase();
  return workflows.filter(workflow => {
    const group = workflowGroup(workflow).label;
    const searchable = `${workflow.name || ''} ${workflow.workflow_id} ${workflow.source || ''} ${group}`.toLowerCase();
    return (!needle || searchable.includes(needle))
      && (!health || (workflow.health?.flag || 'healthy') === health)
      && (!source || (workflow.source || 'unknown') === source);
  });
}

const healthSentence = (health: Workflow['health']): string => {
  const dominant = health?.dominant_fingerprint;
  if (!dominant) return '';
  const date = new Date(dominant.first_seen_at).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  return `failed ${dominant.count}× at step ${(dominant.step_index ?? 0) + 1} (${dominant.error_class}) since ${date}`;
};

const option = (value: string, label = value) => `<option value="${esc(value)}">${esc(label)}</option>`;

export const paramLabel = (name: string): string => name.replace(/[-_]+/g, ' ').replace(/\b\w/g, letter => letter.toUpperCase());

const defaultText = (definition: ParamDef): string => {
  if (definition.default == null) return '';
  if (definition.kind === 'array' && Array.isArray(definition.default)) return definition.default.map(String).join('\n');
  return String(definition.default);
};

const paramField = (name: string, definition: ParamDef): string => {
  const label = paramLabel(name);
  const required = definition.required ? ' required' : '';
  const marker = definition.required ? '<span class="required-mark">Required</span>' : '<span class="optional-mark">Optional</span>';
  const help = definition.description ? `<small class="field-help">${esc(definition.description)}</small>` : '';
  const value = defaultText(definition);
  const common = `data-param-name="${esc(name)}"${required}`;
  let control: string;
  if (definition.enum_values?.length) {
    control = `<select ${common}>${definition.default == null ? `<option value="">${definition.required ? 'Select…' : 'Not set'}</option>` : ''}${definition.enum_values.map(item => `<option value="${esc(String(item))}" ${String(item) === value ? 'selected' : ''}>${esc(String(item))}</option>`).join('')}</select>`;
  } else if (definition.kind === 'boolean') {
    control = `<select ${common}>${definition.default == null ? `<option value="">${definition.required ? 'Select…' : 'Not set'}</option>` : ''}<option value="true" ${value === 'true' ? 'selected' : ''}>Yes</option><option value="false" ${value === 'false' ? 'selected' : ''}>No</option></select>`;
  } else if (definition.kind === 'array') {
    control = `<textarea ${common} rows="3" placeholder="One item per line">${esc(value)}</textarea><small class="field-hint">Enter one item per line.</small>`;
  } else if (definition.kind === 'object') {
    control = `<textarea ${common} rows="3" placeholder="key=value, one per line">${esc(value)}</textarea><small class="field-hint">Enter one key=value pair per line.</small>`;
  } else if (definition.kind === 'integer' || definition.kind === 'number') {
    control = `<input ${common} type="number" ${definition.kind === 'integer' ? 'step="1"' : 'step="any"'}${definition.min != null ? ` min="${definition.min}"` : ''}${definition.max != null ? ` max="${definition.max}"` : ''} value="${esc(value)}">`;
  } else if (/body|markdown|message|comment|description|prompt|text/i.test(name)) {
    control = `<textarea ${common} rows="4"${definition.min_length != null ? ` minlength="${definition.min_length}"` : ''}${definition.max_length != null ? ` maxlength="${definition.max_length}"` : ''}>${esc(value)}</textarea>`;
  } else {
    control = `<input ${common} type="${definition.sensitive ? 'password' : 'text'}"${definition.min_length != null ? ` minlength="${definition.min_length}"` : ''}${definition.max_length != null ? ` maxlength="${definition.max_length}"` : ''} value="${esc(value)}">`;
  }
  return `<label class="parameter-field"><span class="parameter-label">${esc(label)} ${marker}</span>${control}${help}</label>`;
};

export function normalizeWorkflowParams(schema: ParamSchema | undefined, values: Record<string, string>): Record<string, unknown> {
  const output: Record<string, unknown> = {};
  for (const [name, definition] of Object.entries(schema?.properties || {})) {
    const raw = values[name]?.trim() || '';
    if (!raw) {
      if (definition.default != null) output[name] = definition.default;
      else if (definition.required) throw new Error(`${paramLabel(name)} is required.`);
      continue;
    }
    if (definition.kind === 'integer') {
      const parsed = Number(raw); if (!Number.isInteger(parsed)) throw new Error(`${paramLabel(name)} must be a whole number.`); output[name] = parsed;
    } else if (definition.kind === 'number') {
      const parsed = Number(raw); if (!Number.isFinite(parsed)) throw new Error(`${paramLabel(name)} must be a number.`); output[name] = parsed;
    } else if (definition.kind === 'boolean') output[name] = raw === 'true';
    else if (definition.kind === 'array') output[name] = raw.split('\n').map(item => item.trim()).filter(Boolean);
    else if (definition.kind === 'object') {
      output[name] = Object.fromEntries(raw.split('\n').map(line => line.split('=', 2).map(part => part.trim())).filter(([key, value]) => key && value != null));
    } else output[name] = raw;
  }
  return output;
}

export async function mountWorkflows(root: HTMLElement, call: RpcClient, _route: DashboardRoute, options: { navigate: (hash: string) => void }): Promise<void> {
  root.innerHTML = '<p class="muted">Loading workflows…</p>';
  const [response, snapshot] = await Promise.all([call<any>('workflows.list', {}), call<any>('status.snapshot', {}).catch(() => ({ paused: false }))]);
  const paused = isPaused(snapshot);
  const workflows: Workflow[] = response.workflows || [];
  const groups = new Map<string, { label: string; workflows: Array<{ workflow: Workflow; index: number }> }>();
  workflows.forEach((workflow, index) => {
    const group = workflowGroup(workflow);
    const entry = groups.get(group.key) || { label: group.label, workflows: [] };
    entry.workflows.push({ workflow, index }); groups.set(group.key, entry);
  });
  const orderedGroups = [...groups.entries()].sort((a, b) => a[1].label.localeCompare(b[1].label));
  const sources = [...new Set(workflows.map(workflow => workflow.source || 'unknown'))].sort();

  const rows = (items: Array<{ workflow: Workflow; index: number }>) => items.map(({ workflow, index }) => {
    const flag = workflow.health?.flag || 'healthy';
    const sentence = healthSentence(workflow.health);
    const parameterFields = Object.entries(workflow.params?.properties || {}).map(([name, definition]) => paramField(name, definition)).join('');
    return `<tr class="workflow-row" data-workflow-row data-index="${index}" data-health="${esc(flag)}" data-source="${esc(workflow.source || 'unknown')}">
      <td class="primary-cell"><strong>${esc(workflow.name || workflow.workflow_id)}</strong><code>${esc(workflow.workflow_id)}</code>${sentence ? `<p class="health-sentence">${esc(sentence)}</p>` : ''}</td>
      <td><span class="status">${dot(flag)} ${esc(flag)}</span></td>
      <td>${relativeTime(workflow.last_run_at)}</td>
      <td><span class="chip">${esc(workflow.source || 'unknown')}</span></td>
      <td class="actions"><button class="secondary" data-run-now="${index}" ${paused ? 'disabled title="Automation is paused."' : ''}>Run</button></td>
    </tr>
    <tr class="workflow-panel" data-panel="${index}" hidden><td colspan="5"><form class="run-form" data-form="${index}" data-workflow="${esc(workflow.workflow_id)}">
      ${parameterFields ? `<div class="parameter-grid">${parameterFields}</div>` : '<p class="muted">This workflow does not require any parameters.</p>'}
      <div class="form-actions"><button type="submit">Start run</button><p class="error" data-error></p></div>
    </form></td></tr>`;
  }).join('');

  root.innerHTML = `<section><header class="page-header"><div><h1>Workflows</h1><p>Browse and run workflows available on this device.</p></div></header>
    ${workflows.length ? `<div class="filters" aria-label="Workflow filters">
      <label class="control search"><span class="control-label">Search</span><input type="search" data-workflow-search placeholder="Name, ID, or group"></label>
      <label class="control"><span class="control-label">Health</span><select data-workflow-health>${option('', 'All health')}${['healthy', 'degraded', 'broken'].map(value => option(value)).join('')}</select></label>
      <label class="control"><span class="control-label">Source</span><select data-workflow-source>${option('', 'All sources')}${sources.map(value => option(value)).join('')}</select></label>
      <span class="section-count" data-workflow-count>${workflows.length} workflows</span>
    </div>
    <div data-workflow-groups>${orderedGroups.map(([key, group]) => `<section class="workflow-group" data-group="${esc(key)}"><div class="section-heading"><h2>${esc(group.label)}</h2><span class="section-count" data-group-count>${group.workflows.length} workflows</span></div><div class="table-wrap"><table class="workflow-table"><thead><tr><th>Workflow</th><th>Health</th><th>Last run</th><th>Source</th><th></th></tr></thead><tbody>${rows(group.workflows)}</tbody></table></div></section>`).join('')}</div>
    <div class="empty" data-no-results hidden>No workflows match these filters.</div>` : '<div class="empty">No workflows are available on this device.</div>'}</section>`;

  const applyFilters = (): void => {
    const query = root.querySelector<HTMLInputElement>('[data-workflow-search]')?.value || '';
    const health = root.querySelector<HTMLSelectElement>('[data-workflow-health]')?.value || '';
    const source = root.querySelector<HTMLSelectElement>('[data-workflow-source]')?.value || '';
    const visibleIndexes = new Set(filterWorkflows(workflows, query, health, source).map(workflow => workflows.indexOf(workflow)));
    root.querySelectorAll<HTMLTableRowElement>('[data-workflow-row]').forEach(row => {
      const visible = visibleIndexes.has(Number(row.dataset.index)); row.hidden = !visible;
      if (!visible) { const panel = root.querySelector<HTMLTableRowElement>(`[data-panel="${row.dataset.index}"]`); if (panel) panel.hidden = true; }
    });
    root.querySelectorAll<HTMLElement>('.workflow-group').forEach(group => {
      const visible = [...group.querySelectorAll<HTMLTableRowElement>('[data-workflow-row]')].filter(row => !row.hidden).length;
      group.hidden = visible === 0; const count = group.querySelector<HTMLElement>('[data-group-count]'); if (count) count.textContent = `${visible} workflow${visible === 1 ? '' : 's'}`;
    });
    const count = visibleIndexes.size; const total = root.querySelector<HTMLElement>('[data-workflow-count]'); if (total) total.textContent = `${count} of ${workflows.length}`;
    const empty = root.querySelector<HTMLElement>('[data-no-results]'); if (empty) empty.hidden = count !== 0;
  };

  root.querySelector<HTMLInputElement>('[data-workflow-search]')?.addEventListener('input', applyFilters);
  root.querySelector<HTMLSelectElement>('[data-workflow-health]')?.addEventListener('change', applyFilters);
  root.querySelector<HTMLSelectElement>('[data-workflow-source]')?.addEventListener('change', applyFilters);
  root.querySelectorAll<HTMLButtonElement>('[data-run-now]').forEach(button => button.addEventListener('click', () => { const panel = root.querySelector<HTMLTableRowElement>(`[data-panel="${button.dataset.runNow}"]`)!; panel.hidden = !panel.hidden; }));
  root.querySelectorAll<HTMLFormElement>('form.run-form').forEach(form => form.addEventListener('submit', async event => {
    event.preventDefault();
    const error = form.querySelector<HTMLElement>('[data-error]')!;
    let params: Record<string, unknown>;
    try {
      const workflow = workflows.find(item => item.workflow_id === form.dataset.workflow);
      const values = Object.fromEntries([...form.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>('[data-param-name]')].map(field => [field.dataset.paramName!, field.value]));
      params = normalizeWorkflowParams(workflow?.params, values);
    }
    catch (cause) { error.textContent = cause instanceof Error ? cause.message : 'Check the required fields.'; return; }
    try { const run: any = await call('runs.start', { workflow_id: form.dataset.workflow!, params }); options.navigate(`#runs/${encodeURIComponent(run.run_id || run.record?.run_id)}`); }
    catch (cause) { error.textContent = cause instanceof Error ? cause.message : String(cause); }
  }));
}
