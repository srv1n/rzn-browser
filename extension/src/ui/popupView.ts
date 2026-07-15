import { rpc } from './rpc';

type Run = { run_id:string; workflow_id:string; origin:string; status:string; started_at:number; ended_at:number };
export type Snapshot = {
  supervisor_version:string; native_host_connected:boolean; extension_connected:boolean; paused:boolean;
  now_running:(Record<string, unknown> & { run_id?:string; workflow_id?:string; origin?:string; step_index?:number; step_total?:number; started_at?:number })|null;
  fleet:(Record<string, unknown> & { device_name?:string; device_id?:string; tenant_id?:string; status?:string; state?:string; last_poll_ms?:number })|null;
  recent_runs:Run[]; flagged_workflows:number;
};

const esc=(value:unknown)=>String(value??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!));
function age(timestamp:number|undefined,now:number){if(!timestamp)return'never';const seconds=Math.max(0,Math.floor((now-timestamp)/1000));if(seconds<60)return`${seconds}s ago`;const minutes=Math.floor(seconds/60);return minutes<60?`${minutes}m ago`:`${Math.floor(minutes/60)}h ago`;}
function elapsed(timestamp:number|undefined,now:number){if(!timestamp)return'just started';const seconds=Math.max(0,Math.floor((now-timestamp)/1000));const minutes=Math.floor(seconds/60);return minutes?`${minutes}m ${seconds%60}s`:`${seconds}s`;}
function originLabel(origin:string|undefined){if(origin?.startsWith('fleet'))return'fleet';if(origin==='schedule'||origin?.startsWith('schedule:'))return'schedule';return'local';}
function statusClass(status:string){if(status==='succeeded')return'ok';if(status==='failed')return'bad';if(status==='running')return'running';if(status==='cancelled'||status==='canceled')return'cancelled';return'muted';}

export function popupHtml(snapshot:Snapshot,now=Date.now()):string {
  const running=snapshot.now_running;const recent=snapshot.recent_runs.slice(0,5);const cloud=snapshot.fleet?`<span class="ok">● Cloud <small>${esc(age(snapshot.fleet.last_poll_ms,now))}</small></span>`:'<span class="muted">● Cloud</span>';
  return `<header><b>RZN Automation</b><a href="dashboard.html#runs" target="_blank">Dashboard</a></header>
<section class="health"><span class="${snapshot.extension_connected?'ok':'bad'}">● Extension</span><span class="${snapshot.native_host_connected?'ok':'bad'}">● Native</span><span class="ok">● Supervisor</span>${cloud}</section>
${running?`<section class="now-running"><small>NOW RUNNING <span class="origin origin-${originLabel(running.origin)}">${originLabel(running.origin)}</span></small><h2>${esc(running.workflow_id)}</h2><p>Step ${running.step_index??0} of ${running.step_total??0} · ${elapsed(running.started_at,now)}</p><button id="stop" class="danger">Stop</button></section>`:`<section><h2>${snapshot.paused?'Automation paused':'Ready'}</h2><p>${snapshot.paused?'No new workflows will start.':'Waiting for work.'}</p></section>`}
<label class="toggle"><input id="pause" type="checkbox" ${snapshot.paused?'checked':''}> Pause automation</label>
<section><h3>Recent runs</h3>${recent.length?recent.map(run=>`<a class="run" href="dashboard.html#runs/${encodeURIComponent(run.run_id)}" target="_blank"><span class="${statusClass(run.status)}">●</span>${esc(run.workflow_id)}<span class="origin origin-${originLabel(run.origin)}">${originLabel(run.origin)}</span><small>${esc(run.status)}</small></a>`).join(''):'<p class="muted">No runs yet</p>'}</section>
<footer>${snapshot.fleet?`${esc(snapshot.fleet.device_name||snapshot.fleet.device_id)} · ${esc(snapshot.fleet.tenant_id||snapshot.fleet.status||snapshot.fleet.state||'enrolled')}`:'<a href="dashboard.html#fleet" target="_blank">Connect to a server</a>'}</footer>`;
}

export const unreachableHtml = '<section class="offline"><h2>Supervisor unreachable</h2><p>Check the native host, then retry.</p><button id="retry">Retry</button></section>';

export function bindPopupActions(root:ParentNode,snapshot:Snapshot,refresh:()=>void|Promise<void>,call=rpc):void {
  root.querySelector('#stop')?.addEventListener('click',()=>{void call('runs.cancel',{run_id:snapshot.now_running?.run_id}).then(refresh);});
  root.querySelector('#pause')?.addEventListener('change',async event=>{const paused=(event.target as HTMLInputElement).checked;let cancel_current=false;if(paused&&snapshot.now_running)cancel_current=confirm('Cancel the current run too?');await call(paused?'automation.pause':'automation.resume',{cancel_current});await refresh();});
}
