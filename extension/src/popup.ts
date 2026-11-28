import './popup.css';
import { siteOf } from './capture';
import { rpc } from './ui/rpc';
import { bindPopupActions, popupHtml, problemHtml, type Account, type PopupState, type Settings, type Snapshot } from './ui/popupView';

const app=document.querySelector<HTMLElement>('#app')!;
let shown='';
let refreshTimer: ReturnType<typeof setTimeout> | undefined;

async function activeSite():Promise<{site:string|null;icon:string|null}>{const [tab]=await chrome.tabs.query({active:true,currentWindow:true});return {site:tab?.url?siteOf(tab.url):null,icon:tab?.favIconUrl||null};}

// Re-render only when something changed, so the 2 s refresh never steals keyboard focus.
function show(html:string,bind:()=>void){if(html===shown)return;const focused=(document.activeElement as HTMLElement|null)?.id;shown=html;app.innerHTML=html;bind();if(focused)app.querySelector<HTMLElement>(`#${focused}`)?.focus();}

export async function refresh(){
  if(refreshTimer)clearTimeout(refreshTimer);
  try{
    const [snapshot,account,settings,{site,icon}]=await Promise.all([rpc<Snapshot>('status.snapshot'),rpc<Account>('account.status'),rpc<Settings>('settings.get'),activeSite()]);
    const state:PopupState={snapshot,account,settings,site,icon};
    show(popupHtml(state),()=>bindPopupActions(app,state,refresh));
  }catch(error){
    const retry=()=>app.querySelector('#retry')?.addEventListener('click',async()=>{await chrome.runtime.sendMessage({type:'RZN_WAKE_NATIVE',resetTerminal:true});await refresh();});
    show(problemHtml(error),retry);
  }finally{refreshTimer=setTimeout(refresh,2000);}
}
app.innerHTML='<p class="skeleton muted">Connecting…</p>';
void refresh();
