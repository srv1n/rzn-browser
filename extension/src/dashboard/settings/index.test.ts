import { afterEach, describe, expect, it, vi } from 'vitest';
import { mountSettings } from '.';
import { TestElement, TestRoot, route } from '../testDom';
describe('Settings tab DOM', () => {
  afterEach(() => vi.unstubAllGlobals());
  it('shows the persisted settings and exposes all controls', async () => {
    const root = new TestRoot(); const form = new TestElement(); root.children.set('[data-settings]', form); const call = vi.fn().mockResolvedValue({ run_retention_count: 500, run_retention_days: 30, notifications_enabled: true, notify_on: 'all', fleet_keep_window_on_failure: false });
    await mountSettings(root as any, call, route('settings')); expect(root.innerHTML).toContain('Retention days'); expect(root.innerHTML).toContain('Failures only'); expect(root.innerHTML).toContain('Poll interval');
  });
  it('rolls an optimistic save back to persisted values after an RPC failure', async () => {
    vi.stubGlobal('FormData', class extends Map<any, any> { constructor(_form?: unknown) { super(); this.set('run_retention_count', '20'); this.set('run_retention_days', '4'); this.set('notifications_enabled', 'on'); this.set('notify_on', 'all'); } });
    const root = new TestRoot(); const form = new TestElement(); form.children.set('button', new TestElement()); form.children.set('[data-save-message]', new TestElement()); root.children.set('[data-settings]', form); const stored = { run_retention_count: 500, run_retention_days: 30, notifications_enabled: true, notify_on: 'all', fleet_keep_window_on_failure: false }; const call = vi.fn().mockResolvedValueOnce(stored).mockRejectedValueOnce(new Error('disk full'));
    await mountSettings(root as any, call, route('settings')); await form.fire('submit'); expect(call).toHaveBeenCalledWith('settings.set', expect.objectContaining({ patch: expect.objectContaining({ run_retention_count: 20 }) })); expect(root.innerHTML).toContain('Save failed; restored previous values'); expect(root.innerHTML).toContain('value="500"');
  });
});
