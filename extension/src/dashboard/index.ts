import './style.css';
import { rpc, SupervisorUnreachable } from '../ui/rpc';
import { mountFleet } from './fleet';
import { mountLogs } from './logs';
import { mountRuns } from './runs';
import { applicationError, DashboardRoute, esc, tabs, unavailable } from './shared';
import { mountSettings } from './settings';
import { mountWorkflows } from './workflows';

const app = document.querySelector<HTMLElement>('#app');
let dispose: (() => void) | undefined;

export function route(hash = location.hash): DashboardRoute {
  const [path, queryString = ''] = hash.slice(1).split('?');
  const [tab = 'runs', id] = path.split('/');
  return { tab: tabs.includes(tab) ? tab : 'runs', id, query: new URLSearchParams(queryString) };
}

export async function render(): Promise<void> {
  if (!app) return;
  dispose?.(); dispose = undefined;
  const current = route();
  app.innerHTML = `<aside><h2>RZN</h2>${tabs.map(tab => `<a class="${tab === current.tab ? 'active' : ''}" href="#${tab}">${tab}</a>`).join('')}</aside><main><p>Loading…</p></main>`;
  const main = app.querySelector<HTMLElement>('main')!;
  const navigate = (hash: string) => { location.hash = hash; };
  try {
    if (current.tab === 'runs') await mountRuns(main, rpc, current, { navigate });
    else if (current.tab === 'workflows') await mountWorkflows(main, rpc, current, { navigate });
    else if (current.tab === 'fleet') dispose = await mountFleet(main, rpc, current, { navigate });
    else if (current.tab === 'logs') dispose = await mountLogs(main, rpc, current);
    else await mountSettings(main, rpc, current);
  } catch (error) {
    if (error instanceof SupervisorUnreachable) unavailable(main, error);
    else applicationError(main, error);
  }
}

export function startDashboard(): void {
  window.addEventListener('hashchange', () => { void render(); });
  void render();
}

if (app) startDashboard();
