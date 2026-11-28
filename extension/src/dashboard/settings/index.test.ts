import { afterEach, describe, expect, it, vi } from 'vitest';
import { mountSettings, siteEntry } from '.';
import { TestElement, TestRoot, route } from '../testDom';

const stored = { run_retention_count: 500, run_retention_days: 30, notifications_enabled: true, notify_on: 'all', fleet_keep_window_on_failure: false, capture_sites: ['example.com'], capture_index: 'my-index' };
const helper = (settings: any) => vi.fn(async (method: string): Promise<any> => method === 'account.status' ? { device_id: 'dev-1', server_url: 'https://app.rzn.ai' } : method === 'status.snapshot' ? { supervisor_version: '1.2.3', paused: false } : settings());

describe('Settings tab DOM', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('leads with saving pages and keeps every other control under one collapsed Advanced', async () => {
    const root = new TestRoot(); root.children.set('[data-settings]', new TestElement());
    await mountSettings(root as any, helper(() => stored), route('settings'));
    const html = root.innerHTML; const advancedAt = html.indexOf('<details class="advanced" data-advanced >');
    expect(advancedAt).toBeGreaterThan(0);
    const [main, advanced] = [html.slice(0, advancedAt), html.slice(advancedAt)];
    for (const name of ['capture_sites', 'capture_index']) expect(main).toContain(`name="${name}"`);
    expect(main).not.toMatch(/fleet|automation|workflow|server|job|token|log/i);
    for (const text of ['name="server_url"', 'name="capture_source"', 'Retention days', 'Failures only', 'Poll interval', 'dev-1', '1.2.3', 'href="#runs"', 'href="#workflows"', 'href="#logs"', 'data-pause']) expect(advanced).toContain(text);
  });

  it('rolls an optimistic save back to persisted values after an RPC failure', async () => {
    vi.stubGlobal('FormData', class extends Map<any, any> { constructor(_form?: unknown) { super(); this.set('run_retention_count', '20'); this.set('run_retention_days', '4'); this.set('notifications_enabled', 'on'); this.set('notify_on', 'all'); this.set('capture_sites', ' example.com\n\nlambda.ai '); } });
    const root = new TestRoot(); const form = new TestElement(); root.children.set('[data-settings]', form);
    let saves = 0; const call = helper(() => { if (saves++ === 0) return stored; throw new Error('disk full'); });
    await mountSettings(root as any, call, route('settings')); await form.fire('submit');
    expect(call).toHaveBeenCalledWith('settings.set', expect.objectContaining({ patch: expect.objectContaining({ run_retention_count: 20, capture_sites: ['example.com', 'lambda.ai'] }) }));
    expect(root.innerHTML).toContain('Couldn&#39;t save. disk full'); expect(root.innerHTML).toContain('value="500"');
  });

  it('lists each site as a row with a switch, and keeps the list in the saved field', async () => {
    const root = new TestRoot(); root.children.set('[data-settings]', new TestElement());
    await mountSettings(root as any, helper(() => ({ ...stored, capture_sites: ['example.com', 'lambda.ai'] })), route('settings'));
    expect(root.innerHTML.match(/class="site-row"/g)).toHaveLength(2);
    expect(root.innerHTML).toContain('data-site="lambda.ai" checked');
    expect(root.innerHTML).toContain('name="capture_sites" value="example.com\nlambda.ai"');
  });

  it('turns a typed address into the bare site', () => {
    expect(siteEntry(' https://News.Example.com:8443/a?b ')).toBe('news.example.com');
    expect(siteEntry('lambda.ai/')).toBe('lambda.ai');
    expect(siteEntry('  ')).toBe('');
  });
});
