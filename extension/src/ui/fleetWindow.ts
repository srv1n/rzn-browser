export type FleetSessionMetadata = {
  sessionId: string;
  origin?: string;
  runId?: string;
};

type FleetSession = { key: string };

export type FleetWindowCloseResult = {
  handled: boolean;
  closed: boolean;
};

/**
 * Keeps fleet windows tied to the run that owns them. Local sessions are never
 * recorded here, so their tab creation path stays exactly as it was.
 */
export class FleetWindowRegistry {
  private readonly sessions = new Map<string, FleetSession>();
  private readonly windows = new Map<string, number>();

  rememberSession({ sessionId, origin, runId }: FleetSessionMetadata): void {
    if (origin !== 'fleet') return;
    this.sessions.set(sessionId, { key: runId || sessionId });
  }

  isFleetSession(sessionId: string): boolean {
    return this.sessions.has(sessionId);
  }

  async createTab(
    sessionId: string,
    properties: chrome.tabs.CreateProperties,
  ): Promise<chrome.tabs.Tab> {
    const session = this.sessions.get(sessionId);
    if (!session) return await chrome.tabs.create(properties);

    const windowId = this.windows.get(session.key);
    if (windowId !== undefined) {
      return await chrome.tabs.create({ ...properties, windowId });
    }

    // `focused: false` leaves the user's active window alone. Deliberately do
    // not set state=minimized: minimized windows are occlusion-throttled.
    const window = await chrome.windows.create({
      url: properties.url || 'about:blank',
      focused: false,
    });
    if (typeof window.id !== 'number') {
      throw new Error('Fleet window creation returned no window id');
    }
    this.windows.set(session.key, window.id);

    const firstTab = window.tabs?.find((tab) => typeof tab.id === 'number');
    if (!firstTab) {
      throw new Error('Fleet window creation returned no tab');
    }
    return firstTab;
  }

  async closeSession(sessionId: string, keepWindow: boolean): Promise<FleetWindowCloseResult> {
    const session = this.sessions.get(sessionId);
    if (!session) return { handled: false, closed: false };
    this.sessions.delete(sessionId);

    const windowId = this.windows.get(session.key);
    if (windowId === undefined || keepWindow) {
      return { handled: true, closed: false };
    }
    this.windows.delete(session.key);

    try {
      await chrome.windows.remove(windowId);
      return { handled: true, closed: true };
    } catch {
      // The user may have already closed the window. Cleanup must not turn a
      // completed workflow into a failed run.
      return { handled: true, closed: false };
    }
  }
}
