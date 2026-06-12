import { describe, expect, it } from 'vitest';
import {
  activeLeaseExpirationsAfterStartup,
  cdpTargetTabIdsToDetachOnStartup,
  normalizePersistedCdpLeases,
  removePersistedCdpLeaseByTab,
  shouldDetachCdpTarget,
  splitCdpLeasesByExpiry,
  upsertPersistedCdpLease,
} from './cdpLeaseDurability';

describe('CDP lease durability decisions', () => {
  it('normalizes only valid persisted leases', () => {
    expect(normalizePersistedCdpLeases([
      { tabId: 1, expiresAtMs: 2000, purpose: 'debug' },
      { tabId: '2', expiresAtMs: '3000' },
      { tabId: -1, expiresAtMs: 3000 },
      { tabId: 3, expiresAtMs: Number.NaN },
    ])).toEqual([
      { tabId: 1, expiresAtMs: 2000, purpose: 'debug' },
      { tabId: 2, expiresAtMs: 3000, purpose: undefined },
    ]);
  });

  it('splits active and expired leases', () => {
    expect(splitCdpLeasesByExpiry([
      { tabId: 1, expiresAtMs: 999 },
      { tabId: 2, expiresAtMs: 1001 },
    ], 1000)).toEqual({
      active: [{ tabId: 2, expiresAtMs: 1001 }],
      expired: [{ tabId: 1, expiresAtMs: 999 }],
    });
  });

  it('upserts and removes persisted lease records for chrome.storage.session', () => {
    const leases = upsertPersistedCdpLease([
      { tabId: 1, expiresAtMs: 1000 },
      { tabId: 2, expiresAtMs: 1000 },
    ], { tabId: 1, expiresAtMs: 2000, purpose: 'reconcile' });

    expect(leases).toEqual([
      { tabId: 2, expiresAtMs: 1000 },
      { tabId: 1, expiresAtMs: 2000, purpose: 'reconcile' },
    ]);
    expect(removePersistedCdpLeaseByTab(leases, 2)).toEqual([
      { tabId: 1, expiresAtMs: 2000, purpose: 'reconcile' },
    ]);
  });

  it('restores only active lease expirations on service worker startup', () => {
    expect(activeLeaseExpirationsAfterStartup([
      { tabId: 1, expiresAtMs: 999 },
      { tabId: 2, expiresAtMs: 1500 },
    ], 1000)).toEqual([[2, 1500]]);
  });

  it('detaches attached targets without an active lease', () => {
    const leases = [{ tabId: 1, expiresAtMs: 2000 }];
    expect(shouldDetachCdpTarget({ attached: true, tabId: 1 }, leases, 1000)).toBe(false);
    expect(shouldDetachCdpTarget({ attached: true, tabId: 2 }, leases, 1000)).toBe(true);
    expect(shouldDetachCdpTarget({ attached: true, tabId: 1 }, leases, 2500)).toBe(true);
    expect(shouldDetachCdpTarget({ attached: false, tabId: 2 }, leases, 1000)).toBe(false);
  });

  it('selects orphaned debugger targets from mocked chrome.debugger.getTargets output', () => {
    const leases = [{ tabId: 1, expiresAtMs: 2000 }];
    expect(cdpTargetTabIdsToDetachOnStartup([
      { attached: true, tabId: 1 },
      { attached: true, tabId: 2 },
      { attached: false, tabId: 3 },
      { attached: true },
    ], leases, 1000)).toEqual([2]);
  });
});
