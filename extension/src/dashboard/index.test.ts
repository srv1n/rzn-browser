import { afterEach, describe, expect, it, vi } from 'vitest';

const mounts = vi.hoisted(() => ({ mountFleet: vi.fn(), mountLogs: vi.fn() }));
vi.mock('./style.css', () => ({}));
vi.mock('../ui/rpc', () => ({ rpc: vi.fn(), SupervisorUnreachable: class extends Error {} }));
vi.mock('./fleet', () => ({ mountFleet: mounts.mountFleet }));
vi.mock('./logs', () => ({ mountLogs: mounts.mountLogs }));
vi.mock('./runs', () => ({ mountRuns: vi.fn() }));
vi.mock('./settings', () => ({ mountSettings: vi.fn() }));
vi.mock('./workflows', () => ({ mountWorkflows: vi.fn() }));

describe('dashboard render', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('disposes a mount that resolves after a newer render started', async () => {
    const app = { innerHTML: '', querySelector: () => ({}) };
    const location = { hash: '#fleet' };
    vi.stubGlobal('document', { querySelector: () => app });
    vi.stubGlobal('window', { addEventListener: vi.fn() });
    vi.stubGlobal('location', location);

    let resolveFleet!: (dispose: () => void) => void;
    mounts.mountFleet.mockReturnValue(new Promise(resolve => { resolveFleet = resolve; }));
    const disposeLogs = vi.fn();
    mounts.mountLogs.mockResolvedValue(disposeLogs);

    const { render } = await import('./index'); // import kicks off the slow #fleet render
    location.hash = '#logs';
    await render();

    const disposeFleet = vi.fn();
    resolveFleet(disposeFleet);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(disposeFleet).toHaveBeenCalledOnce();
    expect(disposeLogs).not.toHaveBeenCalled();

    await render();
    expect(disposeLogs).toHaveBeenCalledOnce();
  });
});
