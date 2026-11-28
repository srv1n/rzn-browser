import { avatar } from '../../ui/brand';
import { friendly } from '../../ui/errors';
import { DashboardRoute, esc, relativeTime, RpcClient } from '../shared';

const field = (label: string, control: string, help = '') => `<label class="field"><span class="field-label">${label}</span>${control}${help ? `<small class="field-help">${help}</small>` : ''}</label>`;
const check = (name: string, on: boolean, label: string) => `<label class="check"><input name="${name}" type="checkbox" ${on ? 'checked' : ''}> ${label}</label>`;
const card = (title: string, help: string, body: string, extra = '') => `<div class="card ${extra}"><div class="card-head"><h2>${title}</h2>${help ? `<p>${help}</p>` : ''}</div>${body}</div>`;

/** A typed site as the bare host the capture list holds: no scheme, path or port. */
export function siteEntry(typed: string): string {
  return typed.trim().toLowerCase().replace(/^[a-z]+:\/\//, '').replace(/[/?#:].*$/, '').replace(/^\.+|\.+$/g, '');
}

/** Device and helper facts for Advanced; absent when the helper does not answer. */
async function facts(call: RpcClient): Promise<{ account: any; snapshot: any }> {
  const [account, snapshot] = await Promise.allSettled([call<any>('account.status', {}), call<any>('status.snapshot', {})]);
  return { account: account.status === 'fulfilled' ? account.value : {}, snapshot: snapshot.status === 'fulfilled' ? snapshot.value : {} };
}

type Site = { site: string; on: boolean };

const siteRow = ({ site, on }: Site, i: number) => `<li class="site-row">${avatar(site, 'site')}<label class="site-label" for="site-${i}">${esc(site)}</label><input id="site-${i}" class="switch" type="checkbox" role="switch" data-site="${esc(site)}" ${on ? 'checked' : ''} aria-label="Save pages from ${esc(site)}"></li>`;
const siteRows = (sites: Site[]) => sites.length ? sites.map(siteRow).join('') : '<li class="site-empty">No sites yet. Add one below, or switch a site on from the RZN button in the toolbar.</li>';

export async function mountSettings(root: HTMLElement, call: RpcClient, _route: DashboardRoute): Promise<void> {
  const stored: any = await call('settings.get', {});
  const { account, snapshot } = await facts(call);
  const render = (current: any, message = '', open = false, failed = false): void => {
    // Switched-off rows stay listed until the page reloads, so a slip can be undone.
    const sites: Site[] = (current.capture_sites || []).map((site: string) => ({ site, on: true }));
    const onSites = () => sites.filter(s => s.on).map(s => s.site).join('\n');
    // The save message shows beside the Save button the reader can see.
    const footer = (attr: string, here: boolean) => `<div class="save-bar"><p ${attr} class="save-note ${failed ? 'error' : here && message ? 'saved' : ''}" role="status">${here ? esc(message) : ''}</p><button type="submit" class="primary">Save changes</button></div>`;
    root.innerHTML = `<section class="narrow"><header class="page-head"><h1>Settings</h1><p class="lede">Choose which sites RZN saves and where the pages go.</p></header>
<form data-settings>
${card('Sites', 'RZN reads the pages you open on these sites, exactly as you see them. Nothing from any other site leaves your browser.', `
<input type="hidden" name="capture_sites" value="${esc(onSites())}">
<ul class="site-list" data-site-list>${siteRows(sites)}</ul>
<div class="add-site"><input data-add-site type="text" placeholder="example.com" aria-label="Add a site" autocomplete="off" spellcheck="false"><button type="button" data-add>Add site</button></div>
<small class="field-help">Subdomains are included: adding example.com also covers news.example.com.</small>`)}
${card('Save to index', 'Pages from the sites above go to this RZN index.', field('Index name', `<input name="capture_index" placeholder="my-index" value="${esc(current.capture_index || '')}" spellcheck="false">`, 'Use the name of an index you can write to, such as <b>reading</b>.'))}
${footer('data-save-message', !open)}
<details class="advanced" data-advanced ${open ? 'open' : ''}><summary><span class="summary-text"><b>Advanced</b><span>Server, this browser's details, history and tools</span></span></summary>
<div class="advanced-body">
${card('This browser', '', `<dl class="facts"><dt>Device ID</dt><dd><code>${esc(account.device_id || '—')}</code></dd><dt>Server</dt><dd><code>${esc(account.server_url || '—')}</code></dd><dt>Last check-in</dt><dd>${relativeTime(account.fleet?.last_poll_ms)}</dd><dt>Helper version</dt><dd><code>${esc(snapshot.supervisor_version || '—')}</code></dd></dl>`)}
${card('Server and source', '', `<div class="stack">
${field('RZN server', `<input name="server_url" placeholder="https://app.rzn.ai" value="${esc(current.server_url || '')}">`, 'Empty uses https://app.rzn.ai. After changing it, sign in again on the Account page.')}
${field('Source', `<input name="capture_source" placeholder="main" value="${esc(current.capture_source || 'main')}">`, 'The pushed source inside the index; <b>main</b> is the index&#39;s first source. Every page read and batch sent is listed in <code>capture/activity.jsonl</code>.')}
</div>`)}
${card('Remote tasks', '', `<div class="stack tight">
<label class="check"><input data-pause type="checkbox" ${snapshot.paused ? 'checked' : ''}> Pause tasks in this browser</label>
${check('fleet_keep_window_on_failure', current.fleet_keep_window_on_failure, 'Keep the window open when a remote task fails')}
${check('notifications_enabled', current.notifications_enabled, 'Show notifications for remote tasks')}
<fieldset><legend>Notify on</legend><label class="check"><input type="radio" name="notify_on" value="all" ${current.notify_on === 'all' ? 'checked' : ''}> All runs</label><label class="check"><input type="radio" name="notify_on" value="failures_only" ${current.notify_on === 'failures_only' ? 'checked' : ''}> Failures only</label></fieldset>
</div>`)}
${card('History', '', `<div class="pair">
${field('Runs to retain', `<input name="run_retention_count" type="number" min="10" max="10000" value="${esc(current.run_retention_count)}">`)}
${field('Retention days', `<input name="run_retention_days" type="number" min="1" max="365" value="${esc(current.run_retention_days)}">`)}
</div><p class="field-help facts-note">Poll interval: <code>${esc(current.poll_interval_seconds || 'environment-controlled')}</code> · Config path: <code>${esc(current.config_path || 'environment-controlled')}</code></p>`)}
${card('Tools', '', `<p class="tools"><a href="#runs">Runs</a><a href="#workflows">Workflows</a><a href="#logs">Logs</a></p>`)}
${footer('data-save-message-advanced', open)}
</div>
</details>
</form></section>`;
    const form = root.querySelector<HTMLFormElement>('[data-settings]')!;
    const list = root.querySelector<HTMLElement>('[data-site-list]');
    const hidden = root.querySelector<HTMLInputElement>('input[name="capture_sites"]');
    const sync = () => { if (hidden) hidden.value = onSites(); };
    const bindSwitches = () => list?.querySelectorAll<HTMLInputElement>('[data-site]').forEach(input => input.addEventListener('change', () => {
      const entry = sites.find(s => s.site === input.dataset.site); if (entry) entry.on = input.checked; sync();
    }));
    bindSwitches();
    const addInput = root.querySelector<HTMLInputElement>('[data-add-site]');
    const add = () => {
      if (!addInput || !list) return;
      const site = siteEntry(addInput.value); addInput.value = '';
      if (!site) return;
      const existing = sites.find(s => s.site === site);
      if (existing) existing.on = true; else sites.push({ site, on: true });
      list.innerHTML = siteRows(sites); bindSwitches(); sync(); addInput.focus();
    };
    root.querySelector('[data-add]')?.addEventListener('click', add);
    // Enter in the add box adds the site instead of submitting the whole form.
    addInput?.addEventListener('keydown', event => { if ((event as KeyboardEvent).key === 'Enter') { event.preventDefault(); add(); } });
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
      try { const saved: any = await call('settings.set', { patch }); render(saved, 'Saved', advancedOpen); }
      catch (cause) { render(current, `Couldn't save. ${friendly(cause, 'RZN settings').line}`, advancedOpen, true); }
    });
  };
  render(stored);
}
