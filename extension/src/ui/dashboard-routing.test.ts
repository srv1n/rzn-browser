import{describe,expect,it,vi}from'vitest';
vi.mock('./rpc',()=>({rpc:vi.fn(),SupervisorUnreachable:class extends Error{}}));
describe('dashboard routing',()=>{it('preserves a run deep-link and logs query parameters',async()=>{
  vi.stubGlobal('document',{querySelector:()=>null});vi.stubGlobal('location',{hash:'#runs/run%2F42'});
  const {route}=await import('../dashboard/index');
  expect(route()).toMatchObject({tab:'runs',id:'run%2F42'});
  expect(route('#logs?run=run%2F42').query.get('run')).toBe('run/42');
});it('renders application errors without claiming the supervisor is unreachable',async()=>{const{applicationError}=await import('../dashboard/shared');const root={innerHTML:''}as HTMLElement;applicationError(root,new Error('automation is paused'));expect(root.innerHTML).toContain('Request failed');expect(root.innerHTML).toContain('automation is paused');expect(root.innerHTML).not.toContain('Supervisor unreachable');});});
