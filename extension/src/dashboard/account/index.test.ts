import { afterEach, describe, expect, it, vi } from 'vitest';
import { mountAccount } from '.';
import { authorizeUrl, codeFrom, pkce, redirectUri, serverUrl, waitForCode } from './signin';
import { TestElement, TestRoot, route } from '../testDom';

const signedIn = { signed_in: true, enrolled: true, user_email: 'me@example.com', user_id: 'u1', device_id: 'dev-1', device_name: 'mac', server_url: 'https://app.rzn.ai', fleet: { state: 'polling', last_poll_ms: 1 } };

describe('Account tab', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('offers sign-in, not a server URL and code, when signed out', async () => {
    const root = new TestRoot(); const button = new TestElement(); root.children.set('[data-sign-in]', button);
    const call: any = vi.fn(async (method: string): Promise<any> => method === 'account.status' ? { signed_in: false } : method === 'settings.get' ? { server_url: '' } : { runs: [] });
    const signIn = vi.fn().mockResolvedValue({ ok: true });
    const dispose = await mountAccount(root as any, call, route('account'), { signIn });
    expect(root.innerHTML).toContain('Sign in to RZN'); expect(root.innerHTML).toContain('https://app.rzn.ai'); expect(root.innerHTML).not.toContain('Enrollment code');
    await button.fire('click');
    expect(signIn).toHaveBeenCalledWith('https://app.rzn.ai', call);
    dispose();
  });

  it('uses the Settings server override', async () => {
    const root = new TestRoot(); const button = new TestElement(); root.children.set('[data-sign-in]', button);
    const call: any = vi.fn(async (method: string): Promise<any> => method === 'account.status' ? { signed_in: false } : method === 'settings.get' ? { server_url: 'https://build1-api.rzn.ai/' } : { runs: [] });
    const signIn = vi.fn().mockResolvedValue({});
    const dispose = await mountAccount(root as any, call, route('account'), { signIn }); await button.fire('click');
    expect(signIn).toHaveBeenCalledWith('https://build1-api.rzn.ai', call); dispose();
  });

  it('shows who, this browser, recent jobs and sign out', async () => {
    const root = new TestRoot();
    const call: any = vi.fn(async (method: string): Promise<any> => method === 'account.status' ? signedIn : method === 'settings.get' ? {} : { runs: [{ workflow_id: 'x/open', status: 'succeeded', started_at: 1 }] });
    const dispose = await mountAccount(root as any, call, route('account'));
    for (const text of ['me@example.com', 'dev-1', 'x/open', 'Recent jobs', 'Sign out']) expect(root.innerHTML).toContain(text);
    dispose();
  });

  it('a browser revoked in the app is told and signs in again', async () => {
    const root = new TestRoot(); const button = new TestElement(); root.children.set('[data-sign-in]', button);
    const call: any = vi.fn(async (method: string): Promise<any> => method === 'account.status' ? { ...signedIn, fleet: { state: 'stopped_revoked' } } : method === 'settings.get' ? {} : { runs: [] });
    const signIn = vi.fn().mockResolvedValue({});
    const dispose = await mountAccount(root as any, call, route('account'), { signIn });
    expect(root.innerHTML).toContain('signed out from the app');
    await button.fire('click');
    expect(call).toHaveBeenCalledWith('account.sign_out', {}); expect(signIn).toHaveBeenCalled(); dispose();
  });
});

describe('sign-in flow', () => {
  const redirect = redirectUri('bogjdnehdficgkhklinmnbgiiofbamji');

  it('asks for a PKCE S256 code as client rzn-browser', async () => {
    const { verifier, challenge } = await pkce();
    expect(verifier.length).toBeGreaterThanOrEqual(43); expect(challenge).toMatch(/^[A-Za-z0-9_-]{43}$/);
    const url = new URL(authorizeUrl('https://app.rzn.ai', challenge, 'st', redirect));
    expect(url.pathname).toBe('/oauth/authorize');
    expect(Object.fromEntries(url.searchParams)).toEqual({ response_type: 'code', client_id: 'rzn-browser', redirect_uri: 'https://bogjdnehdficgkhklinmnbgiiofbamji.chromiumapp.org/', code_challenge: challenge, code_challenge_method: 'S256', state: 'st' });
    expect(serverUrl({ server_url: ' ' })).toBe('https://app.rzn.ai');
  });

  it('takes the code from the return address only, with our state', () => {
    expect(codeFrom('https://app.rzn.ai/oauth/authorize?x', redirect, 'st')).toBeNull();
    expect(codeFrom(`${redirect}?code=abc&state=st`, redirect, 'st')).toBe('abc');
    expect(() => codeFrom(`${redirect}?code=abc&state=other`, redirect, 'st')).toThrow('different request');
  });

  it('closes the sign-in tab when it reaches the return address', async () => {
    const updated: any[] = []; const removed: any[] = [];
    const tabs: any = { create: vi.fn().mockResolvedValue({ id: 7 }), remove: vi.fn().mockResolvedValue(undefined), onUpdated: { addListener: (f: any) => updated.push(f), removeListener: vi.fn() }, onRemoved: { addListener: (f: any) => removed.push(f), removeListener: vi.fn() } };
    const pending = waitForCode(tabs, 'https://app.rzn.ai/oauth/authorize', redirect, 'st');
    await Promise.resolve(); await Promise.resolve();
    updated[0](7, { url: 'https://app.rzn.ai/oauth/authorize' });
    updated[0](8, { url: `${redirect}?code=other&state=st` });
    updated[0](7, {}, { url: `${redirect}?code=abc&state=st` });
    await expect(pending).resolves.toBe('abc'); expect(tabs.remove).toHaveBeenCalledWith(7);
  });
});
