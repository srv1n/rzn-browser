export interface PersistedCdpLease {
  tabId: number;
  expiresAtMs: number;
  purpose?: string;
}

export interface DebuggerTargetLike {
  attached?: boolean;
  tabId?: number;
}

export function normalizePersistedCdpLeases(raw: unknown): PersistedCdpLease[] {
  if (!Array.isArray(raw)) return [];
  const leases: PersistedCdpLease[] = [];
  for (const item of raw) {
    const tabId = Number((item as any)?.tabId);
    const expiresAtMs = Number((item as any)?.expiresAtMs);
    if (!Number.isInteger(tabId) || tabId < 0 || !Number.isFinite(expiresAtMs)) {
      continue;
    }
    const purpose = typeof (item as any)?.purpose === 'string' ? (item as any).purpose : undefined;
    leases.push({ tabId, expiresAtMs, purpose });
  }
  return leases;
}

export function activeCdpLeaseTabIds(leases: PersistedCdpLease[], nowMs: number): Set<number> {
  const active = new Set<number>();
  for (const lease of leases) {
    if (lease.expiresAtMs > nowMs) {
      active.add(lease.tabId);
    }
  }
  return active;
}

export function splitCdpLeasesByExpiry(leases: PersistedCdpLease[], nowMs: number): {
  active: PersistedCdpLease[];
  expired: PersistedCdpLease[];
} {
  const active: PersistedCdpLease[] = [];
  const expired: PersistedCdpLease[] = [];
  for (const lease of leases) {
    if (lease.expiresAtMs > nowMs) {
      active.push(lease);
    } else {
      expired.push(lease);
    }
  }
  return { active, expired };
}

export function upsertPersistedCdpLease(
  leases: PersistedCdpLease[],
  lease: PersistedCdpLease
): PersistedCdpLease[] {
  return [
    ...leases.filter((existing) => existing.tabId !== lease.tabId),
    lease,
  ];
}

export function removePersistedCdpLeaseByTab(
  leases: PersistedCdpLease[],
  tabId: number
): PersistedCdpLease[] {
  return leases.filter((lease) => lease.tabId !== tabId);
}

export function activeLeaseExpirationsAfterStartup(
  leases: PersistedCdpLease[],
  nowMs: number
): Array<[number, number]> {
  return leases
    .filter((lease) => lease.expiresAtMs > nowMs)
    .map((lease) => [lease.tabId, lease.expiresAtMs]);
}

export function shouldDetachCdpTarget(
  target: DebuggerTargetLike,
  leases: PersistedCdpLease[],
  nowMs: number
): boolean {
  if (!target.attached || typeof target.tabId !== 'number') {
    return false;
  }
  return !activeCdpLeaseTabIds(leases, nowMs).has(target.tabId);
}

export function cdpTargetTabIdsToDetachOnStartup(
  targets: DebuggerTargetLike[],
  leases: PersistedCdpLease[],
  nowMs: number
): number[] {
  return targets
    .filter((target) => shouldDetachCdpTarget(target, leases, nowMs))
    .map((target) => target.tabId)
    .filter((tabId): tabId is number => typeof tabId === 'number');
}
