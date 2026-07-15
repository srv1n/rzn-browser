import{describe,expect,it}from'vitest';import{badgeForState}from'./badge';
describe('badgeForState',()=>{it('covers unreachable, paused, failed, and running precedence',()=>{
  expect(badgeForState({reachable:false}).text).toBe('•');
  expect(badgeForState({reachable:true,paused:true,runningCount:1}).text).toBe('⏸');
  expect(badgeForState({reachable:true,lastRunFailed:true,runningCount:1}).text).toBe('!');
  expect(badgeForState({reachable:true,flaggedWorkflows:1}).text).toBe('!');
  expect(badgeForState({reachable:true,runningCount:2}).text).toBe('2');
});});
