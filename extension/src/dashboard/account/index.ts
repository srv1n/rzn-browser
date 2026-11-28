// The Account page (backend docs/plans/browser-fleet/design.md section 3): who is signed in,
// this browser, sign out. Device details live in Settings, under Advanced. Signing in replaces the
// old server URL + enrollment code form; codes remain for headless machines (`rzn-browser fleet enroll`).
import { markSvg } from '../../ui/brand';
import { friendly } from '../../ui/errors';
import { DashboardRoute, esc, RpcClient } from '../shared';

const laptop = '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="4" y="5" width="16" height="11" rx="1.5"/><path d="M2 19h20"/></svg>';
import { DEFAULT_SERVER_URL as DEFAULT_SERVER, serverUrl, signIn } from './signin';

type SignIn = (server: string, call: RpcClient) => Promise<unknown>;

export async function mountAccount(root: HTMLElement, call: RpcClient, _route: DashboardRoute, options: { signIn?: SignIn } = {}): Promise<() => void> {
  const startSignIn = options.signIn || ((server: string, rpc: RpcClient) => signIn(server, rpc));
  let timer: ReturnType<typeof setTimeout> | undefined; let disposed = false; let busy = false; let shown = '';
  // The page refreshes every 5 s; replacing unchanged markup would drop keyboard focus.
  const show = (html: string): boolean => { if (html === shown) return false; shown = html; root.innerHTML = html; return true; };
  const render = async (message = ''): Promise<void> => {
    if (busy) return;
    const [accountResult, settingsResult] = await Promise.allSettled([call<any>('account.status', {}), call<any>('settings.get', {})]);
    if (accountResult.status === 'rejected') {
      const { title, line } = friendly(accountResult.reason, 'RZN account');
      if (!show(`<section class="narrow"><div class="card empty"><h1>${esc(title)}</h1><p class="lede">${esc(line)}</p><div class="actions"><button class="primary" data-retry>Try again</button></div></div></section>`)) return;
      root.querySelector('[data-retry]')?.addEventListener('click', () => { void render(); });
      return;
    }
    const account = accountResult.value || {};
    const server = account.server_url || serverUrl(settingsResult.status === 'fulfilled' ? settingsResult.value : undefined);
    const state = account.fleet?.state || 'disabled';
    const stopped = ['stopped_revoked', 'stopped_dormant', 'revoked', 'dormant'].includes(state);
    const revoked = state.includes('revoked');
    const error = message ? `<p data-error class="alert bad" role="alert">${esc(message)}</p>` : '';
    let html: string;
    if (!account.signed_in) {
      const custom = server !== DEFAULT_SERVER ? `<p class="muted small">Using the server ${esc(server.replace(/^https?:\/\//, ''))}. Change it in Settings, under Advanced.</p>` : '';
      html = `<section class="narrow"><div class="card welcome"><span class="welcome-mark">${markSvg(44)}</span><h1>Sign in to RZN</h1><p class="lede">Connect this browser to your RZN account to save the pages you choose and keep them searchable.</p>${error}<div class="actions"><button class="primary large" data-sign-in>Sign in to RZN</button></div><p class="muted small">A sign-in page opens in a new tab. You come back here when you're done.</p>${custom}</div></section>`;
    } else {
      const who = account.user_email || account.user_id || '';
      const status = stopped ? `<span class="chip ${revoked ? 'bad' : 'warn'}">${revoked ? 'Signed out from the app' : 'Inactive'}</span>` : '<span class="chip ok">Connected</span>';
      const banner = stopped ? `<div class="banner ${revoked ? 'bad' : 'warn'}" role="status"><p><b>This browser is ${revoked ? 'signed out from the app' : 'inactive'}.</b> Sign in again to keep saving pages.</p><button class="primary" data-sign-in>Sign in again</button></div>` : '';
      html = `<section class="narrow"><header class="page-head"><h1>Account</h1><p class="lede">Who this browser saves pages for.</p></header>${banner}${error}
<div class="card user"><span class="avatar lg">${esc((who.match(/[a-z0-9]/i)?.[0] || '?').toUpperCase())}</span><div class="user-text"><span class="label">Signed in as</span><span class="user-email">${esc(who)}</span></div>${status}</div>
<div class="card"><div class="card-head"><h2>This browser</h2><p>Pages you switch on here are saved to your account.</p></div><div class="device-row"><span class="avatar site">${laptop}</span><div class="device-text"><b>${esc(account.device_name || 'This browser')}</b><span class="muted small">RZN at ${esc(server.replace(/^https?:\/\//, ''))}</span></div><button data-sign-out>Sign out</button></div></div></section>`;
    }
    if (!show(html)) return;
    root.querySelector('[data-sign-in]')?.addEventListener('click', async () => {
      busy = true;
      try { if (account.enrolled) await call('account.sign_out', {}); await startSignIn(server, call); busy = false; await render(); }
      catch (cause) { busy = false; await render(`Couldn't sign in. ${friendly(cause, 'RZN account').line}`); }
    });
    root.querySelector('[data-sign-out]')?.addEventListener('click', async () => {
      if (!confirm('Sign out of RZN on this browser? Pages stop saving until you sign in again.')) return;
      await call('account.sign_out', {}); await render();
    });
  };
  await render();
  const poll = async () => { if (document.visibilityState !== 'hidden') await render(); if (!disposed) timer = setTimeout(poll, 5_000); };
  timer = setTimeout(poll, 5_000);
  return () => { disposed = true; if (timer) clearTimeout(timer); };
}
