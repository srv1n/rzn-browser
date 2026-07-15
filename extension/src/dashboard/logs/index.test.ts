import { describe, expect, it, vi } from 'vitest';
import { mountLogs } from '.';
import { TestElement, TestRoot, route } from '../testDom';
describe('Logs tab DOM', () => {
  it('prefills the run filter and replaces the bounded log window', async () => {
    const root = new TestRoot(); const lines = new TestElement(); root.children.set('[data-log-lines]', lines); root.children.set('[data-level]', new TestElement()); root.children.set('[data-component]', new TestElement()); root.children.set('[data-run-id]', new TestElement()); root.children.set('[data-auto]', new TestElement()); root.children.set('[data-export]', new TestElement()); root.children.set('[data-export-result]', new TestElement());
    const entries = Array.from({ length: 501 }, (_, index) => ({ ts: index, level: 'info', component: 'run', run_id: 'r/1', message: `line ${index}` })); const call = vi.fn().mockResolvedValue({ entries });
    const dispose = await mountLogs(root as any, call, route('logs', undefined, 'run=r%2F1')); expect(root.innerHTML).toContain('value="r/1"'); expect(lines.textContent.split('\n')).toHaveLength(500); expect(call).toHaveBeenCalledWith('logs.tail', expect.objectContaining({ run_id: 'r/1', limit: 500 })); dispose();
  });
});
