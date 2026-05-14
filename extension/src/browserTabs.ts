function tabHasId(tab: chrome.tabs.Tab | undefined): tab is chrome.tabs.Tab & { id: number } {
  return typeof tab?.id === 'number' && Number.isFinite(tab.id);
}

function activeTabFromWindow(window: chrome.windows.Window | undefined): chrome.tabs.Tab | undefined {
  return window?.tabs?.find(tab => tab.active && tabHasId(tab));
}

async function getLastFocusedWindowActiveTab(): Promise<chrome.tabs.Tab | undefined> {
  if (!chrome.windows?.getLastFocused) return undefined;

  try {
    const window = await chrome.windows.getLastFocused({
      populate: true,
      windowTypes: ['normal'],
    } as any);
    const tab = activeTabFromWindow(window);
    if (tabHasId(tab)) return tab;
  } catch {}

  return undefined;
}

async function queryLastFocusedActiveTab(): Promise<chrome.tabs.Tab | undefined> {
  try {
    const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
    if (tabHasId(tab)) return tab;
  } catch {}

  return undefined;
}

async function queryAnyActiveNormalTab(): Promise<chrome.tabs.Tab | undefined> {
  try {
    const tabs = await chrome.tabs.query({ active: true });
    const normalTab = tabs.find(tab => tabHasId(tab) && tab.windowId !== chrome.windows.WINDOW_ID_NONE);
    if (tabHasId(normalTab)) return normalTab;
  } catch {}

  return undefined;
}

async function getAnyWindowActiveTab(): Promise<chrome.tabs.Tab | undefined> {
  if (!chrome.windows?.getAll) return undefined;

  try {
    const windows = await chrome.windows.getAll({
      populate: true,
      windowTypes: ['normal'],
    } as any);
    const focused = windows.find(window => window.focused);
    const focusedTab = activeTabFromWindow(focused);
    if (tabHasId(focusedTab)) return focusedTab;

    for (const window of windows) {
      const tab = activeTabFromWindow(window);
      if (tabHasId(tab)) return tab;
    }
  } catch {}

  return undefined;
}

export async function getActiveBrowserTab(): Promise<chrome.tabs.Tab | undefined> {
  return (
    await getLastFocusedWindowActiveTab() ??
    await queryLastFocusedActiveTab() ??
    await getAnyWindowActiveTab() ??
    await queryAnyActiveNormalTab()
  );
}

export async function getActiveBrowserTabId(action: string): Promise<number> {
  const tab = await getActiveBrowserTab();
  if (!tabHasId(tab)) {
    throw new Error(`No active browser tab available for ${action}`);
  }
  return tab.id;
}

