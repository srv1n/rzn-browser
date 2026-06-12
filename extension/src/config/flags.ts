// Feature flag registry with per-domain overrides and circuit breaker integration
// Provides typed configuration with safe defaults and domain-specific behavior

export type Flags = {
  typeAndSubmitRequired: boolean;
  batchActionsEnabled: boolean;
  stickyLeaseMs: number;
  iframesDefaultOn: boolean;
  axFirstExtraction: boolean;
  cdpEnable: boolean;
  maxMacroSteps: number;
  flightRecorder: boolean;
  nativeInputEnabled: boolean;
};

export type FlagOverrides = Record<string, Partial<Flags>>; // "*" and "example.com" keys

const DEFAULTS: Flags = {
  typeAndSubmitRequired: true,
  batchActionsEnabled: true,
  stickyLeaseMs: 1500,
  iframesDefaultOn: true,
  axFirstExtraction: true,
  // Default OFF to avoid chrome.debugger attach (shows "started debugging this browser" infobar).
  // Enable per-domain via chrome.storage.local["flags"] when needed.
  cdpEnable: false,
  maxMacroSteps: 12,
  flightRecorder: false,
  nativeInputEnabled: false,
};

/**
 * Get effective flags for a hostname, merging defaults with global and domain overrides
 */
export async function getFlags(hostname?: string): Promise<Flags> {
  try {
    const { flags = {} } = await chrome.storage.local.get("flags");
    const domain = hostname?.toLowerCase() || "";
    
    // Merge: defaults < global overrides < domain-specific overrides
    const merged: Flags = {
      ...DEFAULTS,
      ...(flags["*"] || {}),
      ...(domain && flags[domain] ? flags[domain] : {})
    };
    
    console.log(`[Flags] Resolved for ${domain || 'default'}:`, merged);
    return merged;
  } catch (error) {
    console.warn('[Flags] Failed to load flags, using defaults:', error);
    return DEFAULTS;
  }
}

/**
 * Set flag overrides for specific domains or globally
 */
export async function setFlags(overrides: FlagOverrides): Promise<void> {
  try {
    const { flags = {} } = await chrome.storage.local.get("flags");
    const updated = { ...flags, ...overrides };
    await chrome.storage.local.set({ flags: updated });
    console.log('[Flags] Updated:', overrides);
  } catch (error) {
    console.error('[Flags] Failed to set flags:', error);
  }
}

/**
 * Reset flags to defaults (for testing/debugging)
 */
export async function resetFlags(): Promise<void> {
  await chrome.storage.local.remove("flags");
  console.log('[Flags] Reset to defaults');
}

/**
 * Get current flag storage (for debugging)
 */
export async function getAllFlags(): Promise<FlagOverrides> {
  const { flags = {} } = await chrome.storage.local.get("flags");
  return flags;
}

// Remote flags fetcher (optional)
let lastRemoteFetch = 0;
const REMOTE_FETCH_INTERVAL = 3600000; // 1 hour

/**
 * Optionally fetch flags from remote source and merge with local
 */
export async function refreshRemoteFlags(remoteUrl?: string): Promise<void> {
  if (!remoteUrl) return;
  
  const now = Date.now();
  if (now - lastRemoteFetch < REMOTE_FETCH_INTERVAL) return;
  
  try {
    const response = await fetch(remoteUrl);
    const remoteFlags = await response.json();
    
    if (remoteFlags && typeof remoteFlags === 'object') {
      await setFlags({ "*": remoteFlags });
      lastRemoteFetch = now;
      console.log('[Flags] Refreshed from remote:', remoteUrl);
    }
  } catch (error) {
    console.warn('[Flags] Failed to fetch remote flags:', error);
  }
}
