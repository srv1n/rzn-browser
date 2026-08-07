import { describe, expect, it, vi } from 'vitest';
import { filterWorkflows, mountWorkflows, normalizeWorkflowParams, workflowGroup } from '.';
import { TestElement, TestRoot, route } from '../testDom';
describe('Workflows tab DOM', () => {
  it('groups namespaces and filters across names, ids, health, and source', () => {
    const workflows = [
      { workflow_id: 'linkedin/profile', name: 'Profile', source: 'local', health: { flag: 'healthy' } },
      { workflow_id: 'chatgpt/send', name: 'Send prompt', source: 'server_cache', health: { flag: 'broken' } },
    ];
    expect(workflowGroup(workflows[0])).toEqual({ key: 'linkedin', label: 'LinkedIn' });
    expect(filterWorkflows(workflows, 'chatgpt')).toEqual([workflows[1]]);
    expect(filterWorkflows(workflows, '', 'healthy', 'local')).toEqual([workflows[0]]);
  });
  it('applies search input to workflow rows and group visibility', async () => {
    const root = new TestRoot(); const search = new TestElement(); const count = new TestElement(); const empty = new TestElement();
    const linkedinRow = new TestElement(); linkedinRow.dataset.index = '0'; const chatgptRow = new TestElement(); chatgptRow.dataset.index = '1';
    const linkedinGroup = new TestRoot(); linkedinGroup.all.set('[data-workflow-row]', [linkedinRow]);
    const chatgptGroup = new TestRoot(); chatgptGroup.all.set('[data-workflow-row]', [chatgptRow]);
    root.children.set('[data-workflow-search]', search); root.children.set('[data-workflow-count]', count); root.children.set('[data-no-results]', empty);
    root.all.set('[data-workflow-row]', [linkedinRow, chatgptRow]); root.all.set('.workflow-group', [linkedinGroup, chatgptGroup]);
    const call = vi.fn().mockResolvedValueOnce({ workflows: [
      { workflow_id: 'linkedin/profile', name: 'Profile', source: 'local', health: { flag: 'healthy' } },
      { workflow_id: 'chatgpt/send', name: 'Send prompt', source: 'local', health: { flag: 'healthy' } },
    ] }).mockResolvedValueOnce({ paused: false });
    await mountWorkflows(root as any, call, route('workflows'), { navigate: vi.fn() });
    search.value = 'linkedin'; await search.fire('input');
    expect(linkedinRow.hidden).toBe(false); expect(chatgptRow.hidden).toBe(true);
    expect(linkedinGroup.hidden).toBe(false); expect(chatgptGroup.hidden).toBe(true); expect(count.textContent).toBe('1 of 2');
  });
  it('renders source, flag, and dominant-fingerprint sentence', async () => {
    const root = new TestRoot(); const call = vi.fn().mockResolvedValueOnce({ workflows: [{ workflow_id: 'checkout', name: 'Checkout', source: 'server_cache', workflow_hash: 'abcdef123', last_run_at: 0, health: { flag: 'broken', dominant_fingerprint: { count: 4, step_index: 5, error_class: 'selector_not_found', first_seen_at: Date.UTC(2026, 6, 9) } } }] }).mockResolvedValueOnce({ paused: false });
    await mountWorkflows(root as any, call, route('workflows'), { navigate: vi.fn() }); expect(root.innerHTML).toContain('failed 4× at step 6 (selector_not_found)'); expect(root.innerHTML).toContain('server_cache'); expect(root.innerHTML).toContain('data-workflow-search');
  });
  it('renders typed fields, requirements, descriptions, defaults, and enums from the manifest', async () => {
    const root = new TestRoot(); const call = vi.fn().mockResolvedValueOnce({ workflows: [{ workflow_id: 'bing/images_search', source: 'local', params: { properties: {
      search_query: { kind: 'string', required: true, description: 'Bing Images search query.' },
      limit: { kind: 'integer', required: false, default: 25, min: 1, max: 50 },
      mode: { kind: 'string', required: true, enum_values: ['fast', 'thorough'] },
    } } }] }).mockResolvedValueOnce({ paused: false });
    await mountWorkflows(root as any, call, route('workflows'), { navigate: vi.fn() });
    expect(root.innerHTML).toContain('Search Query'); expect(root.innerHTML).toContain('Required');
    expect(root.innerHTML).toContain('Bing Images search query.'); expect(root.innerHTML).toContain('type="number"');
    expect(root.innerHTML).toContain('min="1"'); expect(root.innerHTML).toContain('<option value="fast"');
    expect(root.innerHTML).not.toContain('Raw JSON');
  });
  it('normalizes typed values and applies defaults without JSON input', () => {
    const schema = { properties: {
      query: { kind: 'string' as const, required: true }, limit: { kind: 'integer' as const, default: 20 },
      download: { kind: 'boolean' as const }, paths: { kind: 'array' as const },
    } };
    expect(normalizeWorkflowParams(schema, { query: 'cats', limit: '', download: 'false', paths: '/a\n/b' })).toEqual({ query: 'cats', limit: 20, download: false, paths: ['/a', '/b'] });
    expect(() => normalizeWorkflowParams(schema, { query: '' })).toThrow('Query is required.');
  });
  it('shows a disabled run-now action while paused', async () => {
    const root = new TestRoot(); const call = vi.fn().mockResolvedValueOnce({ workflows: [{ workflow_id: 'checkout', source: 'local', health: {} }] }).mockResolvedValueOnce({ paused: true });
    await mountWorkflows(root as any, call, route('workflows'), { navigate: vi.fn() }); expect(root.innerHTML).toContain('disabled title="Automation is paused."');
  });
  it('reports a missing required field inline before starting a run', async () => {
    const root = new TestRoot(); const form = new TestRoot(); form.dataset.workflow = 'checkout'; const error = new TestElement(); form.children.set('[data-error]', error); form.all.set('[data-param-name]', []);
    root.all.set('form.run-form', [form]); root.all.set('[data-run-now]', []); const call = vi.fn().mockResolvedValueOnce({ workflows: [{ workflow_id: 'checkout', source: 'local', params: { properties: { query: { kind: 'string', required: true } } } }] }).mockResolvedValueOnce({ paused: false });
    await mountWorkflows(root as any, call, route('workflows'), { navigate: vi.fn() }); await form.fire('submit'); expect(error.textContent).toContain('Query is required'); expect(call).not.toHaveBeenCalledWith('runs.start', expect.anything());
  });
  it('starts a valid run and deep-links to it', async () => {
    const root = new TestRoot(); const button = new TestElement(); button.dataset.runNow = '0'; const panel = new TestElement(); const form = new TestRoot(); form.dataset.workflow = 'checkout'; const size = new TestElement(); size.dataset.paramName = 'size'; size.value = '2'; const error = new TestElement(); form.children.set('[data-error]', error); form.all.set('[data-param-name]', [size]); root.children.set('[data-panel="0"]', panel); root.all.set('[data-run-now]', [button]); root.all.set('form.run-form', [form]);
    const navigate = vi.fn(); const call = vi.fn().mockResolvedValueOnce({ workflows: [{ workflow_id: 'checkout', source: 'local', health: {}, params: { properties: { size: { kind: 'integer', required: true } } } }] }).mockResolvedValueOnce({ paused: false }).mockResolvedValueOnce({ run_id: 'new-run' });
    await mountWorkflows(root as any, call, route('workflows'), { navigate }); await button.fire('click'); await form.fire('submit'); expect(call).toHaveBeenCalledWith('runs.start', { workflow_id: 'checkout', params: { size: 2 } }); expect(navigate).toHaveBeenCalledWith('#runs/new-run');
  });
});
