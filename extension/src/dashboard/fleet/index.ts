import { DashboardRoute, dot, esc, relativeTime, RpcClient } from '../shared';

const recovery = 'Ask an operator to reactivate this device, or re-enroll with a new code.';
export async function mountFleet(root: HTMLElement, call: RpcClient, _route: DashboardRoute, options: { navigate: (hash: string) => void }): Promise<() => void> {
  let timer: ReturnType<typeof setInterval> | undefined;
  let lastServer = '';
  let enrollment: any;
  const render = async (): Promise<void> => {
    root.innerHTML = '<p>Loading fleet status…</p>';
    const [status, snapshot, runs] = await Promise.all([
      call<any>('fleet.status', {}).catch(() => ({ state: 'disabled' })), call<any>('status.snapshot', {}).catch(() => ({})), call<any>('runs.list', { limit: 200, origin: 'fleet' }).catch(() => ({ runs: [] })),
    ]);
    const fleet = snapshot.fleet || status.fleet || enrollment || status;
    const state = fleet.status || fleet.state || status.state || 'disabled';
    const enrolled = !['disabled', 'unenrolled', 'not_enrolled'].includes(state) || Boolean(fleet.device_id || fleet.tenant_id);
    lastServer = fleet.server_url || lastServer;
    if (!enrolled) {
      root.innerHTML = `<section><h1>Fleet</h1><p>Enrolling lets your team dispatch workflows to this browser.</p><form data-enroll><label>Server URL <input name="server_url" required value="${esc(lastServer)}" placeholder="https://fleet.example"></label><label>Enrollment code <input name="code" required autocomplete="one-time-code"></label><p data-error class="error"></p><button>Enroll device</button></form></section>`;
      root.querySelector<HTMLFormElement>('[data-enroll]')!.addEventListener('submit', async event => { event.preventDefault(); const form = event.currentTarget as HTMLFormElement; const error = form.querySelector<HTMLElement>('[data-error]')!; const values = Object.fromEntries(new FormData(form)); lastServer = String(values.server_url || ''); try { enrollment = await call('fleet.enroll', values); await render(); } catch (cause) { error.textContent = cause instanceof Error ? cause.message : String(cause); } });
      return;
    }
    const rows = runs.runs || []; const failed = rows.filter((run: any) => run.status === 'failed').length; const succeeded = rows.filter((run: any) => run.status === 'succeeded').length;
    const banner = state === 'dormant' || state === 'revoked' ? `<section class="banner ${state}"><b>${esc(state)}</b><p>${recovery}</p><button data-reenroll>Re-enroll</button></section>` : '';
    root.innerHTML = `<section><h1>Fleet</h1>${banner}<article class="device-card"><h2>${esc(fleet.device_name || fleet.device_id || 'Enrolled device')}</h2><p>${dot(state)} <span class="chip">${esc(state)}</span></p><dl><dt>Device ID</dt><dd>${esc(fleet.device_id || '—')}</dd><dt>Tenant</dt><dd>${esc(fleet.tenant_id || fleet.tenant || '—')}</dd><dt>Server</dt><dd>${esc(fleet.server_url || '—')}</dd><dt>Enrolled</dt><dd>${relativeTime(fleet.enrolled_at)}</dd><dt>Last successful poll</dt><dd>${relativeTime(fleet.last_poll_at || fleet.last_poll_ms)}</dd></dl>${fleet.last_poll_error ? `<p class="warning">Last poll failed: ${esc(fleet.last_poll_error)}</p>` : ''}</article><section class="counters"><b>${rows.length}</b> fleet jobs run · <b>${succeeded}</b> succeeded · <b>${failed}</b> failed</section><p>Master pause is controlled in the <a href="popup.html">popup toggle</a>.</p><button data-unenroll>Unenroll</button></section>`;
    root.querySelector('[data-reenroll]')?.addEventListener('click', () => { enrollment = undefined; void call('fleet.unenroll', {}).finally(render); });
    root.querySelector('[data-unenroll]')?.addEventListener('click', async () => { if (!confirm('Unenroll this device? This stops cloud dispatch and deletes its token.')) return; enrollment = undefined; await call('fleet.unenroll', {}); await render(); });
  };
  await render();
  timer = setInterval(() => { if (document.visibilityState !== 'hidden') void render(); }, 5_000);
  return () => { if (timer) clearInterval(timer); };
}
