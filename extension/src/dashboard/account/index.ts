// The Account page (backend docs/plans/browser-fleet/design.md section 3): who is signed in,
// this browser, sign out. Device details live in Settings, under Advanced. Signing in replaces the
// old server URL + enrollment code form; codes remain for headless machines (`rzn-browser fleet enroll`).
import { DashboardRoute, esc, RpcClient } from '../shared';
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
      if (!show('<section class="narrow"><div class="empty"><span class="label">Not connected</span><h1>RZN isn\'t <em>answering</em></h1><p>The RZN helper on this computer didn\'t respond. Make sure RZN is running, then try again.</p><button class="primary" data-retry>Try again</button></div></section>')) return;
      root.querySelector('[data-retry]')?.addEventListener('click', () => { void render(); });
      return;
    }
    const account = accountResult.value || {};
    const server = account.server_url || serverUrl(settingsResult.status === 'fulfilled' ? settingsResult.value : undefined);
    const state = account.fleet?.state || 'disabled';
    const stopped = ['stopped_revoked', 'stopped_dormant', 'revoked', 'dormant'].includes(state);
    const revoked = state.includes('revoked');
    const error = message ? `<p data-error class="error" role="alert">${esc(message)}</p>` : '';
    let html: string;
    if (!account.signed_in) {
      const custom = server !== DEFAULT_SERVER ? `<p class="muted small">Using the server ${esc(server.replace(/^https?:\/\//, ''))}. Change it in Settings, under Advanced.</p>` : '';
      html = `<section class="narrow"><span class="label">Account</span><h1>Sign in to <em>RZN</em></h1><p class="lede">Save the pages you choose to your RZN library and keep them searchable.</p>${error}<div class="actions"><button class="primary" data-sign-in>Sign in to RZN</button></div>${custom}</section>`;
    } else {
      const status = stopped ? `<span class="chip ${revoked ? 'bad' : 'warn'}">${revoked ? 'Signed out from the app' : 'Inactive'}</span>` : '<span class="chip ok">Connected</span>';
      const banner = stopped ? `<div class="banner" role="status"><p><b>This browser is ${revoked ? 'signed out from the app' : 'inactive'}.</b> Sign in again to keep saving pages.</p><button class="primary" data-sign-in>Sign in again</button></div>` : '';
      html = `<section class="narrow"><span class="label">Account</span><h1>Your <em>account</em></h1><p class="lede">Signed in as <b>${esc(account.user_email || account.user_id)}</b></p>${banner}${error}<div class="list"><div class="list-row"><div><span class="label">This browser</span><b>${esc(account.device_name || 'This browser')}</b></div><div class="list-actions">${status}<button data-sign-out>Sign out</button></div></div></div></section>`;
    }
    if (!show(html)) return;
    root.querySelector('[data-sign-in]')?.addEventListener('click', async () => {
      busy = true;
      try { if (account.enrolled) await call('account.sign_out', {}); await startSignIn(server, call); busy = false; await render(); }
      catch (cause) { busy = false; await render(cause instanceof Error ? cause.message : String(cause)); }
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
