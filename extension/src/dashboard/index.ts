import './style.css';
import { rpc, SupervisorUnreachable } from '../ui/rpc';
import { mountAccount } from './account';
import { mountLogs } from './logs';
import { mountRuns } from './runs';
import { advancedTabs, applicationError, DashboardRoute, mainTabs, tabLabels, tabs, unavailable } from './shared';
import { mountSettings } from './settings';
import { mountWorkflows } from './workflows';

const app = document.querySelector<HTMLElement>('#app');
let dispose: (() => void) | undefined;
let renderGeneration = 0;

export function route(hash = location.hash): DashboardRoute {
  const [path, queryString = ''] = hash.slice(1).split('?');
  const [tab = 'account', id] = path.split('/');
  return { tab: tabs.includes(tab) ? tab : 'account', id, query: new URLSearchParams(queryString) };
}

export async function render(): Promise<void> {
  if (!app) return;
  dispose?.(); dispose = undefined;
  const generation = ++renderGeneration;
  // A hash change during a slow mount starts a newer render; dispose the stale mount as soon
  // as it resolves instead of letting its timers leak.
  const adopt = (d: (() => void) | undefined) => {
    if (generation === renderGeneration) dispose = d;
    else d?.();
  };
  const current = route();
  const link = (tab: string) => `<a class="${tab === current.tab ? 'active' : ''}" href="#${tab}" ${tab === current.tab ? 'aria-current="page"' : ''}>${tabLabels[tab]}</a>`;
  const advanced = advancedTabs.includes(current.tab) ? `<span class="label nav-label">Advanced</span>${advancedTabs.map(link).join('')}` : '';
  app.innerHTML = `<aside class="sidebar"><div class="brand">RZN</div><nav aria-label="RZN">${mainTabs.map(link).join('')}${advanced}</nav></aside><main><p class="muted">Loading…</p></main>`;
  const main = app.querySelector<HTMLElement>('main')!;
  const navigate = (hash: string) => { location.hash = hash; };
  try {
    if (current.tab === 'runs') await mountRuns(main, rpc, current, { navigate });
    else if (current.tab === 'workflows') await mountWorkflows(main, rpc, current, { navigate });
    else if (current.tab === 'account') adopt(await mountAccount(main, rpc, current));
    else if (current.tab === 'logs') adopt(await mountLogs(main, rpc, current));
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
