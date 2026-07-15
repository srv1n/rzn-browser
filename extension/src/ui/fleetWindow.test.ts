import { beforeEach, describe, expect, it, vi } from 'vitest';
import { FleetWindowRegistry } from './fleetWindow';

const tabsCreate = vi.fn();
const windowsCreate = vi.fn();
const windowsRemove = vi.fn();

beforeEach(() => {
  tabsCreate.mockReset();
  windowsCreate.mockReset();
  windowsRemove.mockReset();
  (globalThis as any).chrome = {
    tabs: { create: tabsCreate },
    windows: { create: windowsCreate, remove: windowsRemove },
  };
});

describe('fleet window isolation', () => {
  it('creates one unfocused fleet window and targets later tabs to it', async () => {
    windowsCreate.mockResolvedValue({ id: 71, tabs: [{ id: 701 }] });
    tabsCreate.mockResolvedValue({ id: 702, windowId: 71 });
    const windows = new FleetWindowRegistry();
    windows.rememberSession({ sessionId: 'session-1', origin: 'fleet', runId: 'run-1' });

    await windows.createTab('session-1', { url: 'https://one.example', active: true });
    await windows.createTab('session-1', { url: 'https://two.example', active: true });

    expect(windowsCreate).toHaveBeenCalledWith({ url: 'https://one.example', focused: false });
    expect(tabsCreate).toHaveBeenCalledWith({ url: 'https://two.example', active: true, windowId: 71 });
  });

  it('leaves local runs on the existing tab-create path', async () => {
    tabsCreate.mockResolvedValue({ id: 42 });
    const windows = new FleetWindowRegistry();
    windows.rememberSession({ sessionId: 'local-1', origin: 'local_cli', runId: 'run-local' });

    await windows.createTab('local-1', { url: 'https://local.example', active: true });

    expect(windowsCreate).not.toHaveBeenCalled();
    expect(tabsCreate).toHaveBeenCalledWith({ url: 'https://local.example', active: true });
  });

  it('closes fleet windows after a successful run', async () => {
    windowsCreate.mockResolvedValue({ id: 73, tabs: [{ id: 703 }] });
    windowsRemove.mockResolvedValue(undefined);
    const windows = new FleetWindowRegistry();
    windows.rememberSession({ sessionId: 'session-3', origin: 'fleet', runId: 'run-3' });
    await windows.createTab('session-3', { url: 'https://done.example', active: true });

    await expect(windows.closeSession('session-3', false)).resolves.toEqual({ handled: true, closed: true });
    expect(windowsRemove).toHaveBeenCalledWith(73);
  });

  it('keeps failed fleet windows when instructed and ignores an already-closed window', async () => {
    windowsCreate.mockResolvedValue({ id: 74, tabs: [{ id: 704 }] });
    const windows = new FleetWindowRegistry();
    windows.rememberSession({ sessionId: 'session-4', origin: 'fleet', runId: 'run-4' });
    await windows.createTab('session-4', { url: 'https://failed.example', active: true });

    await expect(windows.closeSession('session-4', true)).resolves.toEqual({ handled: true, closed: false });
    expect(windowsRemove).not.toHaveBeenCalled();

    windows.rememberSession({ sessionId: 'session-5', origin: 'fleet', runId: 'run-5' });
    windowsCreate.mockResolvedValue({ id: 75, tabs: [{ id: 705 }] });
    windowsRemove.mockRejectedValue(new Error('No window with id: 75'));
    await windows.createTab('session-5', { url: 'https://gone.example', active: true });
    await expect(windows.closeSession('session-5', false)).resolves.toEqual({ handled: true, closed: false });
  });
});
