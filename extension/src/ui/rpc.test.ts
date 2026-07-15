import{beforeEach,describe,expect,it,vi}from'vitest';import{DEFAULT_RPC_TIMEOUT_MS,rpc,SupervisorApplicationError,SupervisorUnreachable}from'./rpc';
const row={run_id:'r',workflow_id:'wf',origin:'local_cli',started_at:1,ended_at:2,status:'succeeded'};
beforeEach(()=>{(globalThis as any).chrome={runtime:{sendMessage:vi.fn()}}});
describe('rpc',()=>{
  it('validates and unwraps',(async()=>{(chrome.runtime.sendMessage as any).mockResolvedValue({success:true,result:{ok:true,total:1,runs:[row]}});expect((await rpc<any>('runs.list')).total).toBe(1)}));
  it('validates every dashboard response',async()=>{for(const[method,result]of Object.entries({
    'runs.get':{ok:true,record:row,result:{}},'workflows.list':{workflows:[]},'fleet.status':{state:'disabled'},
    'settings.get':{run_retention_count:500,run_retention_days:30,notifications_enabled:true,notify_on:'all',fleet_keep_window_on_failure:false},
    'settings.set':{run_retention_count:500,run_retention_days:30,notifications_enabled:false,notify_on:'all',fleet_keep_window_on_failure:false},
    'diagnostics.export':{path:'/tmp/x.zip'},
  })){(chrome.runtime.sendMessage as any).mockResolvedValueOnce({success:true,result});await expect(rpc(method)).resolves.toEqual(result)}});
  it('maps transport errors',async()=>{(chrome.runtime.sendMessage as any).mockRejectedValue(new Error('down'));await expect(rpc('runs.list')).rejects.toBeInstanceOf(SupervisorUnreachable)});
  it('maps a missing transport response as unreachable',async()=>{(chrome.runtime.sendMessage as any).mockResolvedValue(undefined);await expect(rpc('runs.list')).rejects.toBeInstanceOf(SupervisorUnreachable)});
  it('preserves application errors',async()=>{(chrome.runtime.sendMessage as any).mockResolvedValue({success:false,error:'automation is paused'});await expect(rpc('runs.start')).rejects.toEqual(expect.objectContaining({name:'SupervisorApplicationError',message:'automation is paused'}));await expect(rpc('runs.start')).rejects.toBeInstanceOf(SupervisorApplicationError)});
  it('validates nullable run endings and control responses',async()=>{const inFlight={...row,ended_at:null};(chrome.runtime.sendMessage as any).mockResolvedValueOnce({success:true,result:{ok:true,total:1,runs:[inFlight]}}).mockResolvedValueOnce({success:true,result:{ok:true,cancel_requested:true}}).mockResolvedValueOnce({success:true,result:{ok:true,paused:true,cancel_current:false}}).mockResolvedValueOnce({success:true,result:{ok:true,paused:false}});await expect(rpc('runs.list')).resolves.toMatchObject({runs:[{ended_at:null}]});await expect(rpc('runs.cancel')).resolves.toMatchObject({cancel_requested:true});await expect(rpc('automation.pause')).resolves.toMatchObject({paused:true});await expect(rpc('automation.resume')).resolves.toMatchObject({paused:false})});
  it('waits longer than the native host control deadline',()=>expect(DEFAULT_RPC_TIMEOUT_MS).toBeGreaterThan(10_000));
});
