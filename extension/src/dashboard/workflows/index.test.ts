import { describe, expect, it, vi } from 'vitest';
import { mountWorkflows } from '.';
import { TestElement, TestRoot, route } from '../testDom';
describe('Workflows tab DOM', () => {
  it('renders source, flag, and dominant-fingerprint sentence', async () => {
    const root = new TestRoot(); const call = vi.fn().mockResolvedValueOnce({ workflows: [{ workflow_id: 'checkout', name: 'Checkout', source: 'server_cache', workflow_hash: 'abcdef123', last_run_at: 0, health: { flag: 'broken', dominant_fingerprint: { count: 4, step_index: 5, error_class: 'selector_not_found', first_seen_at: Date.UTC(2026, 6, 9) } } }] }).mockResolvedValueOnce({ paused: false });
    await mountWorkflows(root as any, call, route('workflows'), { navigate: vi.fn() }); expect(root.innerHTML).toContain('failed 4× at step 6 (selector_not_found)'); expect(root.innerHTML).toContain('server_cache');
  });
  it('shows a disabled run-now action while paused', async () => {
    const root = new TestRoot(); const call = vi.fn().mockResolvedValueOnce({ workflows: [{ workflow_id: 'checkout', source: 'local', health: {} }] }).mockResolvedValueOnce({ paused: true });
    await mountWorkflows(root as any, call, route('workflows'), { navigate: vi.fn() }); expect(root.innerHTML).toContain('disabled title="Automation is paused."');
  });
  it('reports malformed JSON inline before starting a run', async () => {
    const root = new TestRoot(); const form = new TestElement(); const textarea = new TestElement(); textarea.value = '{oops'; const raw = new TestElement(); raw.checked = true; const error = new TestElement(); form.children.set('[data-params]', textarea); form.children.set('[data-raw-json]', raw); form.children.set('[data-error]', error); form.children.set('[data-workflow]', new TestElement());
    root.all.set('form.run-form', [form]); root.all.set('[data-run-now]', []); const call = vi.fn().mockResolvedValueOnce({ workflows: [{ workflow_id: 'checkout', source: 'local', health: {} }] }).mockResolvedValueOnce({ paused: false });
    await mountWorkflows(root as any, call, route('workflows'), { navigate: vi.fn() }); await form.fire('submit'); expect(error.textContent).toContain('Invalid JSON'); expect(call).not.toHaveBeenCalledWith('runs.start', expect.anything());
  });
  it('starts a valid run and deep-links to it', async () => {
    const root = new TestRoot(); const button = new TestElement(); button.dataset.runNow = '0'; const form = new TestElement(); const textarea = new TestElement(); textarea.value = '{"size":2}'; const raw = new TestElement(); raw.checked = true; const error = new TestElement(); const row = new TestElement(); row.dataset.workflow = 'checkout'; form.children.set('[data-params]', textarea); form.children.set('[data-raw-json]', raw); form.children.set('[data-error]', error); form.children.set('__closest', row); root.children.set('[data-form="0"]', form); root.all.set('[data-run-now]', [button]); root.all.set('form.run-form', [form]);
    const navigate = vi.fn(); const call = vi.fn().mockResolvedValueOnce({ workflows: [{ workflow_id: 'checkout', source: 'local', health: {} }] }).mockResolvedValueOnce({ paused: false }).mockResolvedValueOnce({ run_id: 'new-run' });
    await mountWorkflows(root as any, call, route('workflows'), { navigate }); await button.fire('click'); await form.fire('submit'); expect(call).toHaveBeenCalledWith('runs.start', { workflow_id: 'checkout', params: { size: 2 } }); expect(navigate).toHaveBeenCalledWith('#runs/new-run');
  });
});
