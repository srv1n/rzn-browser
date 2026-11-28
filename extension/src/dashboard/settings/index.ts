import { friendly } from '../../ui/errors';
import { DashboardRoute, esc, relativeTime, RpcClient } from '../shared';

const field = (label: string, control: string, help = '') => `<label class="field"><span class="field-label">${label}</span>${control}${help ? `<small class="field-help">${help}</small>` : ''}</label>`;
const check = (name: string, on: boolean, label: string) => `<label class="check"><input name="${name}" type="checkbox" ${on ? 'checked' : ''}> ${label}</label>`;

/** Device and helper facts for Advanced; absent when the helper does not answer. */
async function facts(call: RpcClient): Promise<{ account: any; snapshot: any }> {
  const [account, snapshot] = await Promise.allSettled([call<any>('account.status', {}), call<any>('status.snapshot', {})]);
  return { account: account.status === 'fulfilled' ? account.value : {}, snapshot: snapshot.status === 'fulfilled' ? snapshot.value : {} };
}

export async function mountSettings(root: HTMLElement, call: RpcClient, _route: DashboardRoute): Promise<void> {
  const stored: any = await call('settings.get', {});
  const { account, snapshot } = await facts(call);
  const render = (current: any, message = '', open = false, failed = false): void => {
    // The save message shows beside the Save button the reader can see.
    const saveRow = (attr: string, here: boolean) => `<div class="actions"><button type="submit" class="primary">Save</button><p ${attr} class="${failed ? 'error' : 'muted'}" role="status">${here ? esc(message) : ''}</p></div>`;
    root.innerHTML = `<section class="narrow"><h1>Settings</h1>
<form data-settings>
<div class="group"><div><h2>Saving pages</h2><p class="field-help group-help">RZN reads the pages you open on these sites, exactly as you see them, and saves them to your library. Nothing from any other site leaves your browser.</p></div>
${field('Sites', `<textarea name="capture_sites" rows="4" placeholder="example.com">${esc((current.capture_sites || []).join('\n'))}</textarea>`, 'One site per line, such as example.com. Its subdomains are included. You can also switch a site on or off from the RZN button in the toolbar.')}
${field('Save to index', `<input name="capture_index" placeholder="my-index" value="${esc(current.capture_index || '')}">`, 'The RZN index these pages go to.')}
</div>
${saveRow('data-save-message', !open)}
<details class="advanced" data-advanced ${open ? 'open' : ''}><summary>Advanced</summary>
<div class="group"><h2>This browser</h2><dl class="facts"><dt>Device ID</dt><dd><code>${esc(account.device_id || '—')}</code></dd><dt>Server</dt><dd><code>${esc(account.server_url || '—')}</code></dd><dt>Last check-in</dt><dd>${relativeTime(account.fleet?.last_poll_ms)}</dd><dt>Helper version</dt><dd><code>${esc(snapshot.supervisor_version || '—')}</code></dd></dl></div>
<div class="group"><h2>Server and source</h2>
${field('RZN server', `<input name="server_url" placeholder="https://app.rzn.ai" value="${esc(current.server_url || '')}">`, 'Empty uses https://app.rzn.ai. After changing it, sign in again on the Account page.')}
${field('Source', `<input name="capture_source" placeholder="capture" value="${esc(current.capture_source || '')}">`, 'The pushed source inside the index. Every page read and batch sent is listed in <code>capture/activity.jsonl</code>.')}
</div>
<div class="group"><h2>Remote tasks</h2>
<label class="check"><input data-pause type="checkbox" ${snapshot.paused ? 'checked' : ''}> Pause tasks in this browser</label>
${check('fleet_keep_window_on_failure', current.fleet_keep_window_on_failure, 'Keep the window open when a remote task fails')}
${check('notifications_enabled', current.notifications_enabled, 'Show notifications for remote tasks')}
<fieldset><legend>Notify on</legend><label class="check"><input type="radio" name="notify_on" value="all" ${current.notify_on === 'all' ? 'checked' : ''}> All runs</label><label class="check"><input type="radio" name="notify_on" value="failures_only" ${current.notify_on === 'failures_only' ? 'checked' : ''}> Failures only</label></fieldset>
</div>
<div class="group"><h2>History</h2><div class="pair">
${field('Runs to retain', `<input name="run_retention_count" type="number" min="10" max="10000" value="${esc(current.run_retention_count)}">`)}
${field('Retention days', `<input name="run_retention_days" type="number" min="1" max="365" value="${esc(current.run_retention_days)}">`)}
</div><p class="field-help">Poll interval: <code>${esc(current.poll_interval_seconds || 'environment-controlled')}</code> · Config path: <code>${esc(current.config_path || 'environment-controlled')}</code></p></div>
<div class="group"><h2>Tools</h2><p class="tools"><a href="#runs">Runs</a><a href="#workflows">Workflows</a><a href="#logs">Logs</a></p></div>
${saveRow('data-save-message-advanced', open)}
</details>
</form></section>`;
    const form = root.querySelector<HTMLFormElement>('[data-settings]')!;
    root.querySelector<HTMLInputElement>('[data-pause]')?.addEventListener('change', async event => {
      const input = event.target as HTMLInputElement;
      try { await call(input.checked ? 'automation.pause' : 'automation.resume', { cancel_current: false }); }
      catch { input.checked = !input.checked; }
    });
    form.addEventListener('submit', async event => {
      event.preventDefault();
      const advancedOpen = Boolean(root.querySelector<HTMLDetailsElement>('[data-advanced]')?.open);
      form.querySelectorAll<HTMLButtonElement>('button[type="submit"]').forEach(button => { button.disabled = true; });
      const data = new FormData(form);
      const patch = { run_retention_count: Number(data.get('run_retention_count')), run_retention_days: Number(data.get('run_retention_days')), notifications_enabled: data.get('notifications_enabled') === 'on', notify_on: data.get('notify_on'), fleet_keep_window_on_failure: data.get('fleet_keep_window_on_failure') === 'on', capture_sites: String(data.get('capture_sites') || '').split('\n').map(site => site.trim()).filter(Boolean), server_url: String(data.get('server_url') || ''), capture_index: String(data.get('capture_index') || ''), capture_source: String(data.get('capture_source') || '') };
      try { const saved: any = await call('settings.set', { patch }); render(saved, 'Saved.', advancedOpen); }
      catch (cause) { render(current, `Couldn't save. ${friendly(cause, 'RZN settings').line}`, advancedOpen, true); }
    });
  };
  render(stored);
}
