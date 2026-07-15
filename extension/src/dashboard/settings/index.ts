import { DashboardRoute, esc, RpcClient } from '../shared';

export async function mountSettings(root: HTMLElement, call: RpcClient, _route: DashboardRoute): Promise<void> {
  const stored: any = await call('settings.get', {});
  const render = (current: any, message = ''): void => {
    root.innerHTML = `<section><h1>Settings</h1><form data-settings><label>Retain runs <input name="run_retention_count" type="number" min="10" max="10000" value="${esc(current.run_retention_count)}"></label><label>Retention days <input name="run_retention_days" type="number" min="1" max="365" value="${esc(current.run_retention_days)}"></label><label><input name="notifications_enabled" type="checkbox" ${current.notifications_enabled ? 'checked' : ''}> Notifications</label><fieldset><legend>Notify on</legend><label><input type="radio" name="notify_on" value="all" ${current.notify_on === 'all' ? 'checked' : ''}> All runs</label><label><input type="radio" name="notify_on" value="failures_only" ${current.notify_on === 'failures_only' ? 'checked' : ''}> Failures only</label></fieldset><label><input name="fleet_keep_window_on_failure" type="checkbox" ${current.fleet_keep_window_on_failure ? 'checked' : ''}> Keep failed fleet window</label><p>Poll interval: <code>${esc(current.poll_interval_seconds || 'environment-controlled')}</code><br>Config path: <code>${esc(current.config_path || 'environment-controlled')}</code></p><p data-save-message class="error">${esc(message)}</p><button>Save</button></form></section>`;
    root.querySelector<HTMLFormElement>('[data-settings]')!.addEventListener('submit', async event => {
      event.preventDefault(); const form = event.currentTarget as HTMLFormElement; const button = form.querySelector('button')!; button.disabled = true; const message = form.querySelector<HTMLElement>('[data-save-message]')!; message.className = 'muted'; message.textContent = 'Saving…';
      const data = new FormData(form); const patch = { run_retention_count: Number(data.get('run_retention_count')), run_retention_days: Number(data.get('run_retention_days')), notifications_enabled: data.get('notifications_enabled') === 'on', notify_on: data.get('notify_on'), fleet_keep_window_on_failure: data.get('fleet_keep_window_on_failure') === 'on' };
      try { const saved: any = await call('settings.set', { patch }); render(saved, 'Saved.'); }
      catch (cause) { render(current, cause instanceof Error ? `Save failed; restored previous values: ${cause.message}` : 'Save failed; restored previous values.'); }
    });
  };
  render(stored);
}
