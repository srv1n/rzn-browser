import './popup.css';
import { rpc, SupervisorUnreachable } from './ui/rpc';
import { bindPopupActions, popupHtml, unreachableHtml, type Snapshot } from './ui/popupView';

const app=document.querySelector<HTMLElement>('#app')!;
function render(snapshot:Snapshot){app.innerHTML=popupHtml(snapshot);bindPopupActions(app,snapshot,refresh);}
export async function refresh(){try{render(await rpc<Snapshot>('status.snapshot'));}catch(error){if(error instanceof SupervisorUnreachable){app.innerHTML=unreachableHtml;app.querySelector('#retry')?.addEventListener('click',refresh);}}}
app.innerHTML='<div class="skeleton">Connecting…</div>';
void refresh();
setInterval(refresh,2000);
