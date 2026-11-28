import { switchedOn } from '../capture';
import { rpc } from './rpc';

export type Snapshot = {
  supervisor_version:string; native_host_connected:boolean; extension_connected:boolean; paused:boolean;
  now_running:(Record<string, unknown> & { run_id?:string; workflow_id?:string })|null;
  fleet:(Record<string, unknown> & { login_walls?:string[] })|null;
  recent_runs:unknown[]; flagged_workflows:number;
};
export type Account = { signed_in:boolean; user_email?:string|null; user_id?:string|null; server_url?:string|null; fleet?:{ state?:string }|null };
export type Settings = { capture_sites?:string[]; capture_index?:string; server_url?:string };
/** Everything the popup shows: the helper's snapshot, the account, settings and the active tab's site. */
export type PopupState = { snapshot:Snapshot; account:Account; settings:Settings; site:string|null };

const esc=(value:unknown)=>String(value??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!));
const DEFAULT_SERVER='https://app.rzn.ai';

const header=`<header class="top"><span class="wordmark">RZN</span><a class="top-link label" href="dashboard.html#settings" target="_blank">Settings</a></header>`;

export const signedOutHtml=`${header}<main class="hero"><span class="label">Welcome</span><h1>Keep what you <em>read</em>.</h1><p>Sign in to save the pages you choose to RZN and search them any time.</p><a class="button primary wide" id="sign-in" href="dashboard.html#account" target="_blank">Sign in to RZN</a></main>`;

export const unreachableHtml=`${header}<main class="hero"><span class="label">Not connected</span><h1>RZN isn't <em>answering</em></h1><p>The RZN helper on this computer didn't respond. Make sure RZN is installed and running, then try again.</p><button id="retry" class="primary wide">Try again</button></main>`;

export const errorHtml=(error:unknown)=>`${header}<main class="hero"><span class="label">Something went wrong</span><h1>That didn't <em>work</em></h1><p>${esc(error instanceof Error?error.message:error)}</p><button id="retry" class="primary wide">Try again</button></main>`;

/** Whether pages of the tab's site are saved, and the list entry that covers it. */
function coveringEntry(sites:readonly string[],site:string):string|undefined{return sites.find(entry=>switchedOn([entry],site));}

function siteCard(state:PopupState):string{
  const {site,settings}=state;
  if(!site)return `<section class="card site"><span class="label">This page</span><p class="quiet">Pages like this one can't be saved.</p></section>`;
  const sites=settings.capture_sites??[];const entry=coveringEntry(sites,site);const on=Boolean(entry);
  const where=settings.capture_index?`Pages you open here are saved to <b>${esc(settings.capture_index)}</b>.`:'Choose where pages go in <a href="dashboard.html#settings" target="_blank">Settings</a>.';
  const via=entry&&entry!==site?` <span class="muted">(via ${esc(entry)})</span>`:'';
  return `<section class="card site"><span class="label">This site</span><div class="row"><label class="site-name" for="capture-switch">${esc(site)}${via}</label><input id="capture-switch" class="switch" type="checkbox" role="switch" ${on?'checked':''} aria-describedby="capture-help"></div><p id="capture-help" class="quiet">${on?where:'Off. Nothing from this site leaves your browser.'}</p></section>`;
}

function notices(snapshot:Snapshot):string{
  const walls=(snapshot.fleet?.login_walls??[]).map(site=>`<section class="notice"><p><b>Sign in to ${esc(site)}</b> so RZN can carry on there.</p><button class="quiet" data-cleared="${esc(site)}">I've signed in</button></section>`).join('');
  const running=snapshot.now_running?`<section class="notice"><p>RZN is working in a background window.</p><button id="stop" class="quiet">Stop</button></section>`:'';
  return walls+running;
}

export function popupHtml(state:PopupState):string{
  const {account}=state;
  if(!account.signed_in)return signedOutHtml;
  const who=account.user_email||account.user_id||'your account';
  const stopped=String(account.fleet?.state||'').startsWith('stopped');
  const status=stopped?'<span class="chip bad">Signed out from the app</span>':'<span class="chip ok">Connected</span>';
  const server=(account.server_url||state.settings.server_url||DEFAULT_SERVER).replace(/\/+$/,'');
  return `${header}<main><section class="account"><span class="label">Signed in as</span><div class="row"><span class="who" title="${esc(who)}">${esc(who)}</span>${status}</div></section>${siteCard(state)}${notices(state.snapshot)}<a class="button primary wide" id="open-rzn" href="${esc(server)}" target="_blank" rel="noopener">Open RZN</a></main>`;
}

export function bindPopupActions(root:ParentNode,state:PopupState,refresh:()=>void|Promise<void>,call=rpc):void{
  root.querySelectorAll?.('[data-cleared]').forEach(button=>button.addEventListener('click',()=>{void call('fleet.site_cleared',{system:(button as HTMLElement).dataset.cleared}).then(refresh);}));
  root.querySelector('#stop')?.addEventListener('click',()=>{void call('runs.cancel',{run_id:state.snapshot.now_running?.run_id}).then(refresh);});
  root.querySelector('#capture-switch')?.addEventListener('change',async event=>{
    const input=event.target as HTMLInputElement;const site=state.site;if(!site)return;
    // Not disabled while saving: disabling would drop keyboard focus. aria-busy marks the wait.
    if(input.getAttribute?.('aria-busy')==='true')return;input.setAttribute?.('aria-busy','true');
    try{
      // Read the list fresh: this view may be older than the stored settings.
      const sites=(await call<Settings>('settings.get',{})).capture_sites??[];
      // Off removes whichever entry covers this site (it may be a parent domain); on adds the site.
      const next=input.checked?[...sites,site]:sites.filter(entry=>!switchedOn([entry],site));
      await call('settings.set',{patch:{capture_sites:next}});
    }catch(error){
      input.checked=!input.checked;input.removeAttribute?.('aria-busy');
      const help=root.querySelector('#capture-help');if(help)help.textContent=`Couldn't change this: ${error instanceof Error?error.message:String(error)}`;
      return;
    }
    await refresh();
  });
}
