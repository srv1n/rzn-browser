import { describe, expect, it, vi } from 'vitest';
import { mountRuns } from '.';
import { TestElement, TestRoot, route } from '../testDom';

const run = (id: string, status = 'failed') => ({ run_id: id, workflow_id: 'checkout', origin: 'fleet:job-1', status, started_at: 1_000, ended_at: 3_000, failing_step_index: 2, error_class: 'selector_not_found', fingerprint: 'fp', error_message: 'button missing' });
describe('Runs tab DOM', () => {
  it('renders a 50-row page and advances pagination through the mocked RPC', async () => {
    const root = new TestRoot(); const next = new TestElement(); root.children.set('[data-page="next"]', next);
    const call = vi.fn().mockResolvedValueOnce({ total: 51, runs: [run('one')] }).mockResolvedValueOnce({ workflows: [{ workflow_id: 'checkout' }] }).mockResolvedValueOnce({ total: 51, runs: [run('two')] }).mockResolvedValueOnce({ workflows: [] });
    await mountRuns(root as any, call, route('runs'), { navigate: vi.fn() });
    expect(root.innerHTML).toContain('Find on this page'); expect(root.innerHTML).toContain('fleet'); expect(call).toHaveBeenCalledWith('runs.list', expect.objectContaining({ limit: 50, offset: 0 }));
    await next.fire('click'); expect(call).toHaveBeenCalledWith('runs.list', expect.objectContaining({ offset: 50 }));
  });
  it('renders failure context and re-runs to the returned deep link', async () => {
    const root = new TestRoot(); const rerun = new TestElement(); root.children.set('[data-rerun]', rerun); root.children.set('[data-copy-result]', new TestElement()); root.children.set('[data-action-error]', new TestElement());
    const navigate = vi.fn(); const call = vi.fn()
      .mockResolvedValueOnce({ ok: true, record: run('bad'), result: { steps: [{ step_id: 'find', status: 'failed', duration_ms: 30 }], output: { x: 1 } } })
      .mockResolvedValueOnce({ paused: false }).mockResolvedValueOnce({ runs: [run('older'), run('bad')] }).mockResolvedValueOnce({ workflows: [{ workflow_id: 'checkout' }] })
      .mockResolvedValueOnce({ console_tail: 'oops', screenshot_b64: 'AAAA', dom_excerpt: '<button>' }).mockResolvedValueOnce({ run_id: 'retry' });
    await mountRuns(root as any, call, route('runs', 'bad'), { navigate });
    expect(root.innerHTML).toContain('Console tail'); expect(root.innerHTML).toContain('Failure screenshot'); expect(root.innerHTML).toContain('selector_not_found');
    await rerun.fire('click'); expect(call).toHaveBeenCalledWith('runs.replay', { run_id: 'bad' }); expect(navigate).toHaveBeenCalledWith('#runs/retry');
  });
  it('disables re-run when paused', async () => {
    const root = new TestRoot(); root.children.set('[data-rerun]', new TestElement()); root.children.set('[data-copy-result]', new TestElement()); root.children.set('[data-action-error]', new TestElement());
    const call = vi.fn().mockResolvedValueOnce({ ok: true, record: run('bad'), result: {} }).mockResolvedValueOnce({ paused: true }).mockResolvedValueOnce({ runs: [] }).mockResolvedValueOnce({ workflows: [{ workflow_id: 'checkout' }] }).mockResolvedValueOnce({ capture_unavailable: 'none' });
    await mountRuns(root as any, call, route('runs', 'bad'), { navigate: vi.fn() }); expect(root.innerHTML).toContain('disabled title="Automation is paused."');
  });
});
