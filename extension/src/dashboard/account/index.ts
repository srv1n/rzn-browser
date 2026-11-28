// The Account page (backend docs/plans/browser-fleet/design.md section 3): who is signed in,
// this browser, its recent jobs, sign out. Signing in replaces the old server URL + enrollment
// code form; codes remain for headless machines (`rzn-browser fleet enroll`).
import { DashboardRoute, dot, esc, relativeTime, RpcClient } from '../shared';
import { serverUrl, signIn } from './signin';

type SignIn = (server: string, call: RpcClient) => Promise<unknown>;

export async function mountAccount(root: HTMLElement, call: RpcClient, _route: DashboardRoute, options: { signIn?: SignIn } = {}): Promise<() => void> {
  const startSignIn = options.signIn || ((server: string, rpc: RpcClient) => signIn(server, rpc));
  let timer: ReturnType<typeof setTimeout> | undefined; let disposed = false; let busy = false;
  const render = async (message = ''): Promise<void> => {
    if (busy) return;
    const [accountResult, settingsResult, runsResult] = await Promise.allSettled([
      call<any>('account.status', {}), call<any>('settings.get', {}), call<any>('runs.list', { limit: 200, origin: 'fleet' }),
    ]);
    if (accountResult.status === 'rejected') {
      root.innerHTML = '<section class="banner warning"><b>Supervisor unreachable</b><p>Check the local runtime, then retry.</p><button data-retry>Retry</button></section>';
      root.querySelector('[data-retry]')?.addEventListener('click', () => { void render(); });
      return;
    }
    const account = accountResult.value || {};
    const server = account.server_url || serverUrl(settingsResult.status === 'fulfilled' ? settingsResult.value : undefined);
    const fleet = account.fleet || {};
    const state = fleet.state || 'disabled';
    const stopped = ['stopped_revoked', 'stopped_dormant', 'revoked', 'dormant'].includes(state);
    const signInButton = (label: string) => `<button data-sign-in>${label}</button>`;
    if (!account.signed_in) {
      root.innerHTML = `<section><header class="page-header"><div><h1>Account</h1><p>Sign in to RZN so this browser can save pages you choose to your index and run your workflows.</p></div></header><p>Server: <code>${esc(server)}</code> <span class="muted">(change it in Settings)</span></p><p data-error class="error">${esc(message)}</p>${signInButton('Sign in to RZN')}</section>`;
    } else {
      const rows: any[] = (runsResult.status === 'fulfilled' && runsResult.value?.runs) || [];
      const recent = rows.slice(0, 10).map(run => `<tr><td>${esc(run.workflow_id)}</td><td>${dot(run.status)} ${esc(run.status)}</td><td>${relativeTime(run.started_at)}</td></tr>`).join('');
      const banner = stopped ? `<section class="banner ${state.includes('revoked') ? 'revoked' : 'dormant'}"><b>This browser is ${state.includes('revoked') ? 'signed out from the app' : 'dormant'}</b><p>Sign in again to resume.</p>${signInButton('Sign in again')}</section>` : '';
      root.innerHTML = `<section><header class="page-header"><div><h1>Account</h1><p>Signed in as <b>${esc(account.user_email || account.user_id)}</b></p></div></header>${banner}<p data-error class="error">${esc(message)}</p><article class="device-card"><div class="section-heading"><h2>This browser: ${esc(account.device_name || account.device_id)}</h2><span class="status">${dot(state)} ${esc(state)}</span></div><dl><dt>Device ID</dt><dd>${esc(account.device_id || '—')}</dd><dt>Server</dt><dd>${esc(account.server_url || '—')}</dd><dt>Last check-in</dt><dd>${relativeTime(fleet.last_poll_ms)}</dd></dl></article><section><h2>Recent jobs</h2>${recent ? `<table><thead><tr><th>Workflow</th><th>Status</th><th>Started</th></tr></thead><tbody>${recent}</tbody></table>` : '<p class="muted">No jobs yet.</p>'}</section><p class="muted">Master pause is controlled from the <a href="popup.html">extension popup</a>.</p><button class="danger" data-sign-out>Sign out</button></section>`;
    }
    root.querySelector('[data-sign-in]')?.addEventListener('click', async () => {
      busy = true;
      try { if (account.enrolled) await call('account.sign_out', {}); await startSignIn(server, call); busy = false; await render(); }
      catch (cause) { busy = false; await render(cause instanceof Error ? cause.message : String(cause)); }
    });
    root.querySelector('[data-sign-out]')?.addEventListener('click', async () => {
      if (!confirm('Sign out? This browser stops taking jobs and saving pages until you sign in again.')) return;
      await call('account.sign_out', {}); await render();
    });
  };
  await render();
  const poll = async () => { if (document.visibilityState !== 'hidden') await render(); if (!disposed) timer = setTimeout(poll, 5_000); };
  timer = setTimeout(poll, 5_000);
  return () => { disposed = true; if (timer) clearTimeout(timer); };
}
