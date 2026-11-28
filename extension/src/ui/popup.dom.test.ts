import { describe, expect, it, vi } from 'vitest';
import { bindPopupActions, errorHtml, popupHtml, unreachableHtml, type PopupState, type Snapshot } from './popupView';

const snapshot: Snapshot = { supervisor_version: '1', native_host_connected: true, extension_connected: true, paused: false, now_running: null, fleet: null, recent_runs: [], flagged_workflows: 0 };
const signedIn = { signed_in: true, user_email: 'me@example.com', server_url: 'https://app.rzn.ai', fleet: { state: 'polling' } };
const state = (over: Partial<PopupState> = {}): PopupState => ({ snapshot, account: signedIn, settings: { capture_sites: [], capture_index: 'my-index' }, site: 'example.com', ...over });

class ElementStub {
  listeners: Record<string, (event: any) => unknown> = {}; checked = false; disabled = false; textContent = ''; dataset: Record<string, string> = {};
  addEventListener(type: string, fn: (event: any) => unknown) { this.listeners[type] = fn; }
}
const rootWith = (map: Record<string, ElementStub>, all: ElementStub[] = []) => ({ querySelector: (s: string) => map[s] ?? null, querySelectorAll: () => all }) as unknown as ParentNode;

describe('popup states', () => {
  it('signed out: one friendly sign-in action, no site switch', () => {
    const html = popupHtml(state({ account: { signed_in: false } }));
    expect(html).toContain('Sign in to RZN'); expect(html).not.toContain('capture-switch');
  });

  it('signed in: who, this site switch, where pages go, one primary action', () => {
    const off = popupHtml(state());
    expect(off).toContain('me@example.com'); expect(off).toContain('example.com'); expect(off).toContain('role="switch"');
    expect(off).not.toMatch(/id="capture-switch"[^>]*checked/); expect(off).toContain('Nothing from this site leaves your browser');
    expect(off.match(/class="button primary/g)).toHaveLength(1); expect(off).toContain('href="https://app.rzn.ai"');
    const on = popupHtml(state({ settings: { capture_sites: ['example.com'], capture_index: 'my-index' } }));
    expect(on).toMatch(/id="capture-switch"[^>]*checked/); expect(on).toContain('<b>my-index</b>');
    const parent = popupHtml(state({ site: 'news.example.com', settings: { capture_sites: ['example.com'] } }));
    expect(parent).toContain('via example.com'); expect(parent).toContain('Choose where pages go');
  });

  it('never shows automation words', () => {
    const busy = popupHtml(state({ snapshot: { ...snapshot, now_running: { run_id: 'r', workflow_id: 'x/open' }, fleet: { login_walls: ['linkedin'] } } }));
    for (const html of [busy, popupHtml(state({ site: null })), unreachableHtml, errorHtml(new Error('x'))]) {
      expect(html).not.toMatch(/fleet|automation|supervisor|workflow|job|worker|tenant/i);
    }
    expect(busy).toContain('Sign in to linkedin'); expect(busy).toContain('id="stop"');
  });

  it('a page that is not a website says so', () => { expect(popupHtml(state({ site: null }))).toContain("can't be saved"); });

  it('a revoked browser says it was signed out from the app', () => {
    expect(popupHtml(state({ account: { ...signedIn, fleet: { state: 'stopped_revoked' } } }))).toContain('Signed out from the app');
  });
});

describe('popup actions', () => {
  it('the switch adds this site, or removes the entry covering it', async () => {
    const input = new ElementStub(); input.checked = true;
    const call = vi.fn(async (method: string): Promise<any> => method === 'settings.get' ? { capture_sites: ['lambda.ai'] } : {});
    const refresh = vi.fn();
    bindPopupActions(rootWith({ '#capture-switch': input }), state(), refresh, call as any);
    await input.listeners.change({ target: input });
    expect(call).toHaveBeenCalledWith('settings.set', { patch: { capture_sites: ['lambda.ai', 'example.com'] } }); expect(refresh).toHaveBeenCalled();

    const off = new ElementStub(); off.checked = false;
    const call2 = vi.fn(async (method: string): Promise<any> => method === 'settings.get' ? { capture_sites: ['example.com', 'lambda.ai'] } : {});
    bindPopupActions(rootWith({ '#capture-switch': off }), state({ site: 'news.example.com' }), vi.fn(), call2 as any);
    await off.listeners.change({ target: off });
    expect(call2).toHaveBeenCalledWith('settings.set', { patch: { capture_sites: ['lambda.ai'] } });
  });

  it('a failed switch flips back and says why', async () => {
    const input = new ElementStub(); input.checked = true; const help = new ElementStub();
    const call = vi.fn(async (method: string): Promise<any> => { if (method === 'settings.set') throw new Error('disk full'); return { capture_sites: [] }; });
    const refresh = vi.fn();
    bindPopupActions(rootWith({ '#capture-switch': input, '#capture-help': help }), state(), refresh, call as any);
    await input.listeners.change({ target: input });
    expect(input.checked).toBe(false); expect(help.textContent).toContain('disk full'); expect(refresh).not.toHaveBeenCalled();
  });

  it('Stop and a cleared sign-in wall call the helper', async () => {
    const stop = new ElementStub(); const wall = Object.assign(new ElementStub(), { dataset: { cleared: 'linkedin' } });
    const call = vi.fn().mockResolvedValue({});
    bindPopupActions(rootWith({ '#stop': stop }, [wall]), state({ snapshot: { ...snapshot, now_running: { run_id: 'active' } } }), vi.fn(), call);
    stop.listeners.click({}); wall.listeners.click({});
    expect(call).toHaveBeenCalledWith('runs.cancel', { run_id: 'active' }); expect(call).toHaveBeenCalledWith('fleet.site_cleared', { system: 'linkedin' });
  });
});
