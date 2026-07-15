import { afterEach, describe, expect, it, vi } from 'vitest';
import { mountFleet } from '.';
import { TestElement, TestRoot, route } from '../testDom';
describe('Fleet tab DOM', () => {
  afterEach(() => vi.unstubAllGlobals());
  it('renders enrollment for an unenrolled device', async () => {
    const root = new TestRoot(); root.children.set('[data-enroll]', new TestElement()); const call = vi.fn().mockResolvedValueOnce({ state: 'disabled' }).mockResolvedValueOnce({}).mockResolvedValueOnce({ runs: [] });
    const dispose = await mountFleet(root as any, call, route('fleet'), { navigate: vi.fn() }); expect(root.innerHTML).toContain('Enroll device'); dispose();
  });
  it.each(['dormant', 'revoked'])('renders %s recovery guidance and fleet counters', async state => {
    const root = new TestRoot(); const call = vi.fn().mockResolvedValueOnce({ state }).mockResolvedValueOnce({ fleet: { state, device_id: 'dev', tenant_id: 'tenant', last_poll_at: 1, last_poll_error: 'offline' } }).mockResolvedValueOnce({ runs: [{ status: 'succeeded' }, { status: 'failed' }] });
    const dispose = await mountFleet(root as any, call, route('fleet'), { navigate: vi.fn() }); expect(root.innerHTML).toContain('Ask an operator to reactivate this device'); expect(root.innerHTML).toContain('1</b> succeeded'); expect(root.innerHTML).toContain('Last poll failed'); dispose();
  });
  it('enrolls from the form and surfaces an inline server error', async () => {
    vi.stubGlobal('FormData', class extends Map<any, any> { constructor(_form?: unknown) { super(); this.set('server_url', 'https://fleet.test'); this.set('code', 'fresh-code'); } });
    const root = new TestRoot(); const form = new TestElement(); const error = new TestElement(); form.children.set('[data-error]', error); root.children.set('[data-enroll]', form);
    const call = vi.fn().mockResolvedValueOnce({ state: 'disabled' }).mockResolvedValueOnce({}).mockResolvedValueOnce({ runs: [] }).mockRejectedValueOnce(new Error('code expired'));
    const dispose = await mountFleet(root as any, call, route('fleet'), { navigate: vi.fn() }); await form.fire('submit'); expect(call).toHaveBeenCalledWith('fleet.enroll', { server_url: 'https://fleet.test', code: 'fresh-code' }); expect(error.textContent).toContain('code expired'); dispose();
  });
});
