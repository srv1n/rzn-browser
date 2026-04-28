// Background script for RZN Browser Automation
// Handles communication between broker and content script
import { pruneDOM } from './domPrune';

import { logInfo, logError, logDebug, logWarn, setNativePort } from './logger';
import { cdpIntegration } from './cdp/integration';
import { getAccessibilitySnapshot, getInteractiveElements as cdpGetInteractiveElements } from './cdp';
import { ExecutionTier } from './cdp/executionStrategy';
import { frameRouter } from './cdp/frameRouter';
import { setFlags, getFlags } from './config/flags';
import { cdpSessionManager } from './runtime/cdp_session_manager';

// CDP Lease Manager - keeps sessions alive briefly to avoid repeated attach/detach
type TabId = number;
const CDP_LEASE_TTL_MS = 1500;
const leaseTimers = new Map<TabId, number>(); // timeout id
const leaseExpirations = new Map<TabId, number>();

function ensureLeaseTimer(tabId: TabId) {
  if (leaseTimers.has(tabId)) return;
  const intervalId = setInterval(() => {
    void maybeExpireCdpLease(tabId, intervalId);
  }, 300) as unknown as number;
  leaseTimers.set(tabId, intervalId);
}

async function maybeExpireCdpLease(tabId: TabId, intervalId: number): Promise<void> {
  const exp = leaseExpirations.get(tabId) ?? 0;
  if (Date.now() <= exp) {
    return;
  }

  clearInterval(intervalId);
  leaseTimers.delete(tabId);

  await withCdpLock(async () => {
    const currentExp = leaseExpirations.get(tabId) ?? 0;
    if (Date.now() <= currentExp) {
      ensureLeaseTimer(tabId);
      return;
    }

    leaseExpirations.delete(tabId);
    await frameRouter.detachFromTab(tabId).catch(() => {
      console.warn(`Failed to detach CDP from tab ${tabId} after lease expiry`);
    });
  });
}

function setCdpLeaseExpiration(tabId: TabId, expiresAtMs: number) {
  // Never shorten an existing lease (e.g., when a long break-glass lease is active).
  const prev = leaseExpirations.get(tabId) ?? 0;
  leaseExpirations.set(tabId, Math.max(prev, expiresAtMs));
  ensureLeaseTimer(tabId);
}

async function getTabHostname(tabId: number): Promise<string | null> {
  try {
    const tab = await chrome.tabs.get(tabId);
    const url = tab.url || tab.pendingUrl || '';
    if (!/^https?:\/\//i.test(url)) return null;
    return new URL(url).hostname;
  } catch {
    return null;
  }
}

async function isCDPEnabledForTab(tabId: number): Promise<boolean> {
  const exp = leaseExpirations.get(tabId) ?? 0;
  if (Date.now() <= exp) return true;
  const host = await getTabHostname(tabId);
  const flags = await getFlags(host || undefined);
  return !!flags.cdpEnable;
}

export async function extendCDPLease(tabId: number) {
  // Get dynamic lease TTL from flags based on current tab's domain
  let leaseMs = CDP_LEASE_TTL_MS;
  try {
    const tab = await chrome.tabs.get(tabId);
    if (tab.url) {
      const host = new URL(tab.url).hostname;
      const flags = await getFlags(host);
      leaseMs = Math.max(200, flags.stickyLeaseMs || CDP_LEASE_TTL_MS);
    }
  } catch (error) {
    console.warn('Failed to get dynamic lease TTL, using default:', error);
  }

  setCdpLeaseExpiration(tabId, Date.now() + leaseMs);
}

export async function forceDetachCDP(tabId: number) {
  const timerId = leaseTimers.get(tabId);
  if (timerId) {
    clearInterval(timerId);
    leaseTimers.delete(tabId);
  }
  leaseExpirations.delete(tabId);
  await cdpSessionManager.releaseTab(tabId);
}

async function releaseCdpSessionResources(sessionId: string): Promise<number[]> {
  const detachedTabs = await cdpSessionManager.releaseSession(sessionId);
  for (const tabId of detachedTabs) {
    const timerId = leaseTimers.get(tabId);
    if (timerId) {
      clearInterval(timerId);
      leaseTimers.delete(tabId);
    }
    leaseExpirations.delete(tabId);
  }
  return detachedTabs;
}

async function resetCdpTabResources(tabId: number): Promise<void> {
  const timerId = leaseTimers.get(tabId);
  if (timerId) {
    clearInterval(timerId);
    leaseTimers.delete(tabId);
  }
  leaseExpirations.delete(tabId);
  await cdpSessionManager.releaseTab(tabId);
}

async function buildCapabilities(tabId: number) {
  const host = await getTabHostname(tabId);
  const cdpAttached = frameRouter.isAttachedToTab(tabId);
  const cdpEnabled = await isCDPEnabledForTab(tabId);
  return {
    extension_actor: true,
    cdp_available: typeof chrome.debugger !== 'undefined',
    cdp_enabled: !!cdpEnabled,
    cdp_attached: !!cdpAttached,
    hostname: host || undefined,
  };
}

// Circuit Breaker - per-domain failure tracking and automatic flag adjustments
type HostWindowEntry = { 
  ok: boolean; 
  dur: number; 
  cdpError: boolean; 
  ts: number; 
};

const hostWindows = new Map<string, HostWindowEntry[]>();

/**
 * Record action metrics for circuit breaker analysis
 */
export function recordActionMetric(host: string, ok: boolean, durMs: number, cdpError = false) {
  const key = host.toLowerCase();
  const arr = hostWindows.get(key) || [];
  arr.push({ ok, dur: durMs, cdpError, ts: Date.now() });

  // Keep last 5 minutes or 200 entries max
  const cutoff = Date.now() - (5 * 60 * 1000);
  while (arr.length > 200 || (arr[0] && arr[0].ts < cutoff)) {
    arr.shift();
  }

  hostWindows.set(key, arr);

  console.log(`[CircuitBreaker] Recorded for ${host}: ok=${ok}, dur=${durMs}ms, cdpError=${cdpError}`);
}

/**
 * Analyze recent metrics and update flags for problematic domains
 */
async function recomputeCircuitBreakers() {
  const overrides: Record<string, any> = {};

  for (const [host, arr] of hostWindows) {
    if (!arr.length) continue;

    const total = arr.length;
    const failures = arr.filter(a => !a.ok).length;
    const avg = Math.round(arr.reduce((s, a) => s + a.dur, 0) / total);
    const cdpErrRate = arr.filter(a => a.cdpError).length / total;
    const failureRate = failures / total;

    console.log(`[CircuitBreaker] ${host}: failure=${(failureRate*100).toFixed(1)}%, avg=${avg}ms, cdpErr=${(cdpErrRate*100).toFixed(1)}%`);

    const override: any = {};
    let changed = false;

    // High failure rate or slow responses
    if (failureRate > 0.30 || avg > 3000) {
      override.batchActionsEnabled = false;
      changed = true;
      console.log(`[CircuitBreaker] Disabling batch actions for ${host} (failure=${(failureRate*100).toFixed(1)}%, avg=${avg}ms)`);

      // If CDP errors are high, disable CDP entirely
      if (cdpErrRate > 0.10) {
        override.cdpEnable = false;
        changed = true;
        console.log(`[CircuitBreaker] Disabling CDP for ${host} (cdpErr=${(cdpErrRate*100).toFixed(1)}%)`);
      }

      // Reduce lease time to minimize resource contention
      override.stickyLeaseMs = 800;
      changed = true;
    }

    // Recovery logic - if things improve, gradually re-enable
    if (failureRate < 0.05 && avg < 1000 && cdpErrRate < 0.02) {
      // Domain is performing well, could restore defaults
      const currentFlags = await getFlags(host);
      if (!currentFlags.batchActionsEnabled || !currentFlags.cdpEnable) {
        console.log(`[CircuitBreaker] ${host} recovering, considering flag restoration`);
        // Note: We don't auto-restore here to avoid flip-flopping
        // Manual intervention or longer observation period recommended
      }
    }

    if (changed) {
      overrides[host] = override;
    }
  }

  if (Object.keys(overrides).length) {
    console.log('[CircuitBreaker] Updating flags for domains:', Object.keys(overrides));
    await setFlags(overrides);
  }
}

// Run circuit breaker analysis every minute
setInterval(recomputeCircuitBreakers, 60_000);

// Single canonical native host name (keep in sync with rznapp + setup.sh).
const BROKER_HOST_CANDIDATES = ['com.rzn.browser.broker'] as const;
let brokerHostIndex = 0;
let brokerHostInUse: string | null = null;

// DOM Snapshot Types
interface ElementStub {
  id?: string;
  tag: string;
  text?: string;
  attributes: Record<string, string>;
  selector: string;
  spatial_info?: {
    x: number;
    y: number;
    width: number;
    height: number;
    area: number;
    viewport_percentage: number;
  };
}

interface DOMSnapshot {
  elements: ElementStub[];
  hash: string;
  prompt: string;
  metadata: {
    timestamp: number;
    url: string;
    title: string;
    viewport: { width: number; height: number };
  };
  delta?: {
    added: ElementStub[];
    removed: ElementStub[];
    modified: ElementStub[];
  };
}

// Observe cache: hostname|instrHash|scope|dom_hash -> candidates
type ObserveCacheEntry = { ts: number; dom_hash: string; candidates: any[] };
const observeCache = new Map<string, ObserveCacheEntry>();
const OBSERVE_TTL_MS = 120_000;

function simpleHash(str: string): string {
  let h = 0;
  for (let i = 0; i < str.length; i++) {
    h = ((h << 5) - h) + str.charCodeAt(i);
    h |= 0;
  }
  return Math.abs(h).toString(16);
}

interface BrokerMessage {
  // Support both formats for compatibility
  cmd?: string;          // Extension format
  action?: string;       // Orchestrator format
  req_id?: string;       // Extension format
  task_id?: string;      // Orchestrator format  
  plan_id?: string;
  payload?: any;
  task?: any;            // Orchestrator format
  data?: any;            // Orchestrator format
  dom_snapshot?: DOMSnapshot;    // New DOM snapshot format
  dom_hash?: string;     // DOM hash for delta tracking
}

interface ExtensionResponse {
  req_id?: string;       // Extension format
  task_id?: string;      // Orchestrator format
  action?: string;       // Orchestrator format (for responses)
  success: boolean;
  error_code?: string;
  error_msg?: string;
  error?: string;        // Orchestrator format
  validation_passed?: boolean;
  result?: any;          // For step results
  dom_snapshot?: DOMSnapshot;    // New DOM snapshot format
  dom_hash?: string;     // DOM hash for delta tracking
  current_url?: string;  // Current page URL
}

let nativePort: chrome.runtime.Port | null = null;
const CLOUD_ACTOR_CONFIG_VERSION = 'rzn.cloud.actor_config.v1';
const CLOUD_UI_NATIVE_TIMEOUT_MS = 10_000;
const CLOUD_UI_PAIRING_TTL_SECS = 600;
type NativeControlCallback = {
  responseCmd?: string;
  resolve: (message: BrokerMessage) => void;
  reject: (error: Error) => void;
  timeoutId: number;
};
type CloudActorStatus = {
  supported: boolean;
  actor_mode: string;
  config_path: string;
  configured: boolean;
  connected: boolean;
  actor_id?: string;
  workspace_id?: string;
  server_url?: string;
  websocket_url?: string;
  paired_at_ms?: number;
  connect_timeout_ms?: number;
  request_timeout_ms?: number;
  last_connected_at_ms?: number;
  last_ready_at_ms?: number;
  last_error?: string;
};
const nativeControlCallbacks = new Map<string, NativeControlCallback>();

function clearNativeControlCallback(reqId: string): void {
  const callback = nativeControlCallbacks.get(reqId);
  if (!callback) return;
  clearTimeout(callback.timeoutId);
  nativeControlCallbacks.delete(reqId);
}

function rejectAllNativeControlCallbacks(reason: string): void {
  for (const [reqId, callback] of nativeControlCallbacks.entries()) {
    clearTimeout(callback.timeoutId);
    callback.reject(new Error(reason));
    nativeControlCallbacks.delete(reqId);
  }
}

function maybeResolveNativeControlCallback(message: BrokerMessage): boolean {
  const reqId = message.req_id || message.task_id;
  if (!reqId) return false;
  const callback = nativeControlCallbacks.get(reqId);
  if (!callback) return false;
  if (callback.responseCmd && message.cmd !== callback.responseCmd) {
    return false;
  }
  clearNativeControlCallback(reqId);
  callback.resolve(message);
  return true;
}

async function ensureNativeHostConnected(timeoutMs = 1500): Promise<void> {
  if (nativePort) return;
  connectToNative();
  const deadline = Date.now() + timeoutMs;
  while (!nativePort && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  if (!nativePort) {
    throw new Error('Native host is not connected');
  }
}

async function callNativeHostControl(
  cmd: string,
  payload: any = {},
  options?: { timeoutMs?: number; responseCmd?: string }
): Promise<BrokerMessage> {
  await ensureNativeHostConnected();
  if (!nativePort) {
    throw new Error('Native host is not connected');
  }

  const reqId = `cloud-ui-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const timeoutMs = Math.max(1000, options?.timeoutMs ?? CLOUD_UI_NATIVE_TIMEOUT_MS);
  return await new Promise<BrokerMessage>((resolve, reject) => {
    const timeoutId = setTimeout(() => {
      nativeControlCallbacks.delete(reqId);
      reject(new Error(`Native host timeout for ${cmd}`));
    }, timeoutMs) as unknown as number;

    nativeControlCallbacks.set(reqId, {
      responseCmd: options?.responseCmd || `${cmd}_response`,
      resolve,
      reject,
      timeoutId,
    });

    try {
      nativePort!.postMessage({
        cmd,
        req_id: reqId,
        payload,
      } as BrokerMessage);
    } catch (error: any) {
      clearNativeControlCallback(reqId);
      reject(new Error(error?.message || String(error)));
    }
  });
}
// Reconnect/backoff management for native host
let reconnectTimer: number | null = null;
let reconnectAttempts = 0;
let nativeConnectInFlight = false;
const RECONNECT_BASE_DELAY_MS = 1000;
const RECONNECT_MAX_DELAY_MS = 15000;
const RECONNECT_ALARM_NAME = 'rzn_native_reconnect';
const NATIVE_KEEPALIVE_ALARM_NAME = 'rzn_native_keepalive';
const NATIVE_KEEPALIVE_PERIOD_MINUTES = 0.5;

// Heartbeat management
let heartbeatTimer: number | null = null;
let missedNativeHeartbeats = 0;
let suppressNextNativeDisconnect = false;
const HEARTBEAT_INTERVAL_MS = 10000;
const HEARTBEAT_TIMEOUT_MS = 5000;
const HEARTBEAT_MISSES_BEFORE_RECONNECT = 2;

function clearHeartbeat() {
  if (heartbeatTimer !== null) {
    clearInterval(heartbeatTimer);
    heartbeatTimer = null;
  }
  missedNativeHeartbeats = 0;
}

function startHeartbeat() {
  clearHeartbeat();
  heartbeatTimer = setInterval(() => {
    void (async () => {
      try {
        if (!nativePort) return;
        await callNativeHostControl(
          'ping',
          { source: 'extension_keepalive' },
          { timeoutMs: HEARTBEAT_TIMEOUT_MS, responseCmd: 'ping_response' }
        );
        missedNativeHeartbeats = 0;
      } catch (e) {
        missedNativeHeartbeats += 1;
        console.warn(
          `[Heartbeat] native host ping missed (${missedNativeHeartbeats}/${HEARTBEAT_MISSES_BEFORE_RECONNECT})`,
          e
        );
        if (missedNativeHeartbeats >= HEARTBEAT_MISSES_BEFORE_RECONNECT) {
          disconnectNativePort('heartbeat missed native host echo');
          scheduleReconnect();
        }
      }
    })();
  }, HEARTBEAT_INTERVAL_MS) as unknown as number;
}

function clearReconnectTimer() {
  if (reconnectTimer !== null) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  try {
    if (chrome.alarms?.clear) {
      chrome.alarms.clear(RECONNECT_ALARM_NAME);
    }
  } catch (e) {
    console.warn('[NativeReconnect] Failed to clear alarm:', e);
  }
}

function scheduleReconnectAlarm(delayMs: number) {
  // MV3 service workers can be suspended, and timers are not guaranteed to survive.
  // Use alarms as a best-effort wakeup so reconnect continues even after suspension.
  try {
    if (!chrome.alarms?.create) return;
    const when = Date.now() + Math.max(1000, delayMs);
    chrome.alarms.create(RECONNECT_ALARM_NAME, { when });
  } catch (e) {
    console.warn('[NativeReconnect] Failed to schedule alarm:', e);
  }
}

function ensureNativeKeepaliveAlarm() {
  // MV3 service workers can be suspended without an explicit native port disconnect callback.
  // A lightweight periodic alarm gives the extension a chance to wake up and reconnect the
  // native host before the next native-run attaches.
  try {
    if (!chrome.alarms?.create) return;
    chrome.alarms.create(NATIVE_KEEPALIVE_ALARM_NAME, {
      periodInMinutes: NATIVE_KEEPALIVE_PERIOD_MINUTES,
    });
  } catch (e) {
    console.warn('[NativeKeepalive] Failed to schedule keepalive alarm:', e);
  }
}

function scheduleReconnect() {
  // Avoid duplicate timers
  if (reconnectTimer !== null) return;
  // Compute exponential backoff with cap
  const delay = Math.min(
    RECONNECT_MAX_DELAY_MS,
    RECONNECT_BASE_DELAY_MS * Math.pow(2, Math.max(0, reconnectAttempts))
  );
  console.log(`[NativeReconnect] Scheduling reconnect in ${delay}ms (attempt ${reconnectAttempts + 1})`);
  scheduleReconnectAlarm(delay);
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    reconnectAttempts++;
    connectToNative();
  }, delay) as unknown as number;
}

function disconnectNativePort(reason: string): void {
  const port = nativePort;
  if (!port) return;

  console.warn('[NativeReconnect] Tearing down native port:', reason);
  suppressNextNativeDisconnect = true;
  nativePort = null;
  nativeConnectInFlight = false;
  setNativePort(null);
  clearHeartbeat();
  rejectAllNativeControlCallbacks(reason);

  try {
    port.disconnect();
  } catch (error) {
    console.warn('[NativeReconnect] Native port disconnect failed:', error);
  }
}

const DEFAULT_WORKFLOW_SESSION_ID = 'default';
const WORKFLOW_SESSION_STORAGE_KEY = 'workflow_sessions_v1';

interface WorkflowSessionState {
  workflowTabId?: number;
  currentUrl?: string;
  updatedAtMs?: number;
  queue: Promise<void>;
}

interface StoredWorkflowSessionState {
  workflowTabId?: number;
  currentUrl?: string;
  updatedAtMs: number;
}

const workflowSessions = new Map<string, WorkflowSessionState>();
let cdpLockQueue: Promise<void> = Promise.resolve();
let workflowSessionsLoaded = false;
let workflowSessionsLoadPromise: Promise<void> | null = null;
let workflowSessionsPersistChain: Promise<void> = Promise.resolve();

function normalizeSessionId(raw: any): string {
  if (typeof raw === 'string' && raw.trim().length > 0) {
    return raw.trim();
  }
  return DEFAULT_WORKFLOW_SESSION_ID;
}

function extractSessionIdCandidate(message?: BrokerMessage): any {
  return (
    (message as any)?.data?.session_id ??
    (message as any)?.payload?.session_id
  );
}

function resolveSessionId(message?: BrokerMessage): string {
  return normalizeSessionId(extractSessionIdCandidate(message));
}

function resolveRequestedTabId(message?: BrokerMessage): number | undefined {
  const candidate =
    (message as any)?.data?.current_tab_id ??
    (message as any)?.payload?.current_tab_id;
  return typeof candidate === 'number' && Number.isFinite(candidate) ? candidate : undefined;
}

function isDefaultWorkflowSession(sessionId?: string): boolean {
  return normalizeSessionId(sessionId) === DEFAULT_WORKFLOW_SESSION_ID;
}

function sessionMayUseActiveTab(sessionId: string, preferCurrentTab = false): boolean {
  return preferCurrentTab || isDefaultWorkflowSession(sessionId);
}

function buildMissingWorkflowTabError(sessionId: string, action: string): string {
  return `No dedicated workflow tab available for ${action} (session=${normalizeSessionId(sessionId)}).`;
}

async function loadWorkflowSessionsFromStorage(): Promise<void> {
  if (workflowSessionsLoaded) {
    return;
  }
  if (!workflowSessionsLoadPromise) {
    workflowSessionsLoadPromise = (async () => {
      try {
        const stored = await chrome.storage.local.get(WORKFLOW_SESSION_STORAGE_KEY);
        const raw = stored?.[WORKFLOW_SESSION_STORAGE_KEY];
        if (raw && typeof raw === 'object') {
          for (const [sessionId, value] of Object.entries(raw as Record<string, StoredWorkflowSessionState>)) {
            if (!value || typeof value !== 'object') continue;
            const normalized = normalizeSessionId(sessionId);
            workflowSessions.set(normalized, {
              workflowTabId:
                typeof value.workflowTabId === 'number' && Number.isFinite(value.workflowTabId)
                  ? value.workflowTabId
                  : undefined,
              currentUrl: typeof value.currentUrl === 'string' ? value.currentUrl : undefined,
              updatedAtMs:
                typeof value.updatedAtMs === 'number' && Number.isFinite(value.updatedAtMs)
                  ? value.updatedAtMs
                  : undefined,
              queue: Promise.resolve(),
            });
          }
        }
      } catch (error) {
        console.warn('[WorkflowSessions] Failed to load from storage:', error);
      } finally {
        workflowSessionsLoaded = true;
      }
    })();
  }
  await workflowSessionsLoadPromise;
}

function snapshotWorkflowSessionsForStorage(): Record<string, StoredWorkflowSessionState> {
  const snapshot: Record<string, StoredWorkflowSessionState> = {};
  for (const [sessionId, state] of workflowSessions.entries()) {
    if (sessionId !== DEFAULT_WORKFLOW_SESSION_ID && state.workflowTabId === undefined && !state.currentUrl) {
      continue;
    }
    snapshot[sessionId] = {
      workflowTabId: state.workflowTabId,
      currentUrl: state.currentUrl,
      updatedAtMs: state.updatedAtMs || Date.now(),
    };
  }
  return snapshot;
}

function scheduleWorkflowSessionsPersist(): void {
  const snapshot = snapshotWorkflowSessionsForStorage();
  workflowSessionsPersistChain = workflowSessionsPersistChain
    .catch(() => {})
    .then(async () => {
      try {
        await chrome.storage.local.set({
          [WORKFLOW_SESSION_STORAGE_KEY]: snapshot,
        });
      } catch (error) {
        console.warn('[WorkflowSessions] Failed to persist session store:', error);
      }
    });
}

function updateWorkflowSessionMetadata(
  sessionId: string,
  updates: Partial<Pick<WorkflowSessionState, 'workflowTabId' | 'currentUrl'>>
): void {
  const state = getWorkflowSessionState(sessionId);
  if (Object.prototype.hasOwnProperty.call(updates, 'workflowTabId')) {
    state.workflowTabId = updates.workflowTabId;
  }
  if (Object.prototype.hasOwnProperty.call(updates, 'currentUrl')) {
    state.currentUrl = updates.currentUrl;
  }
  state.updatedAtMs = Date.now();
  scheduleWorkflowSessionsPersist();
}

function refreshWorkflowSessionUrl(sessionId: string, tabId: number | undefined): void {
  if (tabId === undefined) {
    updateWorkflowSessionMetadata(sessionId, { currentUrl: undefined });
    return;
  }

  void chrome.tabs
    .get(tabId)
    .then((tab) => {
      updateWorkflowSessionMetadata(sessionId, {
        currentUrl: tab.url || tab.pendingUrl || undefined,
      });
    })
    .catch(() => {
      updateWorkflowSessionMetadata(sessionId, { currentUrl: undefined });
    });
}

async function getActiveTabIdOrThrow(action: string): Promise<number> {
  const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
  const activeTabId = tabs[0]?.id;
  if (activeTabId === undefined) {
    throw new Error(`No active tab available for ${action}`);
  }
  return activeTabId;
}

function getWorkflowSessionState(sessionId: string): WorkflowSessionState {
  const normalized = normalizeSessionId(sessionId);
  let state = workflowSessions.get(normalized);
  if (!state) {
    state = { queue: Promise.resolve() };
    workflowSessions.set(normalized, state);
  }
  return state;
}

function getWorkflowTabId(sessionId: string): number | undefined {
  return getWorkflowSessionState(sessionId).workflowTabId;
}

function setWorkflowTabId(sessionId: string, tabId: number | undefined): void {
  const normalized = normalizeSessionId(sessionId);
  updateWorkflowSessionMetadata(normalized, { workflowTabId: tabId });
  refreshWorkflowSessionUrl(normalized, tabId);
}

async function withCdpLock<T>(run: () => Promise<T>): Promise<T> {
  let release!: () => void;
  const waitForTurn = cdpLockQueue;
  cdpLockQueue = new Promise<void>((resolve) => {
    release = resolve;
  });

  await waitForTurn;
  try {
    return await run();
  } finally {
    release();
  }
}

// Clear cached tab IDs if a tab is closed by the user.
chrome.tabs.onRemoved.addListener((tabId) => {
  for (const [sessionId, state] of workflowSessions.entries()) {
    if (state.workflowTabId === tabId) {
      state.workflowTabId = undefined;
      state.currentUrl = undefined;
      state.updatedAtMs = Date.now();
      // Keep map entries lightweight for active/default sessions; otherwise prune.
      if (sessionId !== DEFAULT_WORKFLOW_SESSION_ID) {
        workflowSessions.set(sessionId, state);
      }
      scheduleWorkflowSessionsPersist();
    }
  }
});

// Wait for navigation events using webNavigation or tabs APIs
async function waitForNavigation(
  tabId: number,
  wait: any,
  timeoutMs: number
): Promise<void> {
  let waitCondition: 'load' | 'domcontentloaded' | 'networkidle' = 'domcontentloaded';
  let extraDelay = 0;

  if (typeof wait === 'string') {
    if (['load', 'domcontentloaded', 'networkidle'].includes(wait)) {
      waitCondition = wait as any;
    } else if (!isNaN(Number(wait))) {
      extraDelay = Number(wait);
    }
  } else if (typeof wait === 'number') {
    extraDelay = wait;
  }

  let timeoutHandle: number;

  const eventPromise = new Promise<void>((resolve) => {
    const cleanup = () => {
      chrome.tabs.onUpdated.removeListener(onUpdated);
      chrome.webNavigation.onDOMContentLoaded.removeListener(onDomContentLoaded);
      chrome.webNavigation.onCompleted.removeListener(onCompleted);
      clearTimeout(timeoutHandle);
    };

    const done = () => {
      cleanup();
      resolve();
    };

    const onUpdated = (id: number, info: chrome.tabs.TabChangeInfo) => {
      if (waitCondition === 'load' && id === tabId && info.status === 'complete') {
        done();
      }
    };

    const onDomContentLoaded = (
      details: chrome.webNavigation.WebNavigationFramedCallbackDetails
    ) => {
      if (waitCondition === 'domcontentloaded' && details.tabId === tabId && details.frameId === 0) {
        done();
      }
    };

    const onCompleted = (details: chrome.webNavigation.WebNavigationFramedCallbackDetails) => {
      if (details.tabId === tabId && details.frameId === 0) {
        if (waitCondition === 'load' || waitCondition === 'networkidle') {
          done();
        }
      }
    };

    if (waitCondition === 'domcontentloaded') {
      chrome.webNavigation.onDOMContentLoaded.addListener(onDomContentLoaded, { tabId, frameId: 0 });
    } else {
      chrome.webNavigation.onCompleted.addListener(onCompleted, { tabId, frameId: 0 });
      if (waitCondition === 'load') {
        chrome.tabs.onUpdated.addListener(onUpdated);
      }
    }

    timeoutHandle = setTimeout(done, timeoutMs);
  });

  await Promise.race([eventPromise]);

  if (waitCondition === 'networkidle') {
    // Approximate network idle with small delay after load
    await new Promise((r) => setTimeout(r, 500));
  }

  if (extraDelay > 0) {
    await new Promise((r) => setTimeout(r, extraDelay));
  }
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function urlMatchesGlobPattern(url: string, pattern: string): boolean {
  const p = String(pattern ?? '').trim();
  if (!p) return true;

  // If no wildcards, treat as substring match (more forgiving).
  if (!p.includes('*')) {
    return url.includes(p);
  }

  try {
    const escaped = escapeRegExp(p)
      .replace(/\\\*\\\*/g, '.*')
      .replace(/\\\*/g, '.*');
    const re = new RegExp(`^${escaped}$`);
    return re.test(url);
  } catch {
    return url.includes(p.replace(/\*/g, ''));
  }
}

async function waitForNavigationOrUrlMatch(
  tabId: number,
  urlPattern: string | undefined,
  timeoutMs: number
): Promise<string> {
  const start = Date.now();
  let timeoutHandle: number;

  return await new Promise((resolve, reject) => {
    const cleanup = () => {
      if (chrome.tabs?.onUpdated?.removeListener) {
        chrome.tabs.onUpdated.removeListener(onUpdated);
      }
      if (chrome.webNavigation?.onCompleted?.removeListener) {
        chrome.webNavigation.onCompleted.removeListener(onCompleted);
      }
      if (chrome.webNavigation?.onCommitted?.removeListener) {
        chrome.webNavigation.onCommitted.removeListener(onCommitted);
      }
      if (chrome.webNavigation?.onHistoryStateUpdated?.removeListener) {
        chrome.webNavigation.onHistoryStateUpdated.removeListener(onHistory);
      }
      clearTimeout(timeoutHandle);
    };

    const done = (url: string) => {
      cleanup();
      resolve(url);
    };

    const fail = (err: any) => {
      cleanup();
      reject(err);
    };

    const checkTab = async (urlHint?: string) => {
      try {
        const tab = await chrome.tabs.get(tabId);
        const url = urlHint || tab.url || tab.pendingUrl || '';

        if (urlPattern) {
          if (url && urlMatchesGlobPattern(url, urlPattern)) {
            // If it's still loading, give it a short bounded chance to settle.
            if (tab.status !== 'complete') {
              const remaining = Math.max(0, timeoutMs - (Date.now() - start));
              await waitForTabComplete(tabId, Math.min(remaining, 15000)).catch(() => {});
            }
            done(url);
          }
          return;
        }

        // No pattern: wait until tab completes.
        if (tab.status === 'complete') {
          done(url);
        }
      } catch {
        // Ignore transient tab lookup errors.
      }
    };

    const onUpdated = (id: number, info: chrome.tabs.TabChangeInfo, tab?: chrome.tabs.Tab) => {
      if (id !== tabId) return;
      if (info.url) {
        void checkTab(info.url);
        return;
      }
      if (info.status === 'complete') {
        void checkTab(tab?.url);
      }
    };

    const onCommitted = (details: chrome.webNavigation.WebNavigationFramedCallbackDetails) => {
      if (details.tabId !== tabId || details.frameId !== 0) return;
      void checkTab(details.url);
    };

    const onHistory = (details: chrome.webNavigation.WebNavigationFramedCallbackDetails) => {
      if (details.tabId !== tabId || details.frameId !== 0) return;
      void checkTab(details.url);
    };

    const onCompleted = (details: chrome.webNavigation.WebNavigationFramedCallbackDetails) => {
      if (details.tabId !== tabId || details.frameId !== 0) return;
      void checkTab(details.url);
    };

    if (guardListener(chrome.tabs?.onUpdated, 'chrome.tabs.onUpdated')) {
      chrome.tabs.onUpdated.addListener(onUpdated);
    }
    if (guardListener(chrome.webNavigation?.onCommitted, 'chrome.webNavigation.onCommitted')) {
      chrome.webNavigation.onCommitted.addListener(onCommitted, { tabId, frameId: 0 });
    }
    if (
      guardListener(
        chrome.webNavigation?.onHistoryStateUpdated,
        'chrome.webNavigation.onHistoryStateUpdated'
      )
    ) {
      chrome.webNavigation.onHistoryStateUpdated.addListener(onHistory, { tabId, frameId: 0 });
    }
    if (guardListener(chrome.webNavigation?.onCompleted, 'chrome.webNavigation.onCompleted')) {
      chrome.webNavigation.onCompleted.addListener(onCompleted, { tabId, frameId: 0 });
    }

    timeoutHandle = setTimeout(() => {
      const msg = urlPattern
        ? `wait_for_navigation timeout waiting for url_pattern=${urlPattern}`
        : 'wait_for_navigation timeout waiting for tab to complete';
      fail(new Error(msg));
    }, timeoutMs);

    // Immediate check (handles the "already satisfied" case).
    void checkTab();
  });
}

// Guard function to safely add listeners
function guardListener(obj: any, name: string): boolean {
  if (!obj) {
    console.error(`[RZN] ${name} is undefined (permission or context issue)`);
    logError(`${name} is undefined`, { context: 'listener_guard' });
    return false;
  }
  if (!obj.addListener) {
    console.error(`[RZN] ${name}.addListener is undefined (wrong API or context)`);
    logError(`${name}.addListener is undefined`, { context: 'listener_guard' });
    return false;
  }
  return true;
}

// Wait for tab to be complete with timeout
async function waitForTabComplete(tabId: number, timeoutMs = 10000): Promise<void> {
  return new Promise((resolve, reject) => {
    const start = Date.now();
    const onUpdated = (updatedTabId: number, changeInfo: chrome.tabs.TabChangeInfo, tab?: chrome.tabs.Tab) => {
      if (updatedTabId !== tabId) return;
      if (changeInfo.status === 'complete') {
        chrome.tabs.onUpdated.removeListener(onUpdated);
        resolve();
      }
    };
    if (guardListener(chrome.tabs?.onUpdated, 'chrome.tabs.onUpdated')) {
      chrome.tabs.onUpdated.addListener(onUpdated);
    }
    const t = setInterval(async () => {
      if (Date.now() - start > timeoutMs) {
        if (chrome.tabs?.onUpdated?.removeListener) {
          chrome.tabs.onUpdated.removeListener(onUpdated);
        }
        clearInterval(t);
        reject(new Error('waitForTabComplete timeout'));
      } else {
        try {
          const tab = await chrome.tabs.get(tabId);
          if (tab.status === 'complete') {
            if (chrome.tabs?.onUpdated?.removeListener) {
              chrome.tabs.onUpdated.removeListener(onUpdated);
            }
            clearInterval(t);
            resolve();
          }
        } catch {}
      }
    }, 200);
  });
}

function getTabNavigationUrl(tab?: chrome.tabs.Tab | null): string {
  return (tab?.url || tab?.pendingUrl || '').trim();
}

function isHttpNavigableUrl(url: string): boolean {
  return /^https?:\/\//i.test(url);
}

async function waitForTabUrl(
  tabId: number,
  predicate: (url: string) => boolean,
  timeoutMs = 10000,
): Promise<string> {
  return new Promise((resolve, reject) => {
    const start = Date.now();
    let settled = false;
    let t: ReturnType<typeof setInterval>;

    const cleanup = () => {
      if (chrome.tabs?.onUpdated?.removeListener) {
        chrome.tabs.onUpdated.removeListener(onUpdated);
      }
      clearInterval(t);
    };

    const maybeResolve = async (candidateTab?: chrome.tabs.Tab) => {
      if (settled) return;

      try {
        const tab = candidateTab ?? await chrome.tabs.get(tabId);
        const url = getTabNavigationUrl(tab);
        if (url && predicate(url)) {
          settled = true;
          cleanup();
          resolve(url);
          return;
        }
      } catch {}

      if (Date.now() - start > timeoutMs) {
        settled = true;
        cleanup();
        reject(new Error('waitForTabUrl timeout'));
      }
    };

    const onUpdated = (
      updatedTabId: number,
      changeInfo: chrome.tabs.TabChangeInfo,
      tab?: chrome.tabs.Tab,
    ) => {
      if (updatedTabId !== tabId) return;
      if (changeInfo.url || changeInfo.status || getTabNavigationUrl(tab)) {
        void maybeResolve(tab);
      }
    };

    if (guardListener(chrome.tabs?.onUpdated, 'chrome.tabs.onUpdated')) {
      chrome.tabs.onUpdated.addListener(onUpdated);
    }

    t = setInterval(() => {
      void maybeResolve();
    }, 200);

    void maybeResolve();
  });
}

const CONTENT_SCRIPT_PROTOCOL_VERSION = 'rzn-cs-2026-03-17-3';
const CONTENT_SCRIPT_HANDSHAKE_CMD = 'rzn_handshake_v1';
const CONTENT_SCRIPT_EXECUTE_STEP_CMD = 'rzn_execute_step_v1';
const CONTENT_SCRIPT_DOM_SNAPSHOT_CMD = 'rzn_get_dom_snapshot_v1';
const RZN_NOTIFICATION_ICON_DATA_URL = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">
    <rect width="128" height="128" rx="24" fill="#111827"/>
    <rect x="18" y="18" width="92" height="92" rx="18" fill="#f97316"/>
    <path d="M39 92V36h19.8c12.5 0 20.2 6.8 20.2 17.8 0 7.6-4.2 13.1-11.5 15.7L90 92H71.7L52.8 71.6H55V92H39zm16-31.5h3.3c6.1 0 9.7-2.4 9.7-6.9 0-4.4-3.3-6.8-9.5-6.8H55v13.7z" fill="#fff"/>
  </svg>`
)}`;

function withContentScriptCommand(message: any): any {
  if (!message || typeof message !== 'object') {
    return message;
  }

  if (message.cmd === 'execute_step') {
    return { ...message, cmd: CONTENT_SCRIPT_EXECUTE_STEP_CMD };
  }

  if (message.cmd === 'get_dom_snapshot') {
    return { ...message, cmd: CONTENT_SCRIPT_DOM_SNAPSHOT_CMD };
  }

  return message;
}

// Ensure content script is ready with handshake
async function ensureContentReady(tabId: number, injectPath = 'contentScript.js', timeoutMs = 5000): Promise<void> {
  // 1) Ensure URL is http(s)
  const tab = await chrome.tabs.get(tabId);
  const url = tab.url || tab.pendingUrl || '';
  if (!/^https?:\/\//i.test(url)) {
    throw new Error(`Cannot inject on non-http(s) URL: ${url}`);
  }
  const host = new URL(url).hostname;

  // 2) Require a versioned handshake so stale content scripts from a previous
  // extension reload do not count as "ready".
  try {
    const resp = await chrome.tabs.sendMessage(tabId, { cmd: CONTENT_SCRIPT_HANDSHAKE_CMD });
    if (resp?.success && resp?.protocol_version === CONTENT_SCRIPT_PROTOCOL_VERSION) return;
  } catch {}

  // 3) Fallback: inject content script
  try {
    await chrome.scripting.executeScript({ target: { tabId }, files: [injectPath] });
  } catch (e) {
    console.warn('[ensureContentReady] executeScript returned:', e);
  }

  // 4) Ping loop
  const start = Date.now();
  while (true) {
    try {
      const resp = await chrome.tabs.sendMessage(tabId, { cmd: CONTENT_SCRIPT_HANDSHAKE_CMD });
      if (resp?.success && resp?.protocol_version === CONTENT_SCRIPT_PROTOCOL_VERSION) {
        try {
          const leaseActive = (leaseExpirations.get(tabId) ?? 0) > Date.now();
          await withCdpLock(async () => {
            if (leaseActive) {
              if (!frameRouter.isAttachedToTab(tabId)) {
                await frameRouter.attachToTab(tabId);
              }
              await extendCDPLease(tabId);
            } else if (frameRouter.isAttachedToTab(tabId)) {
              // Ensure we don't leave the "Chrome is being controlled" banner around
              // after a previous CDP operation has finished.
              await frameRouter.detachFromTab(tabId);
            }
          });
        } catch (err) {
          console.warn('[ensureContentReady] Failed to reconcile CDP state after ping:', err);
        }
        return;
      }
    } catch (e) {
      // ignore and retry
    }
    if (Date.now() - start > timeoutMs) {
      throw new Error('Content script did not respond to ping');
    }
    await new Promise(r => setTimeout(r, 150));
  }
}

// Send a message specifically to the top-level frame of a tab.
// Falls back to enumerating frames via webNavigation and finally to default routing.
async function sendMessageTopFrame<T = any>(tabId: number, message: any): Promise<T> {
  const normalizedMessage = withContentScriptCommand(message);
  try {
    // Preferred: explicitly target top frame (frameId 0)
    return await chrome.tabs.sendMessage(tabId, normalizedMessage, { frameId: 0 as number } as any) as T;
  } catch (primaryErr) {
    try {
      // Robust fallback: find the top-level frame using webNavigation
      const frames = await chrome.webNavigation.getAllFrames({ tabId });
      const top = frames.find(f => (f as any).parentFrameId === -1) || frames.find(f => f.frameId === 0);
      if (top) {
        return await chrome.tabs.sendMessage(tabId, normalizedMessage, { frameId: top.frameId } as any) as T;
      }
    } catch (enumErr) {
      // Swallow and fallback to default routing below
      console.warn('[sendMessageTopFrame] getAllFrames failed; falling back to default routing', enumErr);
    }
    // Last resort: send without frame hint (may hit an iframe)
    return await chrome.tabs.sendMessage(tabId, normalizedMessage) as T;
  }
}

async function createSystemNotification(title: string, message: string): Promise<string> {
  if (!chrome.notifications?.create) {
    throw new Error('chrome.notifications is unavailable');
  }

  return await new Promise((resolve, reject) => {
    chrome.notifications.create(
      {
        type: 'basic',
        iconUrl: RZN_NOTIFICATION_ICON_DATA_URL,
        title,
        message,
        priority: 2,
      },
      (notificationId) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message));
          return;
        }

        if (notificationId) {
          setTimeout(() => {
            try {
              chrome.notifications.clear(notificationId, () => void chrome.runtime.lastError);
            } catch {}
          }, 30000);
        }

        resolve(notificationId || '');
      }
    );
  });
}

async function sendMessageTopFrameWithRetry<T = any>(
  tabId: number,
  message: any,
  opts?: { injectPath?: string; attempts?: number; timeoutMs?: number }
): Promise<T> {
  const attempts = Math.max(1, opts?.attempts ?? 3);
  const injectPath = opts?.injectPath ?? 'contentScript.js';
  const timeoutMs = opts?.timeoutMs ?? 8000;

  let lastErr: any;
  for (let i = 0; i < attempts; i++) {
    try {
      return await sendMessageTopFrame<T>(tabId, message);
    } catch (e: any) {
      lastErr = e;
      const msg = (e && (e.message || e.toString?.())) ? (e.message || e.toString()) : String(e);
      const retriable =
        msg.includes('Receiving end does not exist') ||
        msg.includes('Could not establish connection') ||
        msg.includes('No tab with id') ||
        msg.includes('The tab was closed');
      if (!retriable || i === attempts - 1) throw e;

      console.debug(
        `[sendMessageTopFrameWithRetry] Retrying after transient error (attempt ${i + 1}/${attempts}): ${msg}`
      );
      await new Promise(r => setTimeout(r, 200));
      await ensureContentReady(tabId, injectPath, timeoutMs).catch(() => {});
    }
  }
  throw lastErr;
}

async function ensureEvalBridgeReady(tabId: number, timeoutMs = 8000): Promise<void> {
  let url = '';
  try {
    const tab = await chrome.tabs.get(tabId);
    url = tab.url || tab.pendingUrl || '';
  } catch {
    return;
  }

  if (!/^https?:\/\//i.test(url)) {
    return;
  }

  await ensureContentReady(tabId, 'contentScript.js', timeoutMs).catch((error) => {
    console.warn('[ensureEvalBridgeReady] content script ensure failed:', error);
  });

  try {
    const [result] = await chrome.scripting.executeScript({
      target: { tabId },
      world: 'MAIN',
      func: () => typeof (window as any).__rznExecuteStep === 'function',
    });
    if (result?.result === true) {
      return;
    }
  } catch (error) {
    console.warn('[ensureEvalBridgeReady] bridge probe failed:', error);
  }

  try {
    await chrome.scripting.executeScript({
      target: { tabId },
      files: ['pageBridge.js'],
      world: 'MAIN',
    });
  } catch (error) {
    console.warn('[ensureEvalBridgeReady] pageBridge inject failed:', error);
  }
}

async function focusTypeTextTarget(tabId: number, selector?: string): Promise<void> {
  const trimmedSelector = typeof selector === 'string' ? selector.trim() : '';
  if (!trimmedSelector) {
    return;
  }

  const [result] = await chrome.scripting.executeScript({
    target: { tabId },
    world: 'MAIN',
    func: (sel: string) => {
      const isVisible = (node: Element): boolean => {
        if (!(node instanceof HTMLElement)) return false;
        const style = window.getComputedStyle(node);
        if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') {
          return false;
        }
        if (node.getAttribute('aria-hidden') === 'true') {
          return false;
        }
        const rect = node.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0;
      };

      const matches = Array.from(document.querySelectorAll(sel));
      const activeElement = document.activeElement;
      const target =
        matches.find((node) => node === activeElement || (!!activeElement && node.contains(activeElement))) ||
        matches.find(isVisible) ||
        matches[0] ||
        null;
      if (!target) {
        return { ok: false, error: `TYPE_TEXT_TARGET_NOT_FOUND: ${sel}` };
      }

      if (target instanceof HTMLElement) {
        try {
          target.scrollIntoView({ block: 'center', behavior: 'instant' as ScrollBehavior });
        } catch {
          target.scrollIntoView();
        }
        target.focus?.();
      }

      if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
        try {
          const end = target.value.length;
          target.setSelectionRange(end, end);
        } catch {}
      }

      return {
        ok: document.activeElement === target || (!!document.activeElement && target.contains(document.activeElement)),
        tag: target.tagName,
        matchCount: matches.length,
      };
    },
    args: [trimmedSelector],
  });

  const focusResult = result?.result as { ok?: boolean; error?: string } | undefined;
  if (focusResult?.ok !== true) {
    throw new Error(focusResult?.error || 'Failed to focus type_text target');
  }
}

async function executeDirectTypeTextStep(tabId: number, step: any): Promise<any> {
  const text = String(step?.text ?? step?.value ?? '');
  if (!text) {
    return {
      success: true,
      action: 'type_text',
      textLength: 0,
      tabId,
      timestamp: Date.now(),
    };
  }

  await focusTypeTextTarget(tabId, step?.selector);
  const { handleTypeText } = await import('./actions/type_text');
  return await runWithAttachedCdpTab(tabId, async () =>
    handleTypeText({ text, tabId, manageDebuggerLifecycle: false })
  );
}

async function resolveMessageTargetTab(
  sender: chrome.runtime.MessageSender,
  sessionId = DEFAULT_WORKFLOW_SESSION_ID
): Promise<number> {
  const senderTabId = sender?.tab?.id;
  if (senderTabId !== undefined) {
    return senderTabId;
  }

  const workflowTabId = getWorkflowTabId(sessionId);
  if (workflowTabId !== undefined) {
    return workflowTabId;
  }

  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (tab?.id === undefined) {
    throw new Error('No target tab available');
  }

  return tab.id;
}

async function ensureAttachedCdpTab(tabId: number): Promise<void> {
  await frameRouter.attachToTab(tabId);
  await extendCDPLease(tabId);
}

async function runWithAttachedCdpTab<T>(tabId: number, run: () => Promise<T>): Promise<T> {
  return await withCdpLock(async () => {
    await ensureAttachedCdpTab(tabId);
    try {
      return await run();
    } finally {
      await forceDetachCDP(tabId).catch(() => {
        console.warn(`Failed to detach CDP from tab ${tabId} after one-shot action`);
      });
    }
  });
}

function isContentScriptDisconnectError(error: unknown): boolean {
  const msg =
    typeof error === 'string'
      ? error
      : (error as any)?.message || (error as any)?.toString?.() || String(error);
  return (
    msg.includes('Receiving end does not exist') ||
    msg.includes('Could not establish connection') ||
    msg.includes('No tab with id') ||
    msg.includes('The tab was closed') ||
    msg.includes('Content script did not respond to ping')
  );
}

function cdpErrorText(error: unknown): string {
  if (typeof error === 'string') return error;
  const anyError = error as any;
  return (
    anyError?.message ||
    anyError?.description ||
    anyError?.toString?.() ||
    String(error)
  );
}

function isExecutionContextDestroyedError(error: unknown): boolean {
  const text = cdpErrorText(error).toLowerCase();
  return (
    text.includes('promise was collected') ||
    text.includes('inspected target navigated or closed') ||
    text.includes('execution context was destroyed') ||
    text.includes('cannot find context with specified id')
  );
}

function executionContextDestroyedError(error: unknown): Error {
  const original = cdpErrorText(error);
  const suggestion =
    'The page likely navigated or re-rendered while the script was awaiting. Add a short wait_for_timeout between the trigger and the next step, or fire-and-return before navigation instead of awaiting after it.';
  const wrapped = new Error(
    `EXEC_CONTEXT_DESTROYED: page execution context died during Runtime.evaluate. ${suggestion} Original CDP error: ${original}`
  );
  (wrapped as any).code = 'EXEC_CONTEXT_DESTROYED';
  (wrapped as any).suggestion = suggestion;
  (wrapped as any).original_error = original;
  return wrapped;
}

function isTruthyWorkflowValue(value: unknown): boolean {
  const text = String(value ?? '').trim().toLowerCase();
  return text === 'true' || text === '1' || text === 'yes' || text === 'y' || text === 'on';
}

function shouldUseCdpEvalForStep(step: any): boolean {
  if (
    step?.use_cdp === true ||
    step?.use_cdp_eval === true ||
    step?.execution_backend === 'cdp' ||
    step?.backend === 'cdp'
  ) {
    return true;
  }

  const conditionalIndex = step?.use_cdp_eval_when_arg_truthy ?? step?.use_cdp_when_arg_truthy;
  if (conditionalIndex === undefined || conditionalIndex === null || conditionalIndex === '') {
    return false;
  }
  const index = Number(conditionalIndex);
  if (!Number.isInteger(index) || index < 0 || !Array.isArray(step?.args)) {
    return false;
  }
  return isTruthyWorkflowValue(step.args[index]);
}

async function injectedRznScriptingEval(
  script: string,
  args: any[],
  params: Record<string, any>,
  returnValue: boolean,
): Promise<any> {
  const serialize = (value: any, depth = 0, seen = new WeakSet<object>()): any => {
    if (value == null || typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
      return value;
    }
    if (typeof value === 'bigint') return value.toString();
    if (typeof value === 'function') return `[Function ${value.name || 'anonymous'}]`;
    if (typeof Element !== 'undefined' && value instanceof Element) {
      const rect = value instanceof HTMLElement ? value.getBoundingClientRect() : null;
      return {
        tag: value.tagName.toLowerCase(),
        text: (value.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 160),
        rect: rect
          ? { x: rect.x, y: rect.y, width: rect.width, height: rect.height }
          : null,
      };
    }
    if (value instanceof Error) {
      return { name: value.name, message: value.message, stack: value.stack };
    }
    if (depth >= 4) {
      if (Array.isArray(value)) return `[Array(${value.length})]`;
      return '[Object]';
    }
    if (Array.isArray(value)) {
      return value.slice(0, 50).map((item) => serialize(item, depth + 1, seen));
    }
    if (typeof value === 'object') {
      if (seen.has(value)) return '[Circular]';
      seen.add(value);
      const out: Record<string, any> = {};
      for (const [key, child] of Object.entries(value).slice(0, 50)) {
        out[key] = serialize(child, depth + 1, seen);
      }
      return out;
    }
    return String(value);
  };

  const source = String(script || '');
  const trimmed = source.trim();
  const startsWithReturn = /^return\b/.test(trimmed);
  const expressionLike =
    trimmed.startsWith('(') ||
    trimmed.startsWith('[') ||
    trimmed.startsWith('{') ||
    /^async\s*\(/.test(trimmed) ||
    /^function\b/.test(trimmed);
  const statementLike =
    startsWithReturn ||
    /(^|[\s;])(?:const|let|var|if|for|while|throw|try|await)\b/.test(trimmed) ||
    trimmed.includes('\n') ||
    (trimmed.includes(';') && !expressionLike);
  const body = statementLike ? source : `return (${source});`;
  const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor as FunctionConstructor;
  const run = new AsyncFunction(
    'args',
    'params',
    `
    "use strict";
    const __args = Array.isArray(args) ? args : [];
    const __rzn_params = params && typeof params === 'object' ? params : {};
    const arg0 = __args[0];
    const arg1 = __args[1];
    const arg2 = __args[2];
    const __rznGlobal = globalThis;
    const __rznNamespace =
      __rznGlobal.__rzn && typeof __rznGlobal.__rzn === 'object'
        ? __rznGlobal.__rzn
        : {};
    const __rznWalkDeep = (root, visit) => {
      if (!root) return;
      let stop = visit(root);
      if (stop) return stop;
      const nodes = root.querySelectorAll ? Array.from(root.querySelectorAll('*')) : [];
      for (const node of nodes) {
        stop = visit(node);
        if (stop) return stop;
        if (node && node.shadowRoot) {
          stop = __rznWalkDeep(node.shadowRoot, visit);
          if (stop) return stop;
        }
      }
      return undefined;
    };
    __rznNamespace.qsDeep = (selector, root = document) => {
      let found = null;
      __rznWalkDeep(root, (current) => {
        if (!current || !current.querySelector) return false;
        try {
          const el = current.querySelector(selector);
          if (el) {
            found = el;
            return true;
          }
        } catch {}
        return false;
      });
      return found;
    };
    __rznNamespace.qsAllDeep = (selector, root = document) => {
      const found = [];
      const seen = new Set();
      __rznWalkDeep(root, (current) => {
        if (!current || !current.querySelectorAll) return false;
        try {
          for (const el of Array.from(current.querySelectorAll(selector))) {
            if (!seen.has(el)) {
              seen.add(el);
              found.push(el);
            }
          }
        } catch {}
        return false;
      });
      return found;
    };
    const __previousRznParams = __rznGlobal.__rzn_params;
    __rznGlobal.__rzn = __rznNamespace;
    __rznGlobal.__rzn_params = __rzn_params;
    try {
      return await (async () => {
        ${body}
      })();
    } finally {
      if (typeof __previousRznParams === 'undefined') {
        try { delete __rznGlobal.__rzn_params; } catch {}
      } else {
        __rznGlobal.__rzn_params = __previousRznParams;
      }
    }
  `
  );
  const value = await run(args, params);
  return returnValue === false ? null : serialize(value);
}

async function runScriptingEval(
  sender: chrome.runtime.MessageSender | undefined,
  message: any,
): Promise<{
  success: true;
  execution_backend: string;
  requested_world: string;
  result: any;
  tabId: number;
}> {
  const explicitTabId = typeof message?.tabId === 'number' ? message.tabId : undefined;
  const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
  const tabId = explicitTabId ?? sender?.tab?.id ?? tabs[0]?.id;
  if (tabId === undefined) {
    throw new Error('No target tab available for script eval');
  }

  const script = String(message?.script || '');
  const args = Array.isArray(message?.args) ? message.args : [];
  const params =
    message?.params && typeof message.params === 'object' && !Array.isArray(message.params)
      ? message.params
      : {};
  const requestedWorld = String(message?.world || 'main').toLowerCase();
  // Arbitrary string eval in MV3 isolated worlds runs into extension CSP because it
  // requires Function/AsyncFunction inside the injected function. Keep JS-first by
  // using MAIN world for string eval; callers that truly need isolated CDP semantics
  // can opt into runCdpEval explicitly.
  const world = 'MAIN';
  const wantsReturn = message?.return_value !== false;

  try {
    const [result] = await chrome.scripting.executeScript({
      target: { tabId },
      world,
      func: injectedRznScriptingEval,
      args: [script, args, params, wantsReturn],
    } as chrome.scripting.ScriptInjection<any[], any>);

    return {
      success: true,
      execution_backend:
        requestedWorld === 'isolated'
          ? 'chrome_scripting_main_world_for_isolated_eval'
          : 'chrome_scripting_main_world',
      requested_world: requestedWorld,
      result: result?.result ?? null,
      tabId,
    };
  } catch (error) {
    if (isExecutionContextDestroyedError(error)) {
      throw executionContextDestroyedError(error);
    }
    throw error;
  }
}

async function runCdpEval(
  sender: chrome.runtime.MessageSender | undefined,
  message: any,
): Promise<{
  success: true;
  execution_backend: string;
  requested_world: string;
  result: any;
  tabId: number;
}> {
  const explicitTabId = typeof message?.tabId === 'number' ? message.tabId : undefined;
  const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
  const tabId = explicitTabId ?? sender?.tab?.id ?? tabs[0]?.id;
  if (tabId === undefined) {
    throw new Error('No target tab available for CDP eval');
  }

  await ensureEvalBridgeReady(tabId);

  await withCdpLock(async () => {
    await frameRouter.attachToTab(tabId);
    setCdpLeaseExpiration(tabId, Date.now() + 15_000);
  });

  const { cdpClient } = await import('./cdp/cdpClient');
  const sessions = frameRouter.getFrameSessionsForTab(tabId);
  const sessionId = sessions[0]?.sessionId;
  if (!sessionId) {
    throw new Error(`No CDP session available for tab ${tabId}`);
  }

  const script = String(message?.script || '');
  const args = Array.isArray(message?.args) ? message.args : [];
  const params =
    message?.params && typeof message.params === 'object' && !Array.isArray(message.params)
      ? message.params
      : {};
  const wantsReturn = message?.return_value !== false;
  const trimmed = script.trim();
  const startsWithReturn = /^return\b/.test(trimmed);
  const expressionLike =
    trimmed.startsWith('(') ||
    trimmed.startsWith('[') ||
    trimmed.startsWith('{') ||
    /^async\s*\(/.test(trimmed) ||
    /^function\b/.test(trimmed);
  const statementLike =
    startsWithReturn ||
    /(^|[\s;])(?:const|let|var|if|for|while|throw|try|await)\b/.test(trimmed) ||
    trimmed.includes('\n') ||
    (trimmed.includes(';') && !expressionLike);
  const body = statementLike ? script : `return (${script});`;
  const expression = `(async () => {
    const __args = ${JSON.stringify(args)};
    const __rzn_params = ${JSON.stringify(params)};
    const arg0 = __args[0];
    const arg1 = __args[1];
    const arg2 = __args[2];
    const __previousRznParams = globalThis.__rzn_params;
    const __rznGlobal = globalThis;
    const __rznNamespace =
      __rznGlobal.__rzn && typeof __rznGlobal.__rzn === 'object'
        ? __rznGlobal.__rzn
        : {};
    const __rznWalkDeep = (root, visit) => {
      if (!root) return;
      let stop = visit(root);
      if (stop) return stop;
      const nodes = root.querySelectorAll ? Array.from(root.querySelectorAll('*')) : [];
      for (const node of nodes) {
        stop = visit(node);
        if (stop) return stop;
        if (node && node.shadowRoot) {
          stop = __rznWalkDeep(node.shadowRoot, visit);
          if (stop) return stop;
        }
      }
      return undefined;
    };
    __rznNamespace.qsDeep = (selector, root = document) => {
      let found = null;
      __rznWalkDeep(root, (current) => {
        if (!current || !current.querySelector) return false;
        try {
          const el = current.querySelector(selector);
          if (el) {
            found = el;
            return true;
          }
        } catch {}
        return false;
      });
      return found;
    };
    __rznNamespace.qsAllDeep = (selector, root = document) => {
      const found = [];
      const seen = new Set();
      __rznWalkDeep(root, (current) => {
        if (!current || !current.querySelectorAll) return false;
        try {
          for (const el of Array.from(current.querySelectorAll(selector))) {
            if (!seen.has(el)) {
              seen.add(el);
              found.push(el);
            }
          }
        } catch {}
        return false;
      });
      return found;
    };
    __rznGlobal.__rzn = __rznNamespace;
    globalThis.__rzn_params = __rzn_params;
    if (typeof window !== 'undefined') {
      window.__rzn = __rznNamespace;
      window.__rzn_params = __rzn_params;
    }
    try {
      ${body}
    } finally {
      if (typeof __previousRznParams === 'undefined') {
        try { delete globalThis.__rzn_params; } catch {}
        if (typeof window !== 'undefined') {
          try { delete window.__rzn_params; } catch {}
        }
      } else {
        globalThis.__rzn_params = __previousRznParams;
        if (typeof window !== 'undefined') {
          window.__rzn_params = __previousRznParams;
        }
      }
    }
  })()`;

  let evalResult: any;
  try {
    evalResult = await withCdpLock(async () => {
      return await cdpClient.evaluate(
        { tabId, sessionId },
        expression,
        {
          awaitPromise: true,
          returnByValue: true,
          userGesture: true,
          allowUnsafeEvalBlockedByCSP: true,
        }
      );
    });
  } catch (error) {
    if (isExecutionContextDestroyedError(error)) {
      throw executionContextDestroyedError(error);
    }
    throw error;
  } finally {
    await forceDetachCDP(tabId).catch(() => {
      console.warn(`Failed to detach CDP from tab ${tabId} after eval`);
    });
  }

  const errorDescription =
    evalResult?.exceptionDetails?.exception?.description ||
    evalResult?.exceptionDetails?.text;
  if (errorDescription) {
    if (isExecutionContextDestroyedError(errorDescription)) {
      throw executionContextDestroyedError(errorDescription);
    }
    throw new Error(String(errorDescription));
  }

  return {
    success: true,
    execution_backend:
      message?.world === 'isolated'
        ? 'cdp_main_world_fallback'
        : 'cdp_runtime_evaluate',
    requested_world: message?.world || 'main',
    result: wantsReturn
      ? (evalResult?.result?.value ?? evalResult?.result?.unserializableValue ?? null)
      : null,
    tabId,
  };
}

function normalizeScreenshotFormat(format: any): 'png' | 'jpeg' {
  const raw = typeof format === 'string' ? format.toLowerCase().trim() : '';
  if (raw === 'jpg' || raw === 'jpeg') return 'jpeg';
  return 'png';
}

function clampScreenshotQuality(quality: any): number | undefined {
  const n = Number(quality);
  if (!Number.isFinite(n)) return undefined;
  return Math.max(0, Math.min(100, Math.round(n)));
}

type ScreenshotAnnotationRect = {
  idx: number;
  ref: string;
  x: number;
  y: number;
  width: number;
  height: number;
};

function clampAnnotateMaxLabels(max: any): number {
  const n = Number(max);
  if (!Number.isFinite(n)) return 30;
  return Math.max(1, Math.min(200, Math.round(n)));
}

function clampAnnotateMaxElements(max: any): number {
  const n = Number(max);
  if (!Number.isFinite(n)) return 80;
  return Math.max(1, Math.min(200, Math.round(n)));
}

function arrayBufferToBase64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  const chunk = 0x8000;
  let binary = '';
  for (let i = 0; i < bytes.length; i += chunk) {
    const sub = bytes.subarray(i, i + chunk);
    let s = '';
    for (let j = 0; j < sub.length; j++) {
      s += String.fromCharCode(sub[j]);
    }
    binary += s;
  }
  return btoa(binary);
}

async function blobToDataUrl(blob: Blob, mimeType: string): Promise<string> {
  const buf = await blob.arrayBuffer();
  const b64 = arrayBufferToBase64(buf);
  return `data:${mimeType};base64,${b64}`;
}

function collectAnnotationRectsFromSnapshot(domSnapshot: any, maxLabels: number): {
  rects: ScreenshotAnnotationRect[];
  viewport?: { width: number; height: number };
} {
  const elements: any[] = domSnapshot?.elements || [];
  const viewport = domSnapshot?.metadata?.viewport;
  const rects: ScreenshotAnnotationRect[] = [];

  for (let idx = 0; idx < elements.length; idx++) {
    if (rects.length >= maxLabels) break;
    const el = elements[idx];
    const s = el?.spatial_info;
    if (!s) continue;
    const x = Number(s.x);
    const y = Number(s.y);
    const width = Number(s.width);
    const height = Number(s.height);
    if (![x, y, width, height].every(Number.isFinite)) continue;
    if (width <= 0 || height <= 0) continue;
    rects.push({ idx, ref: `@e${idx + 1}`, x, y, width, height });
  }

  return { rects, viewport };
}

async function annotateScreenshotDataUrl(
  dataUrl: string,
  rects: ScreenshotAnnotationRect[],
  viewport: { width: number; height: number } | undefined,
  outputFormat: 'png' | 'jpeg',
  quality?: number,
): Promise<string> {
  const offscreenOk = typeof (globalThis as any).OffscreenCanvas === 'function';
  const createBitmapOk = typeof (globalThis as any).createImageBitmap === 'function';
  if (!offscreenOk || !createBitmapOk) {
    return dataUrl;
  }

  const res = await fetch(dataUrl);
  const blob = await res.blob();
  const bitmap = await createImageBitmap(blob);

  const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
  const ctx = canvas.getContext('2d');
  if (!ctx) return dataUrl;

  ctx.drawImage(bitmap, 0, 0);

  const vw = viewport?.width ? Number(viewport.width) : undefined;
  const vh = viewport?.height ? Number(viewport.height) : undefined;
  const scaleX = vw && vw > 0 ? bitmap.width / vw : 1;
  const scaleY = vh && vh > 0 ? bitmap.height / vh : 1;

  const fontPx = Math.max(12, Math.round(12 * Math.max(scaleX, scaleY)));
  const lineWidth = Math.max(2, Math.round(2 * Math.max(scaleX, scaleY)));

  ctx.lineWidth = lineWidth;
  ctx.strokeStyle = 'rgba(0, 153, 255, 0.95)';
  ctx.fillStyle = 'rgba(0, 153, 255, 0.95)';
  ctx.font = `${fontPx}px ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, Arial`;
  ctx.textBaseline = 'top';

  for (const r of rects) {
    const x = Math.max(0, Math.round(r.x * scaleX));
    const y = Math.max(0, Math.round(r.y * scaleY));
    const w = Math.max(1, Math.round(r.width * scaleX));
    const h = Math.max(1, Math.round(r.height * scaleY));

    // Clamp within the image.
    const x2 = Math.min(bitmap.width - 1, x);
    const y2 = Math.min(bitmap.height - 1, y);
    const w2 = Math.min(bitmap.width - x2 - 1, w);
    const h2 = Math.min(bitmap.height - y2 - 1, h);

    ctx.strokeRect(x2, y2, w2, h2);

    const label = r.ref;
    const metrics = ctx.measureText(label);
    const padX = Math.max(4, Math.round(4 * scaleX));
    const padY = Math.max(2, Math.round(2 * scaleY));
    const boxW = Math.round(metrics.width + padX * 2);
    const boxH = Math.round(fontPx + padY * 2);

    // Slightly above the rect when possible, else inside.
    const lx = x2;
    const ly = y2 - boxH >= 0 ? y2 - boxH : y2;

    ctx.fillStyle = 'rgba(0, 0, 0, 0.65)';
    ctx.fillRect(lx, ly, boxW, boxH);
    ctx.fillStyle = 'rgba(255, 255, 255, 0.98)';
    ctx.fillText(label, lx + padX, ly + padY);
    ctx.fillStyle = 'rgba(0, 153, 255, 0.95)';
  }

  const mime = outputFormat === 'jpeg' ? 'image/jpeg' : 'image/png';
  const outQuality = (() => {
    const q = clampScreenshotQuality(quality);
    if (q === undefined) return undefined;
    return Math.max(0, Math.min(1, q / 100));
  })();

  const outBlob = await canvas.convertToBlob({
    type: mime,
    quality: mime === 'image/jpeg' ? outQuality : undefined,
  } as any);

  return await blobToDataUrl(outBlob, mime);
}

async function captureScreenshotForTab(
  tabId: number,
  opts?: { format?: any; quality?: any }
): Promise<string> {
  const tab = await chrome.tabs.get(tabId);
  const windowId = tab.windowId ?? chrome.windows.WINDOW_ID_CURRENT;

  // captureVisibleTab only captures the active tab in a window.
  try {
    await chrome.tabs.update(tabId, { active: true });
  } catch {}

  await new Promise(r => setTimeout(r, 250));

  const format = normalizeScreenshotFormat(opts?.format);
  const captureOpts: any = { format };

  const quality = clampScreenshotQuality(opts?.quality);
  if (format === 'jpeg' && quality !== undefined) {
    captureOpts.quality = quality;
  }

  return await chrome.tabs.captureVisibleTab(windowId, captureOpts);
}

// Connect to native messaging host
function connectToNative(): void {
  const connectNative = chrome.runtime?.connectNative;
  if (typeof connectNative !== 'function') {
    const manifest = chrome.runtime?.getManifest?.();
    const perms = (manifest as any)?.permissions ?? [];
    logWarn('Native messaging not available in this context', {
      reason: 'connectNative_undefined',
      extension_id: chrome.runtime?.id,
      has_native_messaging_permission: Array.isArray(perms) ? perms.includes('nativeMessaging') : false,
      manifest_version: (manifest as any)?.manifest_version,
      browser_ua: typeof navigator !== 'undefined' ? navigator.userAgent : undefined,
      note: 'Some Chromium-based browsers disable native messaging; try Chrome stable if this persists.',
    });
    return;
  }

  // Avoid spawning multiple native host processes (each connectNative() spawns a new host).
  // MV3 service worker lifecycle + multiple startup hooks can otherwise race and collide on
  // broker IPC sockets (e.g. /tmp/rzn.sock "address in use").
  if (nativePort) return;
  if (nativeConnectInFlight) return;
  nativeConnectInFlight = true;

  try {
    const host = BROKER_HOST_CANDIDATES[Math.min(brokerHostIndex, BROKER_HOST_CANDIDATES.length - 1)];
    brokerHostInUse = host;
    console.log('Connecting to native host:', host);
    logInfo('Connecting to native host');
    nativePort = connectNative(host);

    // Set the native port for the logger
    setNativePort(nativePort);

	    if (guardListener(nativePort?.onMessage, 'nativePort.onMessage')) {
	      nativePort.onMessage.addListener((message: BrokerMessage) => {
	        console.log('Received from broker:', message);
	        logInfo('Received message from broker', { 
	          cmd: message.cmd || message.action, 
	          req_id: message.req_id || message.task_id,
	          hasTask: !!message.task,
	          taskKeys: message.task ? Object.keys(message.task) : []
	        });
	        if (maybeResolveNativeControlCallback(message)) {
	          return;
	        }
	        const sessionId = resolveSessionId(message);
	        const state = getWorkflowSessionState(sessionId);
	        state.queue = state.queue
	          .then(() => handleBrokerMessage(message, sessionId))
	          .catch(err => {
	            console.error('[RZN] broker message handler error:', err);
	          });
	      });
	    }

    if (guardListener(nativePort?.onDisconnect, 'nativePort.onDisconnect')) {
      nativePort.onDisconnect.addListener(() => {
        const err = chrome.runtime.lastError?.message;
        console.log('Native host disconnected:', err);
        logWarn('Native host disconnected', { error: err, host: brokerHostInUse });
        rejectAllNativeControlCallbacks(err || 'Native host disconnected');
        setNativePort(null);  // Clear the logger's reference too
        nativePort = null;
        nativeConnectInFlight = false;
        clearHeartbeat();
        // If the preferred host isn't installed, immediately try the fallback host.
        if (
          err &&
          brokerHostIndex < BROKER_HOST_CANDIDATES.length - 1 &&
          (err.toLowerCase().includes('host not found') || err.toLowerCase().includes('not found'))
        ) {
          brokerHostIndex++;
          reconnectAttempts = 0;
          clearReconnectTimer();
          connectToNative();
          return;
        }
        // Attempt reconnection with backoff
        scheduleReconnect();
      });
    }

    console.log('Connected to native host successfully');
    logInfo('Connected to native host successfully');
    // Reset backoff and start heartbeat on successful connect
    reconnectAttempts = 0;
    clearReconnectTimer();
    nativeConnectInFlight = false;
    startHeartbeat();
  } catch (error) {
    console.error('Failed to connect to native host:', error);
    logError('Failed to connect to native host', { error: error });
    nativePort = null;
    nativeConnectInFlight = false;
    // Schedule reconnect if initial connect fails
    scheduleReconnect();
  }
}

// Handle delta DOM messages for efficient updates
function handleDeltaMessage(message: BrokerMessage): void {
  if (message.action === 'dom_delta' || message.cmd === 'apply_dom_delta') {
    console.log('Received DOM delta message:', message);
    // Delta messages are informational and don't require immediate action
    // They are stored and passed through to the orchestrator
    if (nativePort) {
      const deltaResponse = {
        action: 'delta_received',
        task_id: message.task_id || message.req_id,
        success: true,
        dom_hash: message.dom_hash,
        timestamp: Date.now()
      };
      nativePort.postMessage(deltaResponse);
    }
  }
}

// Handle messages from broker
async function handleBrokerMessage(message: BrokerMessage, sessionId?: string): Promise<void> {
  await loadWorkflowSessionsFromStorage();
  const workflowSessionId = normalizeSessionId(sessionId || resolveSessionId(message));
  let workflowTabId = getWorkflowTabId(workflowSessionId);
  const requestedTabId = resolveRequestedTabId(message);
  if (requestedTabId !== undefined) {
    workflowTabId = requestedTabId;
    setWorkflowTabId(workflowSessionId, requestedTabId);
  }
  const setSessionWorkflowTabId = (tabId: number | undefined) => {
    workflowTabId = tabId;
    setWorkflowTabId(workflowSessionId, tabId);
  };

  // Support both message formats
  const command = message.cmd || (message.action === 'perform_task' ? 'execute_workflow' : message.action);
  const requestId = message.req_id || message.task_id || 'unknown';
  const isOrchestratorFormat = !!message.task_id || !!message.action;

  console.log('Handling broker message:', command, 'req_id:', requestId, 'orchestrator format:', isOrchestratorFormat);
  console.log('Message has task:', !!message.task);
  console.log('Full message:', JSON.stringify(message).substring(0, 500));

  // Handle delta messages
  handleDeltaMessage(message);

  // Handle native_input_response messages
  if (command === 'native_input_response') {
    console.log('Received native_input_response:', message);
    const callback = nativeInputCallbacks.get(requestId);
    if (callback) {
      clearTimeout(callback.timeoutId);
      callback.sendResponse(message.payload || { ok: false, error: 'No payload' });
      nativeInputCallbacks.delete(requestId);
    }
    return;
  }

  if (command === 'ping') {
    sendResponseToBroker({
      req_id: isOrchestratorFormat ? undefined : requestId,
      task_id: isOrchestratorFormat ? requestId : undefined,
      success: true,
      result: {
        pong: true,
        source: message.payload?.source || 'extension_background',
        current_tab_id: workflowTabId,
      },
    });
    return;
  }

  // Break-glass: explicit, time-bounded CDP enablement (no CDP by default)
  if (message.cmd === 'enable_debug') {
    try {
      const mode = (message.payload?.mode || 'enrichment').toString();
      const ttlMs = Math.max(1000, Number(message.payload?.ttl_ms ?? message.payload?.ttlMs ?? 120000));

      let tabId: number | undefined;
      if (workflowTabId !== undefined) {
        try {
          await chrome.tabs.get(workflowTabId);
          tabId = workflowTabId;
        } catch {
          setSessionWorkflowTabId(undefined);
        }
      }
      if (tabId === undefined) {
        const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
        if (!tabs.length || tabs[0].id === undefined) {
          sendResponseToBroker({
            req_id: requestId,
            success: false,
            error_code: 'NO_ACTIVE_TAB',
            error_msg: 'No active tab found for enable_debug',
          });
          return;
        }
        tabId = tabs[0].id!;
      }

      const tab = await chrome.tabs.get(tabId);
      const url = tab.url || tab.pendingUrl || '';
      if (!/^https?:\/\//i.test(url)) {
        sendResponseToBroker({
          req_id: requestId,
          success: false,
          error_code: 'RESTRICTED_URL',
          error_msg: `Cannot enable CDP on restricted URL: ${url || 'unknown'}`,
        });
        return;
      }

      await withCdpLock(async () => {
        await frameRouter.attachToTab(tabId);
        setCdpLeaseExpiration(tabId, Date.now() + ttlMs);
      });

      sendResponseToBroker({
        req_id: requestId,
        success: true,
        result: { mode, ttl_ms: ttlMs, tab_id: tabId, expires_at_ms: leaseExpirations.get(tabId) },
        capabilities: await buildCapabilities(tabId),
      } as any);
      return;
    } catch (error) {
      console.error('enable_debug error:', error);
      sendResponseToBroker({
        req_id: requestId,
        success: false,
        error_code: 'ENABLE_DEBUG_ERROR',
        error_msg: (error as Error).message,
      });
      return;
    }
  }

  if (message.cmd === 'disable_debug') {
    try {
      let tabId: number | undefined;
      if (workflowTabId !== undefined) {
        tabId = workflowTabId;
      } else {
        const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
        if (tabs.length === 0 || tabs[0].id === undefined) {
          sendResponseToBroker({
            req_id: requestId,
            success: false,
            error_code: 'NO_ACTIVE_TAB',
            error_msg: 'No active tab found for disable_debug',
          });
          return;
        }
        tabId = tabs[0].id!;
      }

      await withCdpLock(async () => {
        await forceDetachCDP(tabId);
      });
      sendResponseToBroker({
        req_id: requestId,
        success: true,
        result: { tab_id: tabId, cdp_attached: false },
        capabilities: await buildCapabilities(tabId),
      } as any);
      return;
    } catch (error) {
      console.error('disable_debug error:', error);
      sendResponseToBroker({
        req_id: requestId,
        success: false,
        error_code: 'DISABLE_DEBUG_ERROR',
        error_msg: (error as Error).message,
      });
      return;
    }
  }

  // Map orchestrator actions to extension commands
  if (command === 'execute_workflow' && message.task) {
    // Handle workflow execution from orchestrator
    logInfo('Routing to executeWorkflow', { hasTask: true });
    await executeWorkflow(message, requestId, isOrchestratorFormat, workflowSessionId);
  } else if (command === 'execute_workflow' && !message.task) {
    // Log why we're not executing
    logError('execute_workflow command received but no task provided', {
      command,
      hasTask: false,
      messageKeys: Object.keys(message)
    });
    sendResponseToBroker({
      req_id: isOrchestratorFormat ? undefined : requestId,
      task_id: isOrchestratorFormat ? requestId : undefined,
      success: false,
      error_code: 'NO_TASK',
      error_msg: 'execute_workflow command received but no task object in message'
    });
  } else if (
    (command === 'execute_step' || (command === 'execute_workflow' && message.payload?.step)) &&
    message.payload?.step?.type === 'get_page_source'
  ) {
    try {
      const tabId =
        workflowTabId ??
        (sessionMayUseActiveTab(workflowSessionId)
          ? await getActiveTabIdOrThrow('get_page_source')
          : undefined);
      if (tabId === undefined) {
        sendResponseToBroker({
          req_id: isOrchestratorFormat ? undefined : requestId,
          task_id: isOrchestratorFormat ? requestId : undefined,
          success: false,
          error_code: 'NO_WORKFLOW_TAB',
          error_msg: buildMissingWorkflowTabError(workflowSessionId, 'get_page_source')
        });
        return;
      }
      const [result] = await chrome.scripting.executeScript({
        target: { tabId },
        func: () => document.documentElement.outerHTML
      });

      sendResponseToBroker({
        req_id: isOrchestratorFormat ? undefined : requestId,
        task_id: isOrchestratorFormat ? requestId : undefined,
        success: true,
        result: {
          type: 'page_source',
          html: result.result
        }
      });
      return;
    } catch (error) {
      console.error('Error getting page source:', error);
      sendResponseToBroker({
        req_id: isOrchestratorFormat ? undefined : requestId,
        task_id: isOrchestratorFormat ? requestId : undefined,
        success: false,
        error_code: 'PAGE_SOURCE_ERROR',
        error_msg: (error as Error).message
      });
      return;
    }
  } else if (command === 'eval_with_cdp') {
    try {
      let tabId: number;
      const payload = message.payload || {};
      const mayUseCurrentTab =
        sessionMayUseActiveTab(workflowSessionId) ||
        payload.use_current_tab === true ||
        payload.use_active_tab === true;

      if (workflowTabId !== undefined) {
        try {
          const tab = await chrome.tabs.get(workflowTabId);
          if (tab) {
            tabId = workflowTabId;
          } else {
            setSessionWorkflowTabId(undefined);
          }
        } catch {
          setSessionWorkflowTabId(undefined);
        }
      }

      if (workflowTabId === undefined) {
        if (!mayUseCurrentTab) {
          throw new Error(buildMissingWorkflowTabError(workflowSessionId, 'eval_with_cdp'));
        }
        tabId = await getActiveTabIdOrThrow('eval_with_cdp');
        setSessionWorkflowTabId(tabId);
        await waitForTabComplete(tabId, 15000).catch(() => {});
      } else {
        tabId = workflowTabId;
      }

      const evalResponse = await runCdpEval(undefined, {
        script: payload.script,
        args: payload.args,
        params: payload.params,
        return_value: payload.return_value,
        world: payload.world,
        timeout_ms: payload.timeout_ms,
        tabId,
      });
      const tab = await chrome.tabs.get(tabId);
      sendResponseToBroker({
        req_id: isOrchestratorFormat ? undefined : requestId,
        task_id: isOrchestratorFormat ? requestId : undefined,
        success: true,
        result: {
          success: true,
          world: evalResponse.requested_world,
          execution_backend: evalResponse.execution_backend,
          result: evalResponse.result,
        },
        current_url: tab.url || '',
        current_tab_id: tabId,
      } as any);
      return;
    } catch (error: any) {
      sendResponseToBroker({
        req_id: isOrchestratorFormat ? undefined : requestId,
        task_id: isOrchestratorFormat ? requestId : undefined,
        success: false,
        error_code: 'EVAL_ERROR',
        error_msg: error?.message || String(error)
      } as any);
      return;
    }
  } else if (command === 'execute_step' || (command === 'execute_workflow' && message.payload?.step)) {
    // Forward to content script - create tab if needed
    try {
      let tabId: number | undefined;
      const step = message.payload?.step;
      const stepType = typeof step?.type === 'string' ? step.type : '';
      const requiresExistingTab =
        stepType !== 'open_new_tab' &&
        stepType !== 'switch_to_tab' &&
        stepType !== 'close_current_tab';
      const prefersCurrentTab =
        message.payload?.use_current_tab === true ||
        message.payload?.use_active_tab === true ||
        step?.use_current_tab === true ||
        step?.use_active_tab === true;
      const mayUseCurrentTab = sessionMayUseActiveTab(workflowSessionId, prefersCurrentTab);

      // First check if we have a stored workflow tab
      if (workflowTabId !== undefined && requiresExistingTab) {
        try {
          const tab = await chrome.tabs.get(workflowTabId);
          if (tab) {
            tabId = workflowTabId;
            console.log(`Using existing workflow tab ID: ${tabId}`);
          } else {
            setSessionWorkflowTabId(undefined); // Tab no longer exists
          }
        } catch (e) {
          setSessionWorkflowTabId(undefined); // Tab no longer exists
        }
      }

      // If no workflow tab, either bind to the visible active tab or create a dedicated one.
      if (workflowTabId === undefined && requiresExistingTab) {
        if (mayUseCurrentTab) {
          console.log('Binding execute_step session to current active tab');
          const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
          const activeTabId = tabs[0]?.id;
          if (activeTabId === undefined) {
            throw new Error('No active tab available for use_current_tab execute_step');
          }
          tabId = activeTabId;
          setSessionWorkflowTabId(tabId);
          await waitForTabComplete(tabId, 15000).catch(() => {});
          await ensureContentReady(tabId, 'contentScript.js', 8000).catch(() => {});
        } else {
          console.log('Creating dedicated workflow tab');
          const newTab = await chrome.tabs.create({ url: 'https://www.example.com', active: true });
          tabId = newTab.id!;
          setSessionWorkflowTabId(tabId);
          await waitForTabComplete(tabId, 15000).catch(() => {});
          await ensureContentReady(tabId, 'contentScript.js', 8000).catch(() => {});
        }
      } else {
        tabId = workflowTabId;
      }

      if (step?.type === 'open_new_tab') {
        try {
          const url =
            step.url && typeof step.url === 'string' && step.url.trim() !== ''
              ? step.url
              : 'about:blank';
          const shouldPrepareContent = isHttpNavigableUrl(url);

          const newTab = await chrome.tabs.create({ url, active: true });
          tabId = newTab.id!;
          setSessionWorkflowTabId(tabId);

          if (shouldPrepareContent) {
            await waitForTabUrl(tabId, candidateUrl => isHttpNavigableUrl(candidateUrl), 15000).catch(() => {});
            const openedTab = await chrome.tabs.get(tabId).catch(() => undefined);
            const openedUrl = getTabNavigationUrl(openedTab);
            if (isHttpNavigableUrl(openedUrl)) {
              await waitForTabComplete(tabId, 15000).catch(() => {});
              await ensureContentReady(tabId, 'contentScript.js', 8000).catch(() => {});
            }
          }

          const openedTab = await chrome.tabs.get(tabId);
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: true,
            result: { tabId, opened: true, url: openedTab.url || openedTab.pendingUrl || url },
            current_url: openedTab.url || openedTab.pendingUrl || url,
            current_tab_id: tabId,
          } as any);
          return;
        } catch (error: any) {
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'TAB_CREATE_ERROR',
            error_msg: error?.message || String(error),
          } as any);
          return;
        }
      }

      // Background-level navigation for `execute_step` to ensure we actually wait for page load.
      // The content-script `navigate_to_url` handler sets `window.location.href` and returns immediately,
      // which can cause follow-up steps (detect/extract) to run against the previous page.
      if (step?.type === 'navigate_to_url') {
        try {
          const url = (step.url && typeof step.url === 'string') ? step.url.trim() : '';
          if (!url) throw new Error('Missing URL for navigation');

          await chrome.tabs.update(tabId, { url });
          await waitForNavigation(tabId, step.wait, step.timeout_ms || step.timeoutMs || 15000);
          await ensureContentReady(tabId, 'contentScript.js', 8000).catch(() => {});

          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: true,
            result: true,
            current_url: url
          } as any);
          return;
        } catch (error: any) {
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'NAVIGATION_ERROR',
            error_msg: error?.message || String(error)
          } as any);
          return;
        }
      }

      if (step?.type === 'get_current_url') {
        try {
          const tab = await chrome.tabs.get(tabId);
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: true,
            result: { url: tab.url || '' }
          } as any);
          return;
        } catch (error: any) {
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'GET_URL_ERROR',
            error_msg: error?.message || String(error)
          } as any);
          return;
        }
      }

      if (
        step?.type === 'execute_javascript' ||
        step?.type === 'eval_main_world' ||
        step?.type === 'eval_isolated_world'
      ) {
        try {
          const wantsCdpEval = shouldUseCdpEvalForStep(step);
          const evalRunner = wantsCdpEval ? runCdpEval : runScriptingEval;
          const evalResponse = await evalRunner(undefined, {
            script: step.script,
            args: step.args,
            params: step.params,
            return_value: step.return_value,
            world:
              step.type === 'eval_main_world'
                ? 'main'
                : step.type === 'eval_isolated_world'
                  ? 'isolated'
                  : step.world,
            tabId,
          });
          const tab = await chrome.tabs.get(tabId);
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: true,
            result: {
              success: true,
              world: evalResponse.requested_world,
              execution_backend: evalResponse.execution_backend,
              result: evalResponse.result,
            },
            current_url: tab.url || '',
            current_tab_id: tabId,
          } as any);
          return;
        } catch (error: any) {
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'EVAL_ERROR',
            error_msg: error?.message || String(error)
          } as any);
          return;
        }
      }

      if (step?.type === 'switch_to_tab') {
        try {
          const tabIdentifier = (step as any).tab_identifier;
          let targetTabId: number | undefined;

          if (typeof tabIdentifier === 'number' && Number.isFinite(tabIdentifier)) {
            targetTabId = tabIdentifier;
          } else if (typeof tabIdentifier === 'string') {
            const raw = tabIdentifier.trim().toLowerCase();
            if (raw === 'workflow' || raw === 'current_workflow') {
              targetTabId = workflowTabId;
            } else {
              const n = parseInt(tabIdentifier, 10);
              if (Number.isFinite(n)) targetTabId = n;
            }
          }

          if (targetTabId === undefined) {
            throw new Error(`Invalid tab_identifier: ${String(tabIdentifier)}`);
          }

          // Validate tab exists
          await chrome.tabs.get(targetTabId);

          await chrome.tabs.update(targetTabId, { active: true });
          setSessionWorkflowTabId(targetTabId);

          await waitForTabComplete(targetTabId, 15000).catch(() => {});
          await ensureContentReady(targetTabId, 'contentScript.js', 8000).catch(() => {});

          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: true,
            result: { tabId: targetTabId, switched: true }
          } as any);
          return;
        } catch (error: any) {
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'TAB_SWITCH_ERROR',
            error_msg: error?.message || String(error)
          } as any);
          return;
        }
      }

      if (step?.type === 'close_current_tab') {
        try {
          const tabIdentifier = (step as any).tab_identifier;
          const maybeTabId =
            (typeof tabIdentifier === 'number' ? tabIdentifier : undefined) ??
            workflowTabId ??
            (typeof (message as any)?.data?.current_tab_id === 'number'
              ? (message as any).data.current_tab_id
              : undefined);

          if (maybeTabId === undefined) {
            sendResponseToBroker({
              req_id: isOrchestratorFormat ? undefined : requestId,
              task_id: isOrchestratorFormat ? requestId : undefined,
              success: true,
              result: { closed: false, reason: 'no_tab_id_available' },
            } as any);
            return;
          }

          try {
            await chrome.tabs.remove(maybeTabId);
          } catch (e: any) {
            const msg = e?.message || String(e);
            const alreadyClosed =
              msg.includes('No tab with id') ||
              msg.includes('No tab') ||
              msg.includes('The tab was closed');
            if (!alreadyClosed) throw e;
          }

          if (workflowTabId === maybeTabId) {
            setSessionWorkflowTabId(undefined);
          }

          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: true,
            result: { tabId: maybeTabId, closed: true },
          } as any);
          return;
        } catch (error: any) {
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'TAB_CLOSE_ERROR',
            error_msg: error?.message || String(error),
          } as any);
          return;
        }
      }

      if (tabId === undefined) {
        throw new Error(buildMissingWorkflowTabError(workflowSessionId, stepType || 'execute_step'));
      }

      if (step?.type === 'wait_for_navigation') {
        try {
          const urlPatternRaw = (step as any).url_pattern;
          const urlPattern =
            typeof urlPatternRaw === 'string' && urlPatternRaw.trim() ? urlPatternRaw.trim() : undefined;
          const timeoutMs = (step as any).timeout_ms || (step as any).timeoutMs || 30000;

          const url = await waitForNavigationOrUrlMatch(tabId, urlPattern, timeoutMs);

          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: true,
            result: { url, url_pattern: urlPattern || null }
          } as any);
          return;
        } catch (error: any) {
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'WAIT_NAVIGATION_ERROR',
            error_msg: error?.message || String(error)
          } as any);
          return;
        }
      }

      if (step?.type === 'wait_for_network_idle') {
        try {
          const idleTimeMs = Number((step as any).idle_time_ms ?? 500);
          const maxWaitMs = Number((step as any).max_wait_ms ?? 30000);
          const idle = Number.isFinite(idleTimeMs) ? Math.max(0, Math.round(idleTimeMs)) : 500;
          const maxWait = Number.isFinite(maxWaitMs) ? Math.max(0, Math.round(maxWaitMs)) : 30000;

          const start = Date.now();
          await waitForTabComplete(tabId, maxWait).catch(() => {});
          const spent = Date.now() - start;
          const remaining = Math.max(0, maxWait - spent);
          await new Promise(r => setTimeout(r, Math.min(idle, remaining)));

          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: true,
            result: { idle_time_ms: idle, max_wait_ms: maxWait, waited_ms: Date.now() - start }
          } as any);
          return;
        } catch (error: any) {
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'WAIT_NETWORK_IDLE_ERROR',
            error_msg: error?.message || String(error)
          } as any);
          return;
        }
      }

      if (step?.type === 'type_text' && (step as any).use_cdp === true) {
        try {
          const result = await executeDirectTypeTextStep(tabId, step);
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: true,
            result
          } as any);
          return;
        } catch (error: any) {
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'TYPE_TEXT_ERROR',
            error_msg: error?.message || String(error)
          } as any);
          return;
        }
      }

      if (step?.type === 'upload_file') {
        try {
          const { handleUploadFile } = await import('./actions/upload_file');
          const result = await withCdpLock(async () =>
            handleUploadFile({ ...(step as any), tabId })
          );
          await forceDetachCDP(tabId).catch(() => {
            console.warn(`Failed to detach CDP from tab ${tabId} after upload_file`);
          });
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: true,
            result
          } as any);
          return;
        } catch (error: any) {
          await forceDetachCDP(tabId).catch(() => {
            console.warn(`Failed to detach CDP from tab ${tabId} after upload_file error`);
          });
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'UPLOAD_FILE_ERROR',
            error_msg: error?.message || String(error)
          } as any);
          return;
        }
      }

      if (step?.type === 'take_screenshot') {
        try {
          let dataUrl = await captureScreenshotForTab(tabId, {
            format: (step as any).format,
            quality: (step as any).quality
          });

          const wantAnnotate = (step as any).annotate === true;
          let annotations: ScreenshotAnnotationRect[] | undefined;
          let annotateError: string | undefined;
          if (wantAnnotate) {
            try {
              const maxLabels = clampAnnotateMaxLabels((step as any).annotate_max_labels);
              const maxElements = clampAnnotateMaxElements((step as any).annotate_max_elements);

              await waitForTabComplete(tabId, 15000).catch(() => {});
              await ensureContentReady(tabId, 'contentScript.js', 8000).catch(() => {});

              const domResp: any = await sendMessageTopFrameWithRetry(
                tabId,
                {
                  cmd: 'get_dom_snapshot',
                  req_id: `${requestId}-dom`,
                  payload: { options: { maxElements, highlightElements: false } },
                },
                { attempts: 2, timeoutMs: 8000 }
              );
              const domSnapshot = domResp?.dom_snapshot;

              const collected = collectAnnotationRectsFromSnapshot(domSnapshot, maxLabels);
              annotations = collected.rects;
              if (annotations.length > 0) {
                dataUrl = await annotateScreenshotDataUrl(
                  dataUrl,
                  annotations,
                  collected.viewport,
                  normalizeScreenshotFormat((step as any).format),
                  (step as any).quality
                );
              } else {
                annotateError = 'no_spatial_info_in_snapshot';
              }
            } catch (e: any) {
              annotateError = e?.message || String(e);
            }
          }

          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: true,
            result: {
              type: 'screenshot',
              format: normalizeScreenshotFormat((step as any).format),
              full_page: !!(step as any).full_page,
              data_url: dataUrl,
              annotated: wantAnnotate && !annotateError,
              annotations: annotations?.map(a => ({ ref: a.ref, idx: a.idx, bbox: { x: a.x, y: a.y, width: a.width, height: a.height } })),
              annotate_error: annotateError,
            }
          } as any);
          return;
        } catch (error: any) {
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'SCREENSHOT_ERROR',
            error_msg: error?.message || String(error)
          } as any);
          return;
        }
      }

      // Break-glass: trusted click via CDP when explicitly requested.
      // Some sites ignore synthetic DOM clicks (isTrusted=false). When a workflow sets
      // `use_cdp: true` on a click_element step, perform the click here in the background
      // using chrome.debugger + Input.dispatchMouseEvent.
      if (step?.type === 'click_element' && (step as any).use_cdp === true) {
        try {
          console.log('[RZN] use_cdp click_element: begin', { selector: (step as any).selector });
          const { handleClickElement } = await import('./actions/click_element');
          const result = await withCdpLock(async () =>
            handleClickElement({ ...(step as any), tabId })
          );
          await forceDetachCDP(tabId).catch(() => {
            console.warn(`Failed to detach CDP from tab ${tabId} after CDP click`);
          });
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: true,
            result
          } as any);
          return;
        } catch (error: any) {
          console.warn('[RZN] use_cdp click_element: failed', { error: error?.message || String(error) });
          await forceDetachCDP(tabId).catch(() => {
            console.warn(`Failed to detach CDP from tab ${tabId} after CDP click error`);
          });
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'CDP_CLICK_ERROR',
            error_msg: error?.message || String(error),
          } as any);
          return;
        }
      }

      // Ensure content script is ready before sending message
      try {
        await ensureContentReady(tabId, 'contentScript.js', 8000);
        const response = await sendMessageTopFrameWithRetry(tabId, message, {
          attempts: 3,
          timeoutMs: 8000,
        });
        sendResponseToBroker(response);
      } catch (error: any) {
        const log = isContentScriptDisconnectError(error) ? console.warn : console.error;
        log('Failed to send message to content script:', error);
        sendResponseToBroker({
          req_id: isOrchestratorFormat ? undefined : requestId,
          task_id: isOrchestratorFormat ? requestId : undefined,
          success: false,
          error_code: 'CONTENT_SCRIPT_ERROR',
          error_msg: error?.message || String(error)
        });
      }
    } catch (error) {
      console.error('Error forwarding to content script:', error);
      sendResponseToBroker({
        req_id: isOrchestratorFormat ? undefined : requestId,
        task_id: isOrchestratorFormat ? requestId : undefined,
        success: false,
        error_code: 'CONTENT_SCRIPT_ERROR',
        error_msg: (error as Error).message
      });
    }
  } else if (message.cmd === 'get_pruned_dom' || message.cmd === 'get_dom_snapshot' || message.cmd === 'get_dom_hash' || message.cmd === 'process_dom' || message.cmd === 'detect_auto_list' || message.cmd === 'execute_extraction_plan' ||
             (command === 'execute_step' && message.payload?.step?.type === 'get_dom_snapshot')) {
    // Forward to content script for DOM processing - use workflow tab if available
    try {
      let tabId: number;
      const mayUseCurrentTab =
        sessionMayUseActiveTab(workflowSessionId) ||
        message.payload?.use_current_tab === true ||
        message.payload?.use_active_tab === true ||
        message.payload?.step?.use_current_tab === true ||
        message.payload?.step?.use_active_tab === true;

      // First check if we have a stored workflow tab
      if (workflowTabId !== undefined) {
        try {
          const tab = await chrome.tabs.get(workflowTabId);
          if (tab) {
            tabId = workflowTabId;
            console.log(`Using workflow tab for DOM processing: ${tabId}`);
          } else {
            setSessionWorkflowTabId(undefined); // Tab no longer exists
          }
        } catch (e) {
          setSessionWorkflowTabId(undefined); // Tab no longer exists
        }
      }

      // If no workflow tab, either bind to the visible active tab or create a dedicated one.
      if (workflowTabId === undefined) {
        if (mayUseCurrentTab) {
          tabId = await getActiveTabIdOrThrow('dom processing');
          setSessionWorkflowTabId(tabId);
          await waitForTabComplete(tabId, 15000).catch(() => {});
          await ensureContentReady(tabId, 'contentScript.js', 8000).catch(() => {});
        } else {
          const newTab = await chrome.tabs.create({ url: 'https://www.example.com', active: true });
          tabId = newTab.id!;
          setSessionWorkflowTabId(tabId);
          await waitForTabComplete(tabId, 15000).catch(() => {});
          await ensureContentReady(tabId, 'contentScript.js', 8000).catch(() => {});
        }
      } else {
        tabId = workflowTabId;
      }

      // Ensure the content script is injected and responsive before sending
      try {
        await ensureContentReady(tabId, 'contentScript.js', 8000);
      } catch (e) {
        console.warn('ensureContentReady failed for DOM request, proceeding to sendMessage:', e);
      }

      // Target the top-level frame to avoid iframe snapshots (e.g., YouTube embeds)
      const response = await sendMessageTopFrameWithRetry(tabId, message, {
        attempts: 3,
        timeoutMs: 8000,
      });
      // Ensure DOM snapshot and hash are properly forwarded
      if (response.dom_snapshot || response.dom_hash) {
        response.dom_snapshot = response.dom_snapshot;
        response.dom_hash = response.dom_hash;
      }
      try {
        (response as any).capabilities = await buildCapabilities(tabId);
      } catch (e) {
        // Non-fatal; capabilities are best-effort.
      }
      sendResponseToBroker(response);
    } catch (error) {
      console.error('Error processing DOM request:', error);
      sendResponseToBroker({
        req_id: isOrchestratorFormat ? undefined : requestId,
        task_id: isOrchestratorFormat ? requestId : undefined,
        success: false,
        error_code: 'DOM_PROCESSING_ERROR',
        error_msg: (error as Error).message
      });
    }
  } else if (message.cmd === 'observe') {
    // Observe candidates (selectors) with caching keyed by hostname + instruction + scope + dom_hash
    try {
      let tabId: number;
      const mayUseCurrentTab =
        sessionMayUseActiveTab(workflowSessionId) ||
        message.payload?.use_current_tab === true ||
        message.payload?.use_active_tab === true;
      if (workflowTabId !== undefined) {
        try {
          const tab = await chrome.tabs.get(workflowTabId);
          if (tab) tabId = workflowTabId; else setSessionWorkflowTabId(undefined);
        } catch { setSessionWorkflowTabId(undefined); }
      }
      if (workflowTabId === undefined) {
        if (mayUseCurrentTab) {
          tabId = await getActiveTabIdOrThrow('observe');
          setSessionWorkflowTabId(tabId);
          await waitForTabComplete(tabId, 15000).catch(() => {});
          await ensureContentReady(tabId, 'contentScript.js', 8000).catch(() => {});
        } else {
          const newTab = await chrome.tabs.create({ url: 'https://www.example.com', active: true });
          tabId = newTab.id!;
          setSessionWorkflowTabId(tabId);
          await waitForTabComplete(tabId, 15000).catch(() => {});
          await ensureContentReady(tabId, 'contentScript.js', 8000).catch(() => {});
        }
      } else {
        tabId = workflowTabId!;
      }

      await ensureContentReady(tabId, 'contentScript.js', 8000).catch(() => {});

      const tab = await chrome.tabs.get(tabId);
      const hostname = tab.url ? new URL(tab.url).hostname : 'unknown';

      // Get dom hash first
      let domHashResp: any;
      try {
        domHashResp = await sendMessageTopFrame(tabId, { cmd: 'get_dom_hash', req_id: `${requestId}-hash` });
      } catch (e) {
        domHashResp = { success: false };
      }
      const dom_hash = domHashResp?.hash || 'nohash';
      const instr = (message.payload?.instruction || message.payload?.query || '').toString();
      const scopeSel = (message.payload?.scope_selector || '').toString();
      const instrHash = simpleHash(instr);
      const cacheKey = `${hostname}|${instrHash}|${scopeSel}|${dom_hash}`;

      // Cache hit?
      const entry = observeCache.get(cacheKey);
      const now = Date.now();
      if (entry && (now - entry.ts) < OBSERVE_TTL_MS) {
        sendResponseToBroker({
          req_id: requestId,
          success: true,
          result: { candidates: entry.candidates, cached: true, dom_hash },
        } as any);
        return;
      }

      // Forward as execute_step to content script handler
      const step = {
        type: 'observe',
        instruction: message.payload?.instruction,
        scope_selector: message.payload?.scope_selector,
        max_items: message.payload?.max_items ?? 10,
      };

      let response: any;
      try {
        response = await sendMessageTopFrame(tabId, { cmd: 'execute_step', req_id: requestId, payload: { step } });
      } catch (e: any) {
        sendResponseToBroker({
          req_id: requestId,
          success: false,
          error_code: 'CONTENT_SCRIPT_ERROR',
          error_msg: e?.message || String(e)
        });
        return;
      }

      if (response?.success && response?.result?.candidates) {
        observeCache.set(cacheKey, { ts: now, dom_hash, candidates: response.result.candidates });
      }

      sendResponseToBroker(response);
    } catch (error) {
      console.error('Error handling observe request:', error);
      sendResponseToBroker({
        req_id: requestId,
        success: false,
        error_code: 'OBSERVE_ERROR',
        error_msg: (error as Error).message
      });
    }
  } else if (command === 'get_active_tab') {
    // Handle get active tab request
    try {
      const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
      if (tabs.length > 0) {
        const tab = tabs[0];
        sendResponseToBroker({
          req_id: requestId,
          success: true,
          tabId: tab.id,
          url: tab.url,
          title: tab.title
        });
      } else {
        sendResponseToBroker({
          req_id: requestId,
          success: false,
          error_code: 'NO_ACTIVE_TAB',
          error_msg: 'No active tab found'
        });
      }
    } catch (error) {
      console.error('Error getting active tab:', error);
      sendResponseToBroker({
        req_id: requestId,
        success: false,
        error_code: 'TAB_ERROR',
        error_msg: (error as Error).message
      });
    }
  } else if (command === 'send_to_tab') {
    // Handle sending message to specific tab
    try {
      const tabId = message.tab_id || message.tabId;
      const tabMessage = message.message;

      if (!tabId || !tabMessage) {
        sendResponseToBroker({
          req_id: requestId,
          success: false,
          error_code: 'INVALID_REQUEST',
          error_msg: 'Missing tab_id or message'
        });
        return;
      }

      const response = await sendMessageTopFrame(tabId, tabMessage);
      sendResponseToBroker({
        req_id: requestId,
        success: true,
        ...response
      });
    } catch (error) {
      console.error('Error sending to tab:', error);
      sendResponseToBroker({
        req_id: requestId,
        success: false,
        error_code: 'TAB_MESSAGE_ERROR',
        error_msg: (error as Error).message
      });
    }
  } else if (message.cmd === 'cdp_action') {
    // Handle CDP-enhanced actions with new frameRouter
    try {
      let tabId: number;
      const mayUseCurrentTab =
        sessionMayUseActiveTab(workflowSessionId) ||
        message.payload?.use_current_tab === true ||
        message.payload?.use_active_tab === true;

      // Get tab ID
      if (workflowTabId !== undefined) {
        tabId = workflowTabId;
      } else {
        // Either bind to the current active tab or ensure a dedicated workflow tab exists.
        if (mayUseCurrentTab) {
          tabId = await getActiveTabIdOrThrow('cdp_action');
          setSessionWorkflowTabId(tabId);
          await waitForTabComplete(tabId, 15000).catch(() => {});
          await ensureContentReady(tabId, 'contentScript.js', 8000).catch(() => {});
        } else {
          const newTab = await chrome.tabs.create({ url: 'https://www.example.com', active: true });
          tabId = newTab.id!;
          setSessionWorkflowTabId(tabId);
          await waitForTabComplete(tabId, 15000).catch(() => {});
          await ensureContentReady(tabId, 'contentScript.js', 8000).catch(() => {});
        }
      }

      if (!(await isCDPEnabledForTab(tabId))) {
        sendResponseToBroker({
          req_id: requestId,
          success: false,
          error_code: 'CDP_DISABLED',
          error_msg: 'CDP is disabled for this tab/domain (enable flags.cdpEnable to use CDP actions)'
        });
        return;
      }

      // Use CDP integration with lease-managed sessions
      const result = await withCdpLock(async () => {
        await frameRouter.attachToTab(tabId);
        await extendCDPLease(tabId);
        try {
          return await cdpIntegration.executeAction(tabId, message.payload);
        } finally {
          await forceDetachCDP(tabId).catch(() => {
            console.warn(`Failed to detach CDP from tab ${tabId} after cdp_action`);
          });
        }
      });

      sendResponseToBroker({
        req_id: requestId,
        success: result.success,
        result: result.data,
        error_code: result.error ? 'CDP_ACTION_FAILED' : undefined,
        error_msg: result.error
      });
    } catch (error) {
      console.error('CDP action error:', error);
      sendResponseToBroker({
        req_id: requestId,
        success: false,
        error_code: 'CDP_ACTION_ERROR',
        error_msg: (error as Error).message
      });
    }
  } else if (message.cmd === 'get_cdp_context') {
    // Get DOM context using CDP with short-lived session
    try {
      let tabId: number;

      if (workflowTabId !== undefined) {
        tabId = workflowTabId;
      } else {
        if (!sessionMayUseActiveTab(workflowSessionId)) {
          sendResponseToBroker({
            req_id: requestId,
            success: false,
            error_code: 'NO_WORKFLOW_TAB',
            error_msg: buildMissingWorkflowTabError(workflowSessionId, 'get_cdp_context')
          });
          return;
        }
        tabId = await getActiveTabIdOrThrow('get_cdp_context');
      }

      if (!(await isCDPEnabledForTab(tabId))) {
        sendResponseToBroker({
          req_id: requestId,
          success: false,
          error_code: 'CDP_DISABLED',
          error_msg: 'CDP is disabled for this tab/domain (enable flags.cdpEnable to use CDP context)'
        });
        return;
      }

      const context = await withCdpLock(async () => {
        await frameRouter.attachToTab(tabId);
        await extendCDPLease(tabId);
        try {
          return await cdpIntegration.getDOMContext(tabId, message.payload?.options);
        } finally {
          await forceDetachCDP(tabId).catch(() => {
            console.warn(`Failed to detach CDP from tab ${tabId} after get_cdp_context`);
          });
        }
      });

      sendResponseToBroker({
        req_id: requestId,
        success: true,
        result: context
      });
    } catch (error) {
      console.error('CDP context error:', error);
      sendResponseToBroker({
        req_id: requestId,
        success: false,
        error_code: 'CDP_CONTEXT_ERROR',
        error_msg: (error as Error).message
      });
    }
  } else if (message.cmd === 'set_execution_tier') {
    // Set execution tier for CDP strategy
    try {
      cdpIntegration.configureStrategy({ tier: message.payload.tier as ExecutionTier });
      sendResponseToBroker({
        req_id: requestId,
        success: true,
        result: { tier: message.payload.tier }
      });
    } catch (error) {
      console.error('Set tier error:', error);
      sendResponseToBroker({
        req_id: requestId,
        success: false,
        error_code: 'CONFIG_ERROR',
        error_msg: (error as Error).message
      });
    }
  } else if (message.cmd === 'get_interactive_elements') {
    try {
      let tabId: number;
      if (workflowTabId !== undefined) {
        tabId = workflowTabId;
      } else {
        if (!sessionMayUseActiveTab(workflowSessionId)) {
          sendResponseToBroker({
            req_id: requestId,
            success: false,
            error_code: 'NO_WORKFLOW_TAB',
            error_msg: buildMissingWorkflowTabError(workflowSessionId, 'get_interactive_elements')
          });
          return;
        }
        tabId = await getActiveTabIdOrThrow('get_interactive_elements');
      }

      if (!(await isCDPEnabledForTab(tabId))) {
        sendResponseToBroker({
          req_id: requestId,
          success: false,
          error_code: 'CDP_DISABLED',
          error_msg: 'CDP is disabled for this tab/domain (enable flags.cdpEnable to use interactive elements)'
        });
        return;
      }

      const list = await withCdpLock(async () => {
        await frameRouter.attachToTab(tabId);
        await extendCDPLease(tabId);
        try {
          return await cdpGetInteractiveElements(tabId);
        } finally {
          await forceDetachCDP(tabId).catch(() => {
            console.warn(`Failed to detach CDP from tab ${tabId} after get_interactive_elements`);
          });
        }
      });

      sendResponseToBroker({
        req_id: requestId,
        success: true,
        result: { elements: list }
      });
    } catch (error) {
      console.error('get_interactive_elements error:', error);
      sendResponseToBroker({
        req_id: requestId,
        success: false,
        error_code: 'CDP_INTERACTIVE_ERROR',
        error_msg: (error as Error).message
      });
    }
  } else if (message.rzn && message.cmd) {
    // Handle new static action format for CSP compliance
    try {
      let tabId: number;

      // Use global workflow tab if available
      if (workflowTabId !== undefined) {
        try {
          const tab = await chrome.tabs.get(workflowTabId);
          if (tab) {
            tabId = workflowTabId;
          } else {
            setSessionWorkflowTabId(undefined);
          }
        } catch (e) {
          setSessionWorkflowTabId(undefined);
        }
      }

      // Only default-session or explicit use_current_tab flows may fall back to the active tab.
      if (workflowTabId === undefined) {
        if (!sessionMayUseActiveTab(workflowSessionId)) {
          sendResponseToBroker({
            req_id: requestId,
            success: false,
            error_code: 'NO_WORKFLOW_TAB',
            error_msg: buildMissingWorkflowTabError(workflowSessionId, message.cmd || 'tab action')
          });
          return;
        }
        tabId = await getActiveTabIdOrThrow(message.cmd || 'tab action');
      }

      // Forward to content script
        const response = await sendMessageTopFrame(tabId, message);
        sendResponseToBroker({
          req_id: requestId,
          success: response.success || response.ok,
          result: response.data,
          error_code: response.error,
          error_msg: response.details || response.error
        });
    } catch (error) {
      console.error('Error forwarding static action to content script:', error);
      sendResponseToBroker({
        req_id: requestId,
        success: false,
        error_code: 'STATIC_ACTION_ERROR',
        error_msg: (error as Error).message
      });
    }
  } else {
    sendResponseToBroker({
      req_id: isOrchestratorFormat ? undefined : requestId,
      task_id: isOrchestratorFormat ? requestId : undefined,
      success: false,
      error_code: 'UNKNOWN_COMMAND',
      error_msg: `Unknown command: ${command}`
    });
  }
}

const testBrokerResponseWaiters: Map<string, (response: ExtensionResponse) => void> = new Map();

function getResponseCorrelationId(response: ExtensionResponse): string | undefined {
  const reqId = (response as any)?.req_id;
  if (typeof reqId === 'string' && reqId.trim().length > 0) {
    return reqId;
  }

  const taskId = (response as any)?.task_id;
  if (typeof taskId === 'string' && taskId.trim().length > 0) {
    return taskId;
  }

  return undefined;
}

function resolveTestBrokerResponse(response: ExtensionResponse): boolean {
  const correlationId = getResponseCorrelationId(response);
  if (!correlationId) {
    return false;
  }

  const waiter = testBrokerResponseWaiters.get(correlationId);
  if (!waiter) {
    return false;
  }

  testBrokerResponseWaiters.delete(correlationId);
  waiter(response);
  return true;
}

// Send response back to broker
function sendResponseToBroker(response: ExtensionResponse): void {
  if (resolveTestBrokerResponse(response)) {
    return;
  }

  const postToNative = (payload: any): boolean => {
    if (!nativePort) return false;
    try {
      nativePort.postMessage(payload);
      return true;
    } catch (error: any) {
      console.warn('[NativeReconnect] Failed to send response; reconnecting native port:', error);
      disconnectNativePort(error?.message || 'native port postMessage failed');
      scheduleReconnect();
      return false;
    }
  };

  if (nativePort) {
    // Adapt response format based on the original message format
    // If we have a task_id, it's from the orchestrator and expects different format
    if (response.task_id || response.action) {
      // IMPORTANT: The native broker correlates extension replies using `req_id`.
      // Some call paths (e.g. when we respond using task_id/action) previously omitted `req_id`,
      // which can cause the broker to never match the in-flight request and the host to hang.
      const reqId = response.req_id || response.task_id;

      // Orchestrator format - map error_code/error_msg to error field
      const orchestratorResponse: any = {
        action: response.action || 'task_result',
        task_id: response.task_id || response.req_id,
        req_id: reqId,
        success: response.success,
        result: response.result || null,
        error: response.error || response.error_msg || null
      };

      // Preserve html_content, steps, current_url, dom_snapshot, and dom_hash if present
      if ((response as any).html_content) {
        orchestratorResponse.html_content = (response as any).html_content;
      }
      if ((response as any).steps) {
        orchestratorResponse.steps = (response as any).steps;
      }
      if ((response as any).current_url) {
        orchestratorResponse.current_url = (response as any).current_url;
      }
      if (response.dom_snapshot) {
        orchestratorResponse.dom_snapshot = response.dom_snapshot;
      }
      if (response.dom_hash) {
        orchestratorResponse.dom_hash = response.dom_hash;
      }
      console.log('Sending orchestrator response to broker:', orchestratorResponse);
      postToNative(orchestratorResponse);
    } else {
      // Extension format - keep as is
      console.log('Sending extension response to broker:', response);
      postToNative(response);
    }
  } else {
    console.warn('[NativeReconnect] Dropping stale response because native port is not connected', {
      req_id: response.req_id,
      task_id: response.task_id,
      success: response.success,
      error_code: response.error_code,
    });
    scheduleReconnect();
  }
}

// Send test ping to broker
function sendTestPing(): void {
  if (nativePort) {
    const pingMessage: BrokerMessage = {
      cmd: 'ping',
      req_id: `ping-${Date.now()}`,
      payload: {}
    };
    console.log('Sending ping to broker:', pingMessage);
    nativePort.postMessage(pingMessage);
  } else {
    console.error('Cannot send ping: native port not connected');
  }
}

// Initialize connection on startup (if available)
// Note: onStartup is not available in all contexts, so we check first
if (chrome.runtime?.onStartup?.addListener) {
  chrome.runtime.onStartup.addListener(() => {
    logInfo('Extension startup event');
    ensureNativeKeepaliveAlarm();
    connectToNative();
  });
} else {
  console.warn('[RZN] chrome.runtime.onStartup not available in this context');
}

if (chrome.runtime?.onInstalled?.addListener) {
  chrome.runtime.onInstalled.addListener(async () => {
    logInfo('Extension installed/updated event');
    ensureNativeKeepaliveAlarm();
    connectToNative();

    // Register content scripts for auto-injection
    try {
      await chrome.scripting.registerContentScripts([{
        id: 'rzn_main',
        js: ['contentScript.js'],
        matches: ['http://*/*','https://*/*'],
        allFrames: true,
        runAt: 'document_idle'
      }]);
      console.log('Registered content script rzn_main');
    } catch (e) {
      console.warn('registerContentScripts failed:', e);
    }
  });
} else {
  console.warn('[RZN] chrome.runtime.onInstalled not available in this context');
}

// Initialize CDP integration with safe defaults
cdpIntegration.initialize();

// Alarm-based reconnect fallback (MV3 timers can be dropped when the SW is suspended).
if (chrome.alarms?.onAlarm?.addListener) {
  chrome.alarms.onAlarm.addListener((alarm) => {
    if (alarm?.name === RECONNECT_ALARM_NAME) {
      if (nativePort) return;
      console.log('[NativeReconnect] Alarm fired; attempting native host connect');
      connectToNative();
      return;
    }

    if (alarm?.name === NATIVE_KEEPALIVE_ALARM_NAME) {
      if (nativePort) return;
      console.log('[NativeKeepalive] Alarm fired; native port missing, attempting reconnect');
      connectToNative();
    }
  });
}

// Connect immediately when service worker starts
logInfo('RZN Background Script loaded');
ensureNativeKeepaliveAlarm();
void loadWorkflowSessionsFromStorage();
connectToNative();

// Expose test function for debugging
(globalThis as any).sendTestPing = sendTestPing;
(globalThis as any).reconnectNative = connectToNative;
(globalThis as any).__rznTestRunWorkflowSteps = async (steps: any[], data?: any) => {
  await loadWorkflowSessionsFromStorage();
  const requestId = `pw-test-${Date.now()}`;
  const message: any = {
    action: 'perform_task',
    task_id: requestId,
    task: { steps },
    data: data || {},
  };
  await executeWorkflow(message, requestId, true);
  return true;
};
(globalThis as any).__rznTestHandleBrokerMessage = async (message: any, sessionId?: string) => {
  await loadWorkflowSessionsFromStorage();
  const normalizedMessage = message && typeof message === 'object' ? { ...message } : {};
  const fallbackReqId = `pw-broker-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const correlationId = normalizeSessionId(
    (typeof normalizedMessage.req_id === 'string' && normalizedMessage.req_id) ||
      (typeof normalizedMessage.task_id === 'string' && normalizedMessage.task_id) ||
      fallbackReqId,
  );

  if (typeof normalizedMessage.req_id !== 'string' || normalizedMessage.req_id.trim().length === 0) {
    normalizedMessage.req_id = correlationId;
  }

  const responsePromise = new Promise<ExtensionResponse>((resolve, reject) => {
    const timeoutId = setTimeout(() => {
      testBrokerResponseWaiters.delete(correlationId);
      reject(new Error(`Timed out waiting for broker response (${correlationId})`));
    }, 20000);

    testBrokerResponseWaiters.set(correlationId, (response) => {
      clearTimeout(timeoutId);
      resolve(response);
    });
  });

  try {
    await handleBrokerMessage(
      normalizedMessage as BrokerMessage,
      normalizeSessionId(sessionId || resolveSessionId(normalizedMessage as BrokerMessage)),
    );
  } catch (error) {
    testBrokerResponseWaiters.delete(correlationId);
    throw error;
  }

  return await responsePromise;
};

// Add workflow execution handler
async function executeWorkflow(
  message: BrokerMessage,
  requestId: string,
  isOrchestratorFormat: boolean,
  sessionId: string = resolveSessionId(message),
): Promise<void> {
  const workflowSessionId = normalizeSessionId(sessionId);

  console.log('Executing workflow from orchestrator');
  console.log('Task structure:', JSON.stringify(message.task, null, 2));

  // Log to the aggregated log file
  // Log to the aggregated log file
  logInfo('executeWorkflow called', { 
    hasTask: !!message.task,
    hasWorkflow: !!(message.task?.workflow),
    hasSteps: !!(message.task?.steps),
    requestId 
  });

  // Determine steps early (so we can decide whether we need an initial tab).
  // Supports both CLI workflow format (task.workflow) and orchestrator task format (task.steps).
  let steps: any[] | undefined;
  if (message.task?.workflow) {
    const workflow = message.task.workflow;
    steps = workflow?.browser_automation?.sequences?.[0]?.steps;
    if (!steps) {
      sendResponseToBroker({
        req_id: isOrchestratorFormat ? undefined : requestId,
        task_id: isOrchestratorFormat ? requestId : undefined,
        success: false,
        error_code: 'INVALID_WORKFLOW',
        error_msg: 'Invalid workflow structure'
      });
      return;
    }
  } else if (message.task?.steps) {
    steps = message.task.steps;
  } else {
    sendResponseToBroker({
      req_id: isOrchestratorFormat ? undefined : requestId,
      task_id: isOrchestratorFormat ? requestId : undefined,
      success: false,
      error_code: 'INVALID_TASK',
      error_msg: 'No steps found in task'
    });
    return;
  }

  const preferCurrentTab =
    message.task?.workflow?.browser_automation?.use_current_tab === true ||
    message.task?.workflow?.browser_automation?.use_active_tab === true;

  // Resolve workflow tab:
  // 1) Prefer the session-provided current_tab_id (sent by the broker client)
  // 2) Fall back to the extension's cached global workflow tab id
  // 3) As a last resort, create/use an active tab depending on step type
  let workflowTabId: number | undefined = undefined;
  const setSessionWorkflowTabId = (tabId: number | undefined) => {
    workflowTabId = tabId;
    setWorkflowTabId(workflowSessionId, tabId);
  };
  const mayUseActiveTab = sessionMayUseActiveTab(workflowSessionId, preferCurrentTab);
  const sessionTabIdRaw = (message as any)?.data?.current_tab_id;
  if (typeof sessionTabIdRaw === 'number') {
    workflowTabId = sessionTabIdRaw;
  }
  if (workflowTabId === undefined) {
    workflowTabId = getWorkflowTabId(workflowSessionId);
  }

  const firstStepType = steps?.[0]?.type;
  // Some steps (like open_new_tab and close_current_tab) should NOT trigger tab initialization.
  // close_current_tab is typically used for best-effort cleanup.
  const requiresExistingTab = firstStepType !== 'open_new_tab' && firstStepType !== 'close_current_tab';

  // If a tab id is provided via session/global state but is no longer valid, clear it so we can
  // create a fresh workflow tab below. Avoid recursion here: broker messages can reuse the same
  // data.current_tab_id across retries.
  if (requiresExistingTab && workflowTabId !== undefined) {
    try {
      await chrome.tabs.get(workflowTabId);
    } catch {
      logInfo(`Workflow tab no longer exists, clearing: ${workflowTabId}`);
      setSessionWorkflowTabId(undefined);
      workflowTabId = undefined;
    }
  }

  // Ensure we have a valid tab at the start of the workflow (when needed)
  if (requiresExistingTab && workflowTabId === undefined) {
    logInfo('No workflow tab, initializing...');
    try {
      if (preferCurrentTab) {
        logInfo('Binding workflow to current active tab');
        const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
        const activeTabId = tabs[0]?.id;
        if (activeTabId === undefined) {
          throw new Error('No active tab available for use_current_tab workflow');
        }
        workflowTabId = activeTabId;
        setSessionWorkflowTabId(workflowTabId);
        try {
          await waitForTabComplete(workflowTabId, 15000).catch(() => {});
          await ensureContentReady(workflowTabId, 'contentScript.js', 8000);
        } catch (e) {
          logError('Failed to ensure active tab readiness', e);
        }
      } else {
        // Prefer a dedicated workflow tab over hijacking the user's active tab.
        // This keeps automation isolated and allows deterministic cleanup (tab close) after workflows.
        logInfo('Creating dedicated tab for workflow');
        const newTab = await chrome.tabs.create({
          url: 'about:blank',
          active: true,
        });
        workflowTabId = newTab.id!;
        setSessionWorkflowTabId(workflowTabId);
        logInfo(
          `Created new tab for workflow: ${workflowTabId}, URL: ${newTab.url || newTab.pendingUrl}`
        );

        // Wait for the new tab to load and ensure content script is ready
        logInfo('Waiting for new tab to load and content script to be ready...');
        try {
          await waitForTabComplete(workflowTabId, 15000);
          logInfo('New tab loaded successfully');
          await ensureContentReady(workflowTabId, 'contentScript.js', 8000);
          logInfo('Content script is ready in new workflow tab');
        } catch (e) {
          logError('Failed to ensure tab and content script readiness', e);
        }
      }
    } catch (error) {
      logError('Failed to initialize workflow tab', error);
      sendResponseToBroker({
        req_id: requestId,
        success: false,
        error_code: 'TAB_INIT_FAILED',
        error_msg: `Failed to initialize workflow tab: ${error}`
      });
      return;
    }
  } else if (requiresExistingTab) {
    logInfo(`Using existing workflow tab: ${workflowTabId}`);
  }

  try {
    const results: any[] = [];
    const includeDomSnapshot: boolean = message?.data?.include_dom_snapshot !== false;
    let latestDomSnapshot: any | undefined;
    let latestDomHash: string | undefined;

    // Execute each step sequentially
    for (let i = 0; i < steps.length; i++) {
      const step = steps[i];
      console.log(`Executing step ${i + 1}/${steps.length}:`, step.type);

      // Create a step execution message
      const stepMessage = {
        cmd: 'execute_step',
        req_id: `${requestId}_step_${i}`,
        payload: { step }
      };

      // Convert old action types to enhanced versions when needed
      let convertedStep = step;

      // Map old action types to new enhanced versions if they need CDP
      if (step.type === 'submit_input' || step.type === 'fill_input_field') {
        // These actions might need CDP for cross-origin or complex inputs
        // For now, pass through to content script which will handle them
        console.log('Processing input action:', step.type);
      }

      // Handle special step types in the background script
      if (step.type === 'open_new_tab') {
        try {
          const url = (step.url && typeof step.url === 'string' && step.url.trim() !== '')
            ? step.url
            : 'about:blank';
          const shouldPrepareContent = isHttpNavigableUrl(url);

          const newTab = await chrome.tabs.create({ url, active: true });
          const tabId = newTab.id!;
          console.log(`Opened new tab for workflow: ${tabId} (${url})`);

          // Treat this as the active workflow tab for subsequent steps.
          setSessionWorkflowTabId(tabId);

          if (shouldPrepareContent) {
            await waitForTabUrl(tabId, candidateUrl => isHttpNavigableUrl(candidateUrl), 15000).catch(() => {});

            const openedTab = await chrome.tabs.get(tabId).catch(() => undefined);
            const openedUrl = getTabNavigationUrl(openedTab);
            if (isHttpNavigableUrl(openedUrl)) {
              await waitForTabComplete(tabId, 15000).catch(() => {});
              await ensureContentReady(tabId, 'contentScript.js', 8000).catch(() => {});
            } else {
              console.warn(
                `[open_new_tab] Skipping content readiness for non-http tab ${tabId}: ${openedUrl || 'unknown'}`
              );
            }
          }

          results.push({ type: 'open_new_tab', url, tabId, success: true });
          continue;
        } catch (error) {
          console.error('open_new_tab error:', error);
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'TAB_CREATE_ERROR',
            error_msg: `Failed to open new tab: ${(error as Error).message}`
          });
          return;
        }
      } else if (step.type === 'switch_to_tab') {
        try {
          const tabIdentifier = (step as any).tab_identifier;
          let targetTabId: number | undefined;

          if (typeof tabIdentifier === 'number' && Number.isFinite(tabIdentifier)) {
            targetTabId = tabIdentifier;
          } else if (typeof tabIdentifier === 'string') {
            const raw = tabIdentifier.trim().toLowerCase();
            if (raw === 'workflow' || raw === 'current_workflow') {
              targetTabId = workflowTabId ?? getWorkflowTabId(workflowSessionId);
            } else {
              const n = parseInt(tabIdentifier, 10);
              if (Number.isFinite(n)) targetTabId = n;
            }
          }

          if (targetTabId === undefined) {
            throw new Error(`Invalid tab_identifier: ${String(tabIdentifier)}`);
          }

          // Validate tab exists
          await chrome.tabs.get(targetTabId);

          await chrome.tabs.update(targetTabId, { active: true });
          setSessionWorkflowTabId(targetTabId);

          await waitForTabComplete(targetTabId, 15000).catch(() => {});
          await ensureContentReady(targetTabId, 'contentScript.js', 8000).catch(() => {});

          results.push({ type: 'switch_to_tab', tabId: targetTabId, success: true });
          continue;
        } catch (error: any) {
          console.error('switch_to_tab error:', error);
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'TAB_SWITCH_ERROR',
            error_msg: `Failed to switch tabs: ${(error as Error).message}`
          });
          return;
        }
      } else if (step.type === 'navigate_to_url') {
        try {
          // Skip navigation if URL is empty or invalid
          if (!step.url || step.url.trim() === '') {
            console.log('Skipping navigation - empty URL provided');
            results.push({ 
              type: 'navigation',
              status: 'skipped',
              reason: 'Empty URL'
            });
            continue;
          }

          let tabId: number;

          if (workflowTabId !== undefined) {
            // Reuse existing workflow tab
            tabId = workflowTabId;
            console.log(`Reusing existing workflow tab ID: ${tabId}`);
            await chrome.tabs.update(tabId, { url: step.url });
          } else {
            // Create a dedicated workflow tab if none exists (do not hijack the user's active tab).
            const newTab = await chrome.tabs.create({ url: step.url, active: true });
            tabId = newTab.id!;
            console.log(`Created new workflow tab ID: ${tabId}`);

            // Store tab ID for subsequent steps
            setSessionWorkflowTabId(tabId);
          }

          // Wait for navigation according to step.wait
          await waitForNavigation(tabId, step.wait, step.timeout_ms || step.timeoutMs || 15000);

          // Ensure content script is ready after navigation
          try {
            await ensureContentReady(tabId, 'contentScript.js', 8000);
            console.log('Content script ready after navigation');
          } catch (injectionError) {
            console.warn('Content script not ready after navigation:', injectionError);
          }

          // Navigation successful
          results.push({ type: 'navigation', url: step.url, success: true });
          continue;
        } catch (error) {
          console.error('Navigation error:', error);
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'NAVIGATION_ERROR',
            error_msg: `Navigation failed: ${(error as Error).message}`
          });
          return;
        }
      } else if (step.type === 'wait_for_navigation') {
        try {
          const tabId =
            workflowTabId ??
            (mayUseActiveTab ? await getActiveTabIdOrThrow('wait_for_navigation') : undefined);
          if (tabId === undefined) {
            throw new Error(buildMissingWorkflowTabError(workflowSessionId, 'wait_for_navigation'));
          }

          const urlPatternRaw = (step as any).url_pattern;
          const urlPattern =
            typeof urlPatternRaw === 'string' && urlPatternRaw.trim() ? urlPatternRaw.trim() : undefined;
          const timeoutMs = (step as any).timeout_ms || (step as any).timeoutMs || 30000;

          const url = await waitForNavigationOrUrlMatch(tabId, urlPattern, timeoutMs);
          results.push({ type: 'wait_for_navigation', url, url_pattern: urlPattern || null, success: true });
          continue;
        } catch (error: any) {
          console.error('wait_for_navigation error:', error);
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'WAIT_NAVIGATION_ERROR',
            error_msg: `Failed to wait for navigation: ${error?.message || String(error)}`,
          });
          return;
        }
      } else if (step.type === 'wait_for_network_idle') {
        try {
          const tabId =
            workflowTabId ??
            (mayUseActiveTab ? await getActiveTabIdOrThrow('wait_for_network_idle') : undefined);
          if (tabId === undefined) {
            throw new Error(buildMissingWorkflowTabError(workflowSessionId, 'wait_for_network_idle'));
          }

          const idleTimeMs = Number((step as any).idle_time_ms ?? 500);
          const maxWaitMs = Number((step as any).max_wait_ms ?? 30000);
          const idle = Number.isFinite(idleTimeMs) ? Math.max(0, Math.round(idleTimeMs)) : 500;
          const maxWait = Number.isFinite(maxWaitMs) ? Math.max(0, Math.round(maxWaitMs)) : 30000;

          const start = Date.now();
          await waitForTabComplete(tabId, maxWait).catch(() => {});
          const spent = Date.now() - start;
          const remaining = Math.max(0, maxWait - spent);
          await new Promise(r => setTimeout(r, Math.min(idle, remaining)));

          results.push({ type: 'wait_for_network_idle', idle_time_ms: idle, max_wait_ms: maxWait, waited_ms: Date.now() - start, success: true });
          continue;
        } catch (error: any) {
          console.error('wait_for_network_idle error:', error);
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'WAIT_NETWORK_IDLE_ERROR',
            error_msg: `Failed to wait for network idle: ${error?.message || String(error)}`,
          });
          return;
        }
      } else if (step.type === 'type_text' && (step as any).use_cdp === true) {
        try {
          const tabId =
            workflowTabId ??
            (mayUseActiveTab ? await getActiveTabIdOrThrow('type_text') : undefined);
          if (tabId === undefined) {
            throw new Error(buildMissingWorkflowTabError(workflowSessionId, 'type_text'));
          }

          const result = await executeDirectTypeTextStep(tabId, step);
          results.push(result);
          continue;
        } catch (error: any) {
          console.error('type_text error:', error);
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'TYPE_TEXT_ERROR',
            error_msg: `Failed to type text: ${error?.message || String(error)}`,
          });
          return;
        }
      } else if (step.type === 'upload_file') {
        try {
          const tabId =
            workflowTabId ??
            (mayUseActiveTab ? await getActiveTabIdOrThrow('upload_file') : undefined);
          if (tabId === undefined) {
            throw new Error(buildMissingWorkflowTabError(workflowSessionId, 'upload_file'));
          }

          const { handleUploadFile } = await import('./actions/upload_file');
          const result = await withCdpLock(async () =>
            handleUploadFile({ ...(step as any), tabId })
          );
          // upload_file uses a direct debugger attach/detach cycle. Clear the shared
          // frame-router lease state so the next CDP-backed step reattaches cleanly.
          await forceDetachCDP(tabId).catch(() => {
            console.warn(`Failed to clear CDP state after upload_file on tab ${tabId}`);
          });
          results.push(result);
          continue;
        } catch (error: any) {
          console.error('upload_file error:', error);
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'UPLOAD_FILE_ERROR',
            error_msg: `Failed to upload file: ${error?.message || String(error)}`,
          });
          return;
        }
      } else if (step.type === 'get_page_source') {
        // Handle get_page_source in background script
        try {
          // Use workflow tab ID if available. Only default-session or explicit
          // use_current_tab workflows may fall back to the active tab.
          let tabId: number;
          let targetTab: chrome.tabs.Tab;

          if (workflowTabId !== undefined) {
            tabId = workflowTabId;
            console.log(`Using cached workflow tab ID: ${tabId}`);
            targetTab = await chrome.tabs.get(tabId);
          } else {
            if (!mayUseActiveTab) {
              sendResponseToBroker({
                req_id: isOrchestratorFormat ? undefined : requestId,
                task_id: isOrchestratorFormat ? requestId : undefined,
                success: false,
                error_code: 'NO_WORKFLOW_TAB',
                error_msg: buildMissingWorkflowTabError(workflowSessionId, 'get_page_source')
              });
              return;
            }
            const activeTabId = await getActiveTabIdOrThrow('get_page_source');
            targetTab = await chrome.tabs.get(activeTabId);
            tabId = targetTab.id!;
          }

          // Check if we're trying to access a restricted URL
          if (!targetTab.url || targetTab.url.startsWith('chrome://') || targetTab.url.startsWith('chrome-extension://')) {
            console.log(`Cannot get page source from restricted URL: ${targetTab.url}`);
            sendResponseToBroker({
              req_id: isOrchestratorFormat ? undefined : requestId,
              task_id: isOrchestratorFormat ? requestId : undefined,
              success: false,
              error_code: 'RESTRICTED_URL',
              error_msg: `Cannot access page source from ${targetTab.url?.startsWith('chrome-extension://') ? 'extension' : 'system'} pages. Please navigate to a website first.`
            });
            return;
          }

          const timeoutMs = (step as any).timeout_ms || (step as any).timeoutMs || 15000;

          // Best-effort settle: avoid injecting while the top frame is being swapped during redirects.
          // This is generic and bounded: we don't require "complete" forever, we just give it a chance.
          await waitForTabComplete(tabId, Math.min(timeoutMs, 15000)).catch(() => {});
          await ensureContentReady(tabId, 'contentScript.js', 8000).catch(() => {});

          // Execute script to get page source
          let result: chrome.scripting.InjectionResult<any> | undefined;
          let lastError: any;
          for (let attempt = 0; attempt < 3; attempt++) {
            try {
              [result] = await chrome.scripting.executeScript({
                target: { tabId: tabId },
                func: async () => {
                  if (document.readyState !== 'complete') {
                    await new Promise(resolve => window.addEventListener('load', resolve, { once: true }));
                  }
                  return document.documentElement.outerHTML;
                }
              });
              break;
            } catch (e: any) {
              lastError = e;
              const msg = (e && (e.message || e.toString?.())) ? (e.message || e.toString()) : String(e);
              const retriable =
                msg.includes('Frame with ID') && msg.includes('removed') ||
                msg.includes('No tab with id') ||
                msg.includes('The tab was closed');
              if (!retriable || attempt === 2) throw e;
              console.warn(`[get_page_source] executeScript failed (attempt ${attempt + 1}/3):`, msg);
              await new Promise(r => setTimeout(r, 250));
              await waitForTabComplete(tabId, 3000).catch(() => {});
            }
          }
          if (!result) throw lastError || new Error('No executeScript result for page source');

          let html: string = result.result as string;
          try {
            const [pruned] = await chrome.scripting.executeScript({ target: { tabId: tabId }, func: pruneDOM });
            if (typeof pruned.result === 'string') {
              html = pruned.result;
            }
          } catch (e) {
            console.warn('DOM prune failed:', e);
          }

          // Also try to fetch the DOM snapshot for richer downstream debugging/planning.
          let dom_snapshot: any | undefined;
          let selector_map: any | undefined;
          try {
            const domSnapshotResponse = await sendMessageTopFrame(tabId, {
              cmd: 'get_dom_snapshot',
              req_id: requestId,
              payload: {
                options: {
                  maxElements: 200,
                  highlightElements: false
                }
              }
            });
            dom_snapshot = domSnapshotResponse?.dom_snapshot;
            selector_map = domSnapshotResponse?.selector_map;
          } catch (snapshotError) {
            console.warn('Failed to get DOM snapshot, returning HTML only:', snapshotError);
          }

          const pageSourceResult: any = {
            type: 'page_source',
            html,
            success: true
          };
          if (dom_snapshot) pageSourceResult.dom_snapshot = dom_snapshot;
          if (selector_map) pageSourceResult.selector_map = selector_map;
          results.push(pageSourceResult);
          continue;
        } catch (error) {
          console.error('Get page source error:', error);
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'PAGE_SOURCE_ERROR',
            error_msg: `Failed to get page source: ${(error as Error).message}`
          });
          return;
        }
      }

      if (step.type === 'close_current_tab') {
        try {
          const tabIdentifier = (step as any).tab_identifier;
          const maybeTabId =
            (typeof tabIdentifier === 'number' ? tabIdentifier : undefined) ??
            workflowTabId ??
            (typeof (message as any)?.data?.current_tab_id === 'number'
              ? (message as any).data.current_tab_id
              : undefined);

          if (maybeTabId === undefined) {
            results.push({
              type: 'close_current_tab',
              closed: false,
              reason: 'no_tab_id_available',
              success: true,
            });
            continue;
          }

          // Best-effort: if the tab is already gone, treat as a no-op success.
          try {
            await chrome.tabs.remove(maybeTabId);
          } catch (e: any) {
            const msg = e?.message || String(e);
            const alreadyClosed =
              msg.includes('No tab with id') ||
              msg.includes('No tab') ||
              msg.includes('The tab was closed');
            if (!alreadyClosed) throw e;
          }

          if (workflowTabId === maybeTabId) {
            setSessionWorkflowTabId(undefined);
          }

          results.push({ type: 'close_current_tab', tabId: maybeTabId, closed: true, success: true });
          continue;
        } catch (error: any) {
          console.error('close_current_tab error:', error);
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'TAB_CLOSE_ERROR',
            error_msg: `Failed to close tab: ${error?.message || String(error)}`,
          });
          return;
        }
      }

      if (step.type === 'take_screenshot') {
        try {
          if (workflowTabId === undefined) {
            throw new Error('No workflow tab available for screenshot');
          }

          let dataUrl = await captureScreenshotForTab(workflowTabId, {
            format: (step as any).format,
            quality: (step as any).quality
          });

          const wantAnnotate = (step as any).annotate === true;
          let annotations: ScreenshotAnnotationRect[] | undefined;
          let annotateError: string | undefined;
          if (wantAnnotate) {
            try {
              const maxLabels = clampAnnotateMaxLabels((step as any).annotate_max_labels);
              const maxElements = clampAnnotateMaxElements((step as any).annotate_max_elements);

              await waitForTabComplete(workflowTabId, 15000).catch(() => {});
              await ensureContentReady(workflowTabId, 'contentScript.js', 8000).catch(() => {});

              const domResp: any = await sendMessageTopFrameWithRetry(
                workflowTabId,
                {
                  cmd: 'get_dom_snapshot',
                  req_id: `${requestId}-dom`,
                  payload: { options: { maxElements, highlightElements: false } },
                },
                { attempts: 2, timeoutMs: 8000 }
              );
              const domSnapshot = domResp?.dom_snapshot;

              const collected = collectAnnotationRectsFromSnapshot(domSnapshot, maxLabels);
              annotations = collected.rects;
              if (annotations.length > 0) {
                dataUrl = await annotateScreenshotDataUrl(
                  dataUrl,
                  annotations,
                  collected.viewport,
                  normalizeScreenshotFormat((step as any).format),
                  (step as any).quality
                );
              } else {
                annotateError = 'no_spatial_info_in_snapshot';
              }
            } catch (e: any) {
              annotateError = e?.message || String(e);
            }
          }

          results.push({
            type: 'screenshot',
            format: normalizeScreenshotFormat((step as any).format),
            full_page: !!(step as any).full_page,
            data_url: dataUrl,
            annotated: wantAnnotate && !annotateError,
            annotations: annotations?.map(a => ({ ref: a.ref, idx: a.idx, bbox: { x: a.x, y: a.y, width: a.width, height: a.height } })),
            annotate_error: annotateError,
          });
          continue;
        } catch (error: any) {
          console.error('Screenshot error:', error);
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'SCREENSHOT_ERROR',
            error_msg: `Failed to take screenshot: ${error?.message || String(error)}`
          });
          return;
        }
      }

      // For non-navigation steps, execute via content script
      let tabId: number;
      if (workflowTabId !== undefined) {
        tabId = workflowTabId;
        console.log(`Using cached workflow tab ID for content script: ${tabId}`);
      } else {
        if (!mayUseActiveTab) {
          sendResponseToBroker({
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: 'NO_WORKFLOW_TAB',
            error_msg: buildMissingWorkflowTabError(workflowSessionId, step.type || 'content-script step')
          });
          return;
        }
        tabId = await getActiveTabIdOrThrow(step.type || 'content-script step');
        setSessionWorkflowTabId(tabId); // Cache for future steps
      }

      // Check if we're on a valid page (not extension pages, chrome:// pages, etc.)
      const tab = await chrome.tabs.get(tabId);
      if (!tab.url || tab.url.startsWith('chrome://') || tab.url.startsWith('chrome-extension://')) {
        console.error(`Cannot execute content script on ${tab.url || 'invalid URL'}`);
        sendResponseToBroker({
          req_id: isOrchestratorFormat ? undefined : requestId,
          task_id: isOrchestratorFormat ? requestId : undefined,
          success: false,
          error_code: 'INVALID_PAGE',
          error_msg: `Cannot execute actions on ${tab.url?.startsWith('chrome-extension://') ? 'extension' : 'system'} pages. Please navigate to a valid website first.`
        });
        return;
      }

      try {
        // Ensure content script is ready before sending message
        await ensureContentReady(tabId, 'contentScript.js', 8000);

        // Send message to content script (target top frame)
        const response = await sendMessageTopFrameWithRetry(tabId, stepMessage, { attempts: 3, timeoutMs: 8000 });
        if (!response.success) {
          // Step failed, stop workflow. Keep the response compact by default; snapshots are optional.
          const errResp: any = {
            req_id: isOrchestratorFormat ? undefined : requestId,
            task_id: isOrchestratorFormat ? requestId : undefined,
            success: false,
            error_code: response.error_code || 'STEP_FAILED',
            error_msg: `Step ${i + 1} failed: ${response.error_msg || 'Unknown error'}`,
            current_url: response.current_url
          };
          if (includeDomSnapshot) {
            errResp.dom_snapshot = response.dom_snapshot;
            errResp.dom_hash = response.dom_hash;
          }
          sendResponseToBroker(errResp);
          return;
        }

        if (response.result) {
          results.push(response.result);
        }

        // Track the latest snapshot (optionally returned to broker at the end).
        // Do NOT push snapshots into `results` (it can easily exceed native messaging limits).
        if (response.dom_snapshot) {
          latestDomSnapshot = response.dom_snapshot;
          latestDomHash = response.dom_hash;
        }

        // If the step triggered a navigation, wait for it to settle so the next step doesn't
        // hit "Could not establish connection. Receiving end does not exist." during load.
        // Keep this generic: we only wait when the tab is actually loading.
        try {
          const maybeNavSteps = new Set(['click_element', 'submit_input', 'press_key', 'press_special_key']);
          if (maybeNavSteps.has(step.type)) {
            const settleTimeout = step.timeout_ms || step.timeoutMs || 15000;
            const pollStart = Date.now();

            // A short polling window catches the common race where navigation starts just after
            // we check tab.status once.
            while (Date.now() - pollStart < 1500) {
              const after = await chrome.tabs.get(tabId);
              if (after.status === 'loading') break;
              await new Promise(r => setTimeout(r, 150));
            }

            // If a navigation is in progress, wait for it. Otherwise, still re-ensure content
            // readiness once more to handle fast redirects.
            const after = await chrome.tabs.get(tabId);
            if (after.status === 'loading') {
              await waitForNavigation(tabId, 'domcontentloaded', settleTimeout);
            }
            await ensureContentReady(tabId, 'contentScript.js', 8000);
          }
        } catch (e) {
          console.warn('Post-step navigation settle failed (continuing):', e);
        }
      } catch (error) {
        console.error(`Error executing step ${i + 1}:`, error);
        sendResponseToBroker({
          req_id: isOrchestratorFormat ? undefined : requestId,
          task_id: isOrchestratorFormat ? requestId : undefined,
          success: false,
          error_code: 'STEP_EXECUTION_ERROR',
          error_msg: `Step ${i + 1} error: ${(error as Error).message}`
        });
        return;
      }

      // Small delay between steps
      await new Promise(resolve => setTimeout(resolve, 100));
    }

    // All steps completed successfully
    // Get the current URL from the active tab
    let currentUrl = '';
    if (workflowTabId !== undefined) {
      try {
        const tab = await chrome.tabs.get(workflowTabId);
        currentUrl = tab.url || '';
      } catch (e) {
        console.warn('Could not get current URL:', e);
      }
    }

    // Format response with HTML content for broker client compatibility
    const response: any = {
      req_id: isOrchestratorFormat ? undefined : requestId,
      task_id: isOrchestratorFormat ? requestId : undefined,
      success: true,
      result: { results },
      current_url: currentUrl,
      current_tab_id: workflowTabId
    };

    // Include latest DOM snapshot only when requested (or default behavior).
    // This keeps responses smaller and prevents native host disconnects on heavy pages.
    if (includeDomSnapshot && latestDomSnapshot) {
      response.dom_snapshot = latestDomSnapshot;
      response.dom_hash = latestDomHash;
    }

    // Also include HTML content in expected format for broker client compatibility
    const htmlResults = results.filter(r => r.type === 'page_source' && r.html);
    if (htmlResults.length > 0) {
      response.html_content = htmlResults[htmlResults.length - 1].html; // Use latest HTML
      response.steps = results.map(r => {
        if (r.type === 'page_source') {
          return { data: { html_content: r.html } };
        } else {
          return { data: r };
        }
      });
    }

    sendResponseToBroker(response);

  } catch (error) {
    console.error('Workflow execution error:', error);
    sendResponseToBroker({
      req_id: isOrchestratorFormat ? undefined : requestId,
      task_id: isOrchestratorFormat ? requestId : undefined,
      success: false,
      error_code: 'WORKFLOW_ERROR',
      error_msg: (error as Error).message
    });
  }
}

// Track pending native input callbacks
const nativeInputCallbacks: Map<string, {
  sendResponse: (response: any) => void;
  timeoutId: NodeJS.Timeout;
}> = new Map();

function normalizeCloudServerUrl(serverUrl: string): string {
  const url = new URL(serverUrl);
  if (url.pathname === '/') {
    url.pathname = '';
  }
  url.search = '';
  url.hash = '';
  return url.toString().replace(/\/$/, '');
}

async function fetchCloudJson<T>(input: string, init?: RequestInit): Promise<T> {
  const response = await fetch(input, {
    ...init,
    headers: {
      'content-type': 'application/json',
      ...(init?.headers || {}),
    },
  });
  const json = await response.json().catch(() => ({}));
  if (!response.ok) {
    throw new Error((json as any)?.error || `Cloud request failed (${response.status})`);
  }
  return json as T;
}

async function getCloudActorStatus(): Promise<CloudActorStatus | null> {
  const response = await callNativeHostControl('cloud_get_status', {}, {
    timeoutMs: CLOUD_UI_NATIVE_TIMEOUT_MS,
  });
  if (!response.success) {
    throw new Error(response.error_msg || response.error || 'Failed to load cloud actor status');
  }
  return (response.result || null) as CloudActorStatus | null;
}

async function getCloudUiState(): Promise<any> {
  let status: CloudActorStatus | null = null;
  let error: string | undefined;
  try {
    status = await getCloudActorStatus();
  } catch (nativeError: any) {
    error = nativeError?.message || String(nativeError);
  }

  return {
    success: true,
    nativeHostConnected: !!nativePort,
    nativeHostName: brokerHostInUse,
    cloudStatus: status,
    error,
  };
}

async function issueCloudPairingCode(payload: any): Promise<any> {
  const serverUrl = normalizeCloudServerUrl(String(payload?.serverUrl || payload?.server_url || ''));
  const workspaceId = String(payload?.workspaceId || payload?.workspace_id || 'default').trim() || 'default';
  const ttlSecs = Math.max(60, Number(payload?.ttlSecs ?? payload?.ttl_secs ?? CLOUD_UI_PAIRING_TTL_SECS));
  return await fetchCloudJson(`${serverUrl}/v1/pairing-codes`, {
    method: 'POST',
    body: JSON.stringify({
      workspace_id: workspaceId,
      ttl_secs: ttlSecs,
    }),
  });
}

async function pairCloudActorFromUi(payload: any): Promise<any> {
  const serverUrl = normalizeCloudServerUrl(String(payload?.serverUrl || payload?.server_url || ''));
  const actorId = String(payload?.actorId || payload?.actor_id || '').trim();
  const pairingCode = String(payload?.pairingCode || payload?.pairing_code || '').trim();
  if (!actorId) {
    throw new Error('actor_id is required');
  }
  if (!pairingCode) {
    throw new Error('pairing_code is required');
  }

  const redeemed = await fetchCloudJson<any>(`${serverUrl}/v1/pair/redeem`, {
    method: 'POST',
    body: JSON.stringify({
      pairing_code: pairingCode,
      actor_id: actorId,
    }),
  });

  const controlResponse = await callNativeHostControl(
    'cloud_set_config',
    {
      config: {
        version: CLOUD_ACTOR_CONFIG_VERSION,
        actor_id: redeemed.actor_id,
        workspace_id: redeemed.workspace_id,
        actor_token: redeemed.actor_token,
        server_url: redeemed.server_url,
        websocket_url: redeemed.websocket_url,
        paired_at_ms: redeemed.paired_at_ms,
        connect_timeout_ms: 15_000,
        request_timeout_ms: 45_000,
      },
    },
    { timeoutMs: CLOUD_UI_NATIVE_TIMEOUT_MS }
  );
  if (!controlResponse.success) {
    throw new Error(controlResponse.error_msg || controlResponse.error || 'Failed to apply cloud actor config');
  }

  return {
    pairing: redeemed,
    cloudStatus: controlResponse.result,
  };
}

async function clearCloudActorFromUi(): Promise<any> {
  const response = await callNativeHostControl('cloud_clear_config', {}, {
    timeoutMs: CLOUD_UI_NATIVE_TIMEOUT_MS,
  });
  if (!response.success) {
    throw new Error(response.error_msg || response.error || 'Failed to clear cloud actor config');
  }
  return {
    cloudStatus: response.result || null,
  };
}

async function runCloudBrowserCommandFromUi(payload: any): Promise<any> {
  const serverUrl = normalizeCloudServerUrl(String(payload?.serverUrl || payload?.server_url || ''));
  const actorId = String(payload?.actorId || payload?.actor_id || '').trim();
  const cmd = String(payload?.command?.cmd || payload?.cmd || '').trim();
  const sessionId = String(payload?.sessionId || payload?.session_id || '').trim();
  const timeoutMs = Math.max(1_000, Number(payload?.timeoutMs ?? payload?.timeout_ms ?? 45_000));

  if (!serverUrl) {
    throw new Error('server_url is required');
  }
  if (!actorId) {
    throw new Error('actor_id is required');
  }
  if (!cmd) {
    throw new Error('command.cmd is required');
  }

  let commandPayload = payload?.command?.payload ?? payload?.commandPayload ?? payload?.command_payload;
  if (typeof commandPayload === 'string') {
    const trimmed = commandPayload.trim();
    commandPayload = trimmed ? JSON.parse(trimmed) : undefined;
  }

  let commandData = payload?.command?.data ?? payload?.commandData ?? payload?.command_data;
  if (typeof commandData === 'string') {
    const trimmed = commandData.trim();
    commandData = trimmed ? JSON.parse(trimmed) : undefined;
  }

  return await fetchCloudJson(`${serverUrl}/v1/commands/browser`, {
    method: 'POST',
    body: JSON.stringify({
      actor_id: actorId,
      session_id: sessionId || undefined,
      timeout_ms: timeoutMs,
      command: {
        cmd,
        payload: commandPayload,
        data: commandData,
      },
    }),
  });
}

// Handle native input messages from content script
if (guardListener(chrome.runtime?.onMessage, 'chrome.runtime.onMessage')) {
  chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  // Forward content logs into native logger and console
  if (message && message.type === 'CONTENT_LOG') {
    try {
      const lvl = message.level || 'info';
      const msg = message.message || '';
      const meta = { context: message.context, ...message.metadata };
      if (lvl === 'error') console.error('[RZN:CS]', msg, meta);
      else if (lvl === 'warn') console.warn('[RZN:CS]', msg, meta);
      else if (lvl === 'info') console.info('[RZN:CS]', msg, meta);
      else console.debug('[RZN:CS]', msg, meta);
      // Also send to broker unified log if native host is connected
      try {
        logInfo(`[CS] ${msg}`, meta);
      } catch {}
      sendResponse({ ok: true });
    } catch (e: any) {
      sendResponse({ ok: false, error: e?.message || String(e) });
    }
    return true;
  }

  if (message?.cmd === 'cloud_ui_get_state') {
    (async () => {
      try {
        sendResponse(await getCloudUiState());
      } catch (error: any) {
        sendResponse({ success: false, error: error?.message || String(error) });
      }
    })();
    return true;
  }

  if (message?.cmd === 'cloud_ui_issue_pairing_code') {
    (async () => {
      try {
        const pairing = await issueCloudPairingCode(message.payload || {});
        sendResponse({ success: true, pairing });
      } catch (error: any) {
        sendResponse({ success: false, error: error?.message || String(error) });
      }
    })();
    return true;
  }

  if (message?.cmd === 'cloud_ui_pair_actor') {
    (async () => {
      try {
        const result = await pairCloudActorFromUi(message.payload || {});
        sendResponse({ success: true, ...result });
      } catch (error: any) {
        sendResponse({ success: false, error: error?.message || String(error) });
      }
    })();
    return true;
  }

  if (message?.cmd === 'cloud_ui_disconnect_actor') {
    (async () => {
      try {
        const result = await clearCloudActorFromUi();
        sendResponse({ success: true, ...result });
      } catch (error: any) {
        sendResponse({ success: false, error: error?.message || String(error) });
      }
    })();
    return true;
  }

  if (message?.cmd === 'cloud_ui_run_remote_command') {
    (async () => {
      try {
        const result = await runCloudBrowserCommandFromUi(message.payload || {});
        sendResponse({ success: true, result });
      } catch (error: any) {
        sendResponse({ success: false, error: error?.message || String(error) });
      }
    })();
    return true;
  }

  // Handle test messages
  if (message.type === 'PING') {
    sendResponse({ success: true });
    return false;
  }

  // Site profiles were removed from runtime (selectors are workflow/test data only).
  // Keep these message types as a compatibility stub for older tooling.
  if (message.type === 'CHECK_SITE_PROFILE') {
    sendResponse({ success: false, error: 'SITE_PROFILES_DISABLED' });
    return false;
  }

  if (message.type === 'GET_SITE_PROFILES') {
    sendResponse({ success: false, error: 'SITE_PROFILES_DISABLED' });
    return false;
  }

  if (message.type === 'GET_FEATURE_FLAGS') {
    getFlags(message.domain || '').then(flags => {
      sendResponse({ success: true, flags });
    }).catch(error => {
      sendResponse({ success: false, error: error.message });
    });
    return true;
  }

  if (message.cmd === 'rzn_system_notification') {
    (async () => {
      try {
        const title = String(message.title || 'RZN Automation');
        const body = String(message.message || 'Manual intervention required.');
        const notificationId = await createSystemNotification(title, body);
        sendResponse({ success: true, notificationId });
      } catch (error: any) {
        sendResponse({ success: false, error: error?.message || String(error) });
      }
    })();
    return true;
  }

  if (message.cmd === 'take_screenshot') {
    (async () => {
      try {
        const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
        const tabId = sender?.tab?.id ?? getWorkflowTabId(DEFAULT_WORKFLOW_SESSION_ID) ?? tabs[0]?.id;

        if (tabId === undefined) {
          throw new Error('No target tab available for screenshot');
        }

        let dataUrl = await captureScreenshotForTab(tabId, {
          format: (message as any).format,
          quality: (message as any).quality
        });

        const wantAnnotate = (message as any).annotate === true;
        let annotations: ScreenshotAnnotationRect[] | undefined;
        let annotateError: string | undefined;
        if (wantAnnotate) {
          try {
            const maxLabels = clampAnnotateMaxLabels((message as any).annotate_max_labels);
            const maxElements = clampAnnotateMaxElements((message as any).annotate_max_elements);

            await waitForTabComplete(tabId, 15000).catch(() => {});
            await ensureContentReady(tabId, 'contentScript.js', 8000).catch(() => {});

            const reqId = (message as any).req_id || `screenshot-${Date.now()}`;
            const domResp: any = await sendMessageTopFrameWithRetry(
              tabId,
              {
                cmd: 'get_dom_snapshot',
                req_id: `${reqId}-dom`,
                payload: { options: { maxElements, highlightElements: false } },
              },
              { attempts: 2, timeoutMs: 8000 }
            );
            const domSnapshot = domResp?.dom_snapshot;
            const collected = collectAnnotationRectsFromSnapshot(domSnapshot, maxLabels);
            annotations = collected.rects;
            if (annotations.length > 0) {
              dataUrl = await annotateScreenshotDataUrl(
                dataUrl,
                annotations,
                collected.viewport,
                normalizeScreenshotFormat((message as any).format),
                (message as any).quality
              );
            } else {
              annotateError = 'no_spatial_info_in_snapshot';
            }
          } catch (e: any) {
            annotateError = e?.message || String(e);
          }
        }

        sendResponse({
          success: true,
          dataUrl,
          annotated: wantAnnotate && !annotateError,
          annotations: annotations?.map(a => ({ ref: a.ref, idx: a.idx, bbox: { x: a.x, y: a.y, width: a.width, height: a.height } })),
          annotate_error: annotateError,
        });
      } catch (error: any) {
        sendResponse({ success: false, error: error?.message || String(error) });
      }
    })();
    return true;
  }

  if (message.cmd === 'eval_with_scripting') {
    (async () => {
      try {
        const response = await runScriptingEval(sender, message);
        sendResponse({
          success: true,
          execution_backend: response.execution_backend,
          requested_world: response.requested_world,
          result: response.result,
        });
      } catch (error: any) {
        sendResponse({ success: false, error: error?.message || String(error) });
      }
    })();
    return true;
  }

  if (message.cmd === 'eval_with_cdp') {
    (async () => {
      try {
        const response = await runCdpEval(sender, message);
        sendResponse({
          success: true,
          execution_backend: response.execution_backend,
          requested_world: response.requested_world,
          result: response.result,
        });
      } catch (error: any) {
        sendResponse({ success: false, error: error?.message || String(error) });
      }
    })();
    return true;
  }

  if (message.cmd === 'download_image') {
    // Handle image download request
    console.log('Downloading image:', message.url);
    chrome.downloads.download({
      url: message.url,
      filename: message.filename,
      saveAs: false,
      conflictAction: 'uniquify'
    }, (downloadId) => {
      if (chrome.runtime.lastError) {
        console.error('Download failed:', chrome.runtime.lastError);
        sendResponse({ success: false, error: chrome.runtime.lastError.message });
      } else {
        console.log('Download started:', downloadId);
        sendResponse({ success: true, downloadId: downloadId });
      }
    });
    return true; // Keep message channel open for async response
  } else if (message.cmd === 'native_input' && nativePort) {
    console.log('Forwarding native_input request to broker:', message);

    // Store callback for when broker responds
    const messageId = message.req_id;
    const timeoutId = setTimeout(() => {
      nativeInputCallbacks.delete(messageId);
      sendResponse({ ok: false, error: 'Native input timeout' });
    }, 5000);

    nativeInputCallbacks.set(messageId, { sendResponse, timeoutId });

    // Forward to broker
    nativePort.postMessage(message);

    return true; // Keep message channel open for async response
  } else if (message.cmd === 'export_flight_recorder') {
    // Handle flight recorder export request
    console.log('[Background] Exporting flight recorder session');

    (async () => {
      try {
        // Get the active tab to request export
        const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
        if (!tab.id) throw new Error('No active tab found');

        // Request export from content script
        const response = await chrome.tabs.sendMessage(tab.id, { cmd: 'export_recorder_data' });

        if (response.success) {
          // Create download
          const blob = new Blob([response.data], { type: 'application/json' });
          const url = URL.createObjectURL(blob);
          const filename = `rzn-debug-${response.session_id}-${new Date().toISOString().slice(0, 19)}.json`;

          chrome.downloads.download({
            url: url,
            filename: filename,
            saveAs: true,
            conflictAction: 'uniquify'
          }, (downloadId) => {
            URL.revokeObjectURL(url);
            if (chrome.runtime.lastError) {
              console.error('Flight recorder export failed:', chrome.runtime.lastError);
              sendResponse({ success: false, error: chrome.runtime.lastError.message });
            } else {
              console.log(`Flight recorder exported as ${filename}`);
              sendResponse({ success: true, filename, downloadId });
            }
          });
        } else {
          sendResponse({ success: false, error: response.error });
        }
      } catch (error: any) {
        console.error('Flight recorder export failed:', error);
        sendResponse({ success: false, error: error.message });
      }
    })();

    return true; // Keep message channel open for async response
  } else if (message.action === 'press_key_cdp') {
    // Handle first-class press_key action using CDP
    console.log('Handling press_key_cdp:', message.key);

    (async () => {
      try {
        const targetTabId = await resolveMessageTargetTab(sender);
        const { handlePressKey } = await import('./actions/press_key');
        const result = await runWithAttachedCdpTab(targetTabId, async () =>
          handlePressKey({
            key: message.key,
            tabId: targetTabId,
            manageDebuggerLifecycle: false,
          })
        );
        sendResponse({ success: true, ...result });
      } catch (error: any) {
        console.error('press_key_cdp failed:', error);
        sendResponse({ success: false, error: error.message });
      }
    })();

    return true; // Keep message channel open for async response
  } else if (message.action === 'type_text_cdp') {
    console.log('Handling type_text_cdp');

    (async () => {
      try {
        const targetTabId = await resolveMessageTargetTab(sender);
        const { handleTypeText } = await import('./actions/type_text');
        const result = await runWithAttachedCdpTab(targetTabId, async () =>
          handleTypeText({
            text: message.text,
            tabId: targetTabId,
            manageDebuggerLifecycle: false,
          })
        );
        sendResponse({ success: true, ...result });
      } catch (error: any) {
        console.error('type_text_cdp failed:', error);
        sendResponse({ success: false, error: error.message });
      }
    })();

    return true; // Keep message channel open for async response
  } else if (message.cmd === 'rzn_fetch_ax_slice') {
    // Handle AX slice request for accessibility-first DOM capture
    console.log('Handling rzn_fetch_ax_slice:', message);

    (async () => {
      try {
        const nodes = await withCdpLock(async () =>
          fetchAXSlice(message.maxNodes ?? 150, !!message.viewportOnly)
        );
        sendResponse({ ok: true, nodes });
      } catch (error: any) {
        console.error('rzn_fetch_ax_slice failed:', error);
        sendResponse({ ok: false, error: error.message });
      }
    })();

    return true; // Keep message channel open for async response
  } else if (message.cmd === 'get_ax_tree') {
    // Build simplified AX tree text and id->url map (top frame by default)
    (async () => {
      try {
        const maxNodes = message.payload?.maxNodes ?? 400;
        const includeFrames = !!message.payload?.includeFrames;
        const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
        if (!tab?.id) throw new Error('No active tab found');
        const tabId = tab.id;

        if (!(await isCDPEnabledForTab(tabId))) {
          throw new Error('CDP disabled for this tab/domain');
        }

        const nodesOut = await withCdpLock(async () => {
          await frameRouter.attachToTab(tabId);
          await extendCDPLease(tabId);

          try {
            const frames = frameRouter.getFrameSessionsForTab(tabId);
            const collected: Array<{ id: number; role: string; name?: string; frameId: string; actionable: boolean; href?: string; nodeName?: string }> = [];

            for (const { sessionId, frameId } of frames) {
              try {
                if (!includeFrames && frameId !== frames[0].frameId) continue;
                const { cdpClient } = await import('./cdp/cdpClient');
                await cdpClient.enableDomains({ sessionId }, ['Accessibility', 'DOM']);
                const axResult = await cdpClient.sendCommand<any>({ sessionId }, 'Accessibility.getFullAXTree', {});
                if (!axResult?.nodes) continue;

              const interestingRoles = new Set(['link','heading','button','textbox','combobox','list','listitem','article','cell','row','table','img','paragraph']);

              for (const node of axResult.nodes) {
                const role = node.role?.value as string | undefined;
                if (!role || !interestingRoles.has(role)) continue;
                const backendNodeId = node.backendDOMNodeId ?? node.backendNodeId;
                if (!backendNodeId) continue;

                let nodeId: number | undefined;
                try {
                  const push = await cdpClient.sendCommand<any>({ sessionId }, 'DOM.pushNodesByBackendIdsToFrontend', { backendNodeIds: [backendNodeId] });
                  nodeId = push?.nodeIds?.[0];
                } catch {}

                let href: string | undefined;
                let nodeName: string | undefined;
                if (nodeId) {
                  try {
                    const desc = await cdpClient.sendCommand<any>({ sessionId }, 'DOM.describeNode', { nodeId });
                    nodeName = desc?.node?.nodeName;
                  } catch {}
                  try {
                    const attrs = await cdpClient.sendCommand<any>({ sessionId }, 'DOM.getAttributes', { nodeId });
                    const arr: string[] = attrs?.attributes || [];
                    for (let i = 0; i + 1 < arr.length; i += 2) {
                      if (arr[i].toLowerCase() === 'href') { href = arr[i+1]; break; }
                    }
                  } catch {}
                }

                const actionable = ['button','link','textbox','combobox','menuitem','switch','checkbox','radio','tab','option']
                  .includes((role || '').toLowerCase());
                collected.push({ id: backendNodeId, role, name: node.name?.value, frameId, actionable, href, nodeName });
                if (collected.length >= maxNodes) break;
              }
            } catch (e) {
              console.warn('[AXTree] frame error', e);
            }
            if (collected.length >= maxNodes) break;
          }

            return collected;
          } finally {
            await forceDetachCDP(tabId).catch(() => {
              console.warn(`Failed to detach CDP from tab ${tabId} after AX tree`);
            });
          }
        });

        let text = 'AX Document Summary\n';
        const idUrlMap: Record<string,string> = {};
        for (const n of nodesOut) {
          const safeName = (n.name || '').replace(/\s+/g, ' ').trim();
          text += `[${n.id}] role=${n.role}${n.nodeName?` node=${n.nodeName}`:''}${safeName?` name="${safeName}"`:''}${n.actionable?' actionable':''}\n`;
          if (n.href && /^https?:\/\//i.test(n.href)) {
            idUrlMap[String(n.id)] = n.href;
          }
        }

        sendResponse({ success: true, text, id_url_map: idUrlMap, count: nodesOut.length });
      } catch (error: any) {
        console.error('get_ax_tree failed:', error);
        sendResponse({ success: false, error: error.message });
      }
    })();
    return true;
  } else if (message.cmd === 'set_flags') {
    // Set feature flag overrides (async)
    (async () => {
      try {
        const overrides = message.payload?.overrides || {};
        await setFlags(overrides);
        sendResponse({ success: true });
      } catch (error: any) {
        console.error('Failed to set flags:', error);
        sendResponse({ success: false, error: error.message });
      }
    })();
    return true; // Keep channel open for async response
  }
  });
}

/**
 * Fetch AX slice by iterating through all frame sessions
 * Returns compact accessibility nodes from all frames
 */
async function fetchAXSlice(maxNodes = 150, viewportOnly = true): Promise<any[]> {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab?.id) {
    throw new Error('No active tab found');
  }

  const tabId = tab.id;
  if (!(await isCDPEnabledForTab(tabId))) {
    throw new Error('CDP disabled for this tab/domain');
  }
  await frameRouter.attachToTab(tabId);
  await extendCDPLease(tabId);

  try {
    const frames = frameRouter.getFrameSessionsForTab(tabId);
    const stitched: any[] = [];

    console.log(`[AXSlice] Processing ${frames.length} frame sessions for tab ${tabId}`);

    for (const { sessionId, frameId } of frames) {
      try {
        console.log(`[AXSlice] Processing frame ${frameId} with session ${sessionId}`);

      // Enable domains for this session
      const { cdpClient } = await import('./cdp/cdpClient');
      await cdpClient.enableDomains({ sessionId }, ['Accessibility', 'DOM']);

      // Get accessibility tree for this frame
      const axResult = await cdpClient.sendCommand<any>({ sessionId }, 'Accessibility.getFullAXTree', {});
      if (!axResult?.nodes) {
        console.warn(`[AXSlice] No AX nodes found for frame ${frameId}`);
        continue;
      }

      // Convert to candidates for processing
      const candidates: Array<{
        role: string;
        name?: string;
        frameId: string;
        backendNodeId: number;
      }> = [];

      for (const node of axResult.nodes) {
        const role = node.role?.value as string | undefined;
        if (!role || role === 'generic' || role === 'none') continue;

        const backendNodeId = node.backendDOMNodeId ?? node.backendNodeId;
        if (!backendNodeId) continue;

        const name = node.name?.value as string | undefined;
        candidates.push({ role, name, frameId, backendNodeId });

        // Stop if we have enough candidates across all frames
        if (stitched.length + candidates.length >= maxNodes * 2) break;
      }

      // Process candidates to compute bounds and filter by viewport
      for (const candidate of candidates) {
        if (stitched.length >= maxNodes) break;

        let bounds: { x: number; y: number; w: number; h: number } | undefined;
        let actionable = false;

        try {
          // Push backend node to frontend to get nodeId
          const pushResult = await cdpClient.sendCommand<any>(
            { sessionId }, 
            'DOM.pushNodesByBackendIdsToFrontend', 
            { backendNodeIds: [candidate.backendNodeId] }
          );

          const nodeId = pushResult?.nodeIds?.[0];
          if (!nodeId) continue;

          // Get box model for bounds
          const boxModel = await cdpClient.sendCommand<any>(
            { sessionId }, 
            'DOM.getBoxModel', 
            { nodeId }
          );

          const quad = boxModel?.model?.content ?? boxModel?.model?.border;
          if (quad && quad.length >= 8) {
            const xs = [quad[0], quad[2], quad[4], quad[6]];
            const ys = [quad[1], quad[3], quad[5], quad[7]];
            const x = (Math.min(...xs) + Math.max(...xs)) / 2;
            const y = (Math.min(...ys) + Math.max(...ys)) / 2;
            const w = Math.abs(Math.max(...xs) - Math.min(...xs));
            const h = Math.abs(Math.max(...ys) - Math.min(...ys));
            bounds = { x, y, w, h };
          }
        } catch (error) {
          // Skip nodes that can't be processed
          console.debug(`[AXSlice] Failed to process node ${candidate.backendNodeId}:`, error);
          continue;
        }

        // Skip offscreen elements if viewport filtering is enabled
        if (viewportOnly && bounds) {
          const inViewport = 
            bounds.y + bounds.h > 0 && 
            bounds.x + bounds.w > 0 &&
            bounds.y < tab.height! && 
            bounds.x < tab.width!;

          if (!inViewport) continue;
        }

        // Determine if element is actionable
        actionable = [
          'button', 'link', 'textbox', 'combobox', 'menuitem', 
          'switch', 'checkbox', 'radio', 'tab', 'option'
        ].includes(candidate.role.toLowerCase());

        stitched.push({
          role: candidate.role,
          name: candidate.name,
          frameId: candidate.frameId,
          backendNodeId: candidate.backendNodeId,
          bounds,
          actionable
        });
      }

    } catch (error) {
      console.warn(`[AXSlice] Error processing frame ${frameId}:`, error);
      // Continue with other frames
    }

    if (stitched.length >= maxNodes) break;
  }

    console.log(`[AXSlice] Collected ${stitched.length} AX nodes from ${frames.length} frames`);
    return stitched.slice(0, maxNodes);
  } finally {
    await forceDetachCDP(tabId).catch(() => {
      console.warn(`Failed to detach CDP from tab ${tabId} after AX slice`);
    });
  }
}

// Export for testing
(globalThis as any).__test_fetchAXSlice = fetchAXSlice;
