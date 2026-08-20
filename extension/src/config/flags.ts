// Feature flag registry with per-domain overrides and circuit breaker integration
// Provides typed configuration with safe defaults and domain-specific behavior

export type Flags = {
  batchActionsEnabled: boolean;
  stickyLeaseMs: number;
  iframesDefaultOn: boolean;
  axFirstExtraction: boolean;
  cdpEnable: boolean;
  flightRecorder: boolean;
  nativeInputEnabled: boolean;
};

export type FlagOverrides = Record<string, Partial<Flags>>; // "*" and "example.com" keys

const DEFAULTS: Flags = {
  batchActionsEnabled: true,
  stickyLeaseMs: 1500,
  iframesDefaultOn: true,
  axFirstExtraction: true,
  // Default OFF to avoid chrome.debugger attach (shows "started debugging this browser" infobar).
  // Enable per-domain via chrome.storage.local["flags"] when needed.
  cdpEnable: false,
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
