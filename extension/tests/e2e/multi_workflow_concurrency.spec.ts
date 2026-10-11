import { chromium, test, expect, type BrowserContext, type Worker } from '@playwright/test';
import fs from 'fs';
import http from 'http';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

function startServer(html: string): Promise<{ url: string; close: () => Promise<void> }> {
  return new Promise((resolve) => {
    const server = http.createServer((_req, res) => {
      res.writeHead(200, { 'Content-Type': 'text/html' });
      res.end(html);
    });

    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      const port = typeof address === 'object' && address ? address.port : 0;
      resolve({
        url: `http://127.0.0.1:${port}/`,
        close: () => new Promise<void>((done) => server.close(() => done())),
      });
    });
  });
}

function findBackgroundWorker(context: BrowserContext): Worker | undefined {
  return context.serviceWorkers().find(w => w.url().includes('/background.js'));
}

async function waitForBackgroundHelper(
  context: BrowserContext,
  helperName: string,
  timeout = 30000,
): Promise<Worker> {
  let worker = findBackgroundWorker(context);
  if (!worker) {
    try {
      worker = await context.waitForEvent('serviceworker', { timeout });
    } catch {
      worker = undefined;
    }
  }

  await expect.poll(async () => {
    worker = worker || findBackgroundWorker(context);
    if (!worker) return false;
    try {
      return await worker.evaluate(
        name => typeof (globalThis as any)[name] === 'function',
        helperName,
      );
    } catch {
      return false;
    }
  }, { timeout }).toBeTruthy();

  if (!worker) {
    throw new Error('Missing extension background service worker');
  }
  return worker;
}

const CONCURRENCY_HTML = `
<!doctype html>
<html>
<head>
  <meta charset="utf-8" />
  <title>RZN Multi Workflow Concurrency Fixture</title>
  <style>
    body { font-family: sans-serif; margin: 20px; }
    input { display: block; margin: 12px 0; }
  </style>
  <script>
    window.addEventListener('DOMContentLoaded', () => {
      const keyTarget = document.getElementById('keytarget');
      if (keyTarget) {
        keyTarget.addEventListener('keydown', (e) => {
          if (e.key === 'Enter') {
            keyTarget.setAttribute('data-keyed', 'yes');
          }
        });
      }
    });
  </script>
</head>
<body>
  <h1>Concurrency Fixture</h1>
  <input id="name" placeholder="Name" />
  <input id="keytarget" placeholder="Press Enter via CDP" />
</body>
</html>
`;

test.describe('Multi-workflow concurrency e2e', () => {
  test('native control reconnects after a dead port fails without a disconnect event', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist/chrome');
    const userDataDir = fs.mkdtempSync(path.resolve(__dirname, '../../.pw-user-data-native-reconnect-'));
    let context: BrowserContext | undefined;
    try {
      context = await chromium.launchPersistentContext(userDataDir, {
        headless: process.env.RZN_PW_HEADFUL !== '1',
        channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
        args: [
          `--disable-extensions-except=${extensionPath}`,
          `--load-extension=${extensionPath}`,
        ],
      });
      const worker = await waitForBackgroundHelper(context, '__rznTestCallNativeHostControl');
      const result = await worker.evaluate(async () => {
        const call = (globalThis as any).__rznTestCallNativeHostControl;
        (globalThis as any).__rznTestResetNativeConnection();
        let connections = 0;
        chrome.runtime.connectNative = (() => {
          const connection = ++connections;
          const listeners: Array<(message: any) => void> = [];
          return {
            onMessage: { addListener: (listener: (message: any) => void) => listeners.push(listener) },
            onDisconnect: { addListener: () => {} },
            disconnect: () => {},
            postMessage: (message: any) => {
              if (message.cmd !== 'ping') return;
              if (connection === 1) throw new Error('Attempting to use a disconnected port object');
              queueMicrotask(() => listeners.forEach(listener => listener({
                cmd: 'ping_response', req_id: message.req_id, success: true, result: { pong: true },
              })));
            },
          };
        }) as typeof chrome.runtime.connectNative;
        (globalThis as any).reconnectNative();
        const firstError = await call('ping').then(() => null, (error: Error) => error.message);
        const retryStarted = Date.now();
        const retry = await call('ping');
        return { firstError, retry, connections, retryElapsedMs: Date.now() - retryStarted };
      });
      expect(result.firstError).toContain('disconnected port');
      expect(result.retry).toMatchObject({ success: true, result: { pong: true } });
      expect(result.connections).toBe(2);
      expect(result.retryElapsedMs).toBeLessThan(500);
    } finally {
      await context?.close();
      fs.rmSync(userDataDir, { recursive: true, force: true });
    }
  });

  test('native host recovery probe reconnects after host registration is restored without Retry', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist/chrome');
    const userDataDir = fs.mkdtempSync(path.resolve(__dirname, '../../.pw-user-data-native-host-restore-'));
    let context: BrowserContext | undefined;
    try {
      context = await chromium.launchPersistentContext(userDataDir, {
        headless: process.env.RZN_PW_HEADFUL !== '1',
        channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
        args: [
          `--disable-extensions-except=${extensionPath}`,
          `--load-extension=${extensionPath}`,
        ],
      });
      const worker = await waitForBackgroundHelper(context, '__rznTestSimulateNativePortDisconnect');
      const result = await worker.evaluate(async () => {
        const state = (globalThis as any).__rznTestGetNativeConnectionState;
        (globalThis as any).__rznTestResetNativeConnection();
        let connections = 0;
        let hostRestored = false;
        chrome.runtime.connectNative = (() => {
          connections += 1;
          if (connections > 1 && !hostRestored) throw new Error('Specified native messaging host not found');
          const listeners: Array<(message: any) => void> = [];
          return {
            onMessage: { addListener: (listener: (message: any) => void) => listeners.push(listener) },
            onDisconnect: { addListener: () => {} },
            disconnect: () => {},
            postMessage: (message: any) => {
              if (message.cmd !== 'ping') return;
              queueMicrotask(() => listeners.forEach(listener => listener({
                cmd: 'ping_response', req_id: message.req_id, success: true,
                result: { pong: true, bridge_connected: true },
              })));
            },
          };
        }) as typeof chrome.runtime.connectNative;
        (globalThis as any).reconnectNative();
        const unavailable = (globalThis as any).__rznTestSimulateNativePortDisconnect(
          'Specified native messaging host not found'
        );
        const slowModeBeforeRestore = state().slowMode;
        hostRestored = true;
        await new Promise(resolve => setTimeout(resolve, 1500));
        const connectedByProbe = state().connected;
        if (connectedByProbe) await (globalThis as any).__rznTestRunNativeHeartbeat(1000);
        return { unavailable, slowModeBeforeRestore, connectedByProbe, connections, finalState: state() };
      });
      expect(result.unavailable).toBe(true);
      expect(result.slowModeBeforeRestore).toBe(true);
      expect(result.connectedByProbe).toBe(true);
      expect(result.connections).toBe(2);
      expect(result.finalState).toMatchObject({ connected: true, slowMode: false });
    } finally {
      await context?.close();
      fs.rmSync(userDataDir, { recursive: true, force: true });
    }
  });

  test('health ping responds while workflow and browser identity storage are pending', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist/chrome');
    const userDataDir = fs.mkdtempSync(path.resolve(__dirname, '../../.pw-user-data-ping-storage-'));
    let context: BrowserContext | undefined;
    try {
      context = await chromium.launchPersistentContext(userDataDir, {
        headless: process.env.RZN_PW_HEADFUL !== '1',
        channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
        args: [
          `--disable-extensions-except=${extensionPath}`,
          `--load-extension=${extensionPath}`,
        ],
      });
      const worker = await waitForBackgroundHelper(context, '__rznTestStartBlockedStorageInitialization');
      const result = await worker.evaluate(async () => {
        // Let the worker's own startup reads finish first. Otherwise the stub
        // below captures one of them, nothing ever resolves it, and the
        // evaluation hangs until the test times out.
        await (globalThis as any).__rznTestFinishBlockedStorageInitialization();
        const pendingReads = new Map<string, (value: any) => void>();
        const originalGet = chrome.storage.local.get;
        const originalSet = chrome.storage.local.set;
        chrome.storage.local.get = ((key: string) => new Promise(resolve => {
          pendingReads.set(key, resolve);
        })) as typeof chrome.storage.local.get;
        chrome.storage.local.set = (async (items: Record<string, any>) => {
          void items;
        }) as typeof chrome.storage.local.set;

        let pingPromise: Promise<any> | undefined;
        let initializationStarted = false;
        try {
          (globalThis as any).__rznTestStartBlockedStorageInitialization();
          initializationStarted = true;
          for (let attempt = 0; attempt < 100 && pendingReads.size < 2; attempt += 1) {
            await new Promise(resolve => setTimeout(resolve, 5));
          }
          if (!pendingReads.has('workflow_sessions') || !pendingReads.has('rzn_browser_instance_id')) {
            throw new Error(`Expected both storage reads to be pending; got ${Array.from(pendingReads.keys())}`);
          }

          const startedAt = performance.now();
          pingPromise = (globalThis as any).__rznTestHandleBrokerMessageWithWatchdog({
            cmd: 'ping',
            req_id: 'ping-with-storage-pending',
            timeout_ms: 2000,
          });
          const response = await Promise.race([
            pingPromise,
            new Promise((_resolve, reject) => setTimeout(
              () => reject(new Error('health ping waited on pending storage')),
              500,
            )),
          ]);
          const responseElapsedMs = performance.now() - startedAt;

          pendingReads.get('workflow_sessions')?.({ workflow_sessions: {} });
          pendingReads.get('rzn_browser_instance_id')?.({});
          const initializedId = await (globalThis as any).__rznTestFinishBlockedStorageInitialization();
          const recoveredPing = await (globalThis as any).__rznTestHandleBrokerMessageWithWatchdog({
            cmd: 'ping',
            req_id: 'ping-after-storage-ready',
            timeout_ms: 2000,
          });
          return { response, responseElapsedMs, initializedId, recoveredPing };
        } finally {
          pendingReads.get('workflow_sessions')?.({ workflow_sessions: {} });
          pendingReads.get('rzn_browser_instance_id')?.({});
          if (initializationStarted) {
            await (globalThis as any).__rznTestFinishBlockedStorageInitialization().catch(() => {});
          }
          if (pingPromise) await pingPromise.catch(() => {});
          chrome.storage.local.get = originalGet;
          chrome.storage.local.set = originalSet;
        }
      });

      expect(result.response).toMatchObject({
        success: true,
        result: { pong: true, browser_instance_id: null },
      });
      expect(result.responseElapsedMs).toBeLessThan(500);
      expect(result.initializedId).toMatch(/^[0-9a-f-]{36}$/i);
      expect(result.recoveredPing).toMatchObject({
        success: true,
        result: { pong: true, browser_instance_id: result.initializedId },
      });
    } finally {
      await context?.close();
      fs.rmSync(userDataDir, { recursive: true, force: true });
    }
  });

  test('native control ping misses recycle a host even while stdout heartbeat is fresh', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist/chrome');
    const userDataDir = fs.mkdtempSync(path.resolve(__dirname, '../../.pw-user-data-native-control-health-'));
    let context: BrowserContext | undefined;
    try {
      context = await chromium.launchPersistentContext(userDataDir, {
        headless: process.env.RZN_PW_HEADFUL !== '1',
        channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
        args: [
          `--disable-extensions-except=${extensionPath}`,
          `--load-extension=${extensionPath}`,
        ],
      });
      const worker = await waitForBackgroundHelper(context, '__rznTestRunNativeHeartbeat');
      const result = await worker.evaluate(async () => {
        const state = (globalThis as any).__rznTestGetNativeConnectionState;
        (globalThis as any).__rznTestResetNativeConnection();
        let connections = 0;
        let disconnects = 0;
        chrome.runtime.connectNative = (() => {
          const connection = ++connections;
          const listeners: Array<(message: any) => void> = [];
          return {
            onMessage: { addListener: (listener: (message: any) => void) => listeners.push(listener) },
            onDisconnect: { addListener: () => {} },
            disconnect: () => { disconnects += 1; },
            postMessage: (message: any) => {
              if (message.cmd !== 'ping' || connection === 1) return;
              queueMicrotask(() => listeners.forEach(listener => listener({
                cmd: 'ping_response', req_id: message.req_id, success: true,
                result: { pong: true, bridge_connected: true },
              })));
            },
          };
        }) as typeof chrome.runtime.connectNative;
        (globalThis as any).reconnectNative();
        (globalThis as any).__rznTestEmitNativeStdoutHeartbeat();
        await (globalThis as any).__rznTestRunNativeHeartbeat(1000);
        const firstMiss = state();
        (globalThis as any).__rznTestEmitNativeStdoutHeartbeat();
        await (globalThis as any).__rznTestRunNativeHeartbeat(1000);
        const afterTwoMisses = state();
        const recovered = await (globalThis as any).__rznTestCallNativeHostControl('ping');
        return { firstMiss, afterTwoMisses, recovered, connections, disconnects, finalState: state() };
      });
      expect(result.firstMiss).toMatchObject({ connected: true, missedHeartbeats: 1 });
      expect(result.afterTwoMisses.connected).toBe(false);
      expect(result.disconnects).toBe(1);
      expect(result.recovered).toMatchObject({ success: true, result: { pong: true, bridge_connected: true } });
      expect(result.connections).toBe(2);
      expect(result.finalState.connected).toBe(true);
    } finally {
      await context?.close();
      fs.rmSync(userDataDir, { recursive: true, force: true });
    }
  });

  test('late heartbeat failures from an old port cannot tear down its replacement', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist/chrome');
    const userDataDir = fs.mkdtempSync(path.resolve(__dirname, '../../.pw-user-data-stale-heartbeat-'));
    let context: BrowserContext | undefined;
    try {
      context = await chromium.launchPersistentContext(userDataDir, {
        headless: process.env.RZN_PW_HEADFUL !== '1',
        channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
        args: [
          `--disable-extensions-except=${extensionPath}`,
          `--load-extension=${extensionPath}`,
        ],
      });
      const worker = await waitForBackgroundHelper(context, '__rznTestRunNativeHeartbeat');
      const result = await worker.evaluate(async () => {
        (globalThis as any).__rznTestResetNativeConnection();
        let connections = 0;
        chrome.runtime.connectNative = (() => {
          const connection = ++connections;
          const listeners: Array<(message: any) => void> = [];
          return {
            onMessage: { addListener: (listener: (message: any) => void) => listeners.push(listener) },
            onDisconnect: { addListener: () => {} },
            disconnect: () => {},
            postMessage: (message: any) => {
              if (message.cmd !== 'ping' || connection === 1) return;
              queueMicrotask(() => listeners.forEach(listener => listener({
                cmd: 'ping_response', req_id: message.req_id, success: true,
                result: { pong: true, bridge_connected: true },
              })));
            },
          };
        }) as typeof chrome.runtime.connectNative;
        (globalThis as any).reconnectNative();
        const runHeartbeat = (globalThis as any).__rznTestRunNativeHeartbeat;
        const staleHeartbeats = [runHeartbeat(10_000), runHeartbeat(10_000)];
        await new Promise(resolve => setTimeout(resolve, 0));
        (globalThis as any).__rznTestResetNativeConnection();
        (globalThis as any).reconnectNative();
        await Promise.all(staleHeartbeats);
        const replacementPing = await (globalThis as any).__rznTestCallNativeHostControl('ping');
        return { replacementPing, connections };
      });
      expect(result.replacementPing).toMatchObject({ success: true, result: { pong: true, bridge_connected: true } });
      expect(result.connections).toBe(2);
    } finally {
      await context?.close();
      fs.rmSync(userDataDir, { recursive: true, force: true });
    }
  });

  test('session_close succeeds across repeated cleanup and session reuse', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist/chrome');
    const userDataDir = fs.mkdtempSync(path.resolve(__dirname, '../../.pw-user-data-session-close-'));
    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    try {
      const worker = await waitForBackgroundHelper(context, '__rznTestHandleBrokerMessageWithWatchdog');
      const result = await worker.evaluate(async () => {
        const handle = (globalThis as any).__rznTestHandleBrokerMessageWithWatchdog;
        const sessionId = 'session-close-regression';
        const originalTab = await chrome.tabs.create({ url: 'about:blank', active: false });
        const close = (reqId: string, payload: any = {}) => handle({
          cmd: 'session_close',
          req_id: reqId,
          payload: { session_id: sessionId, ...payload },
        });
        const retained = await close('close-retained', {
          current_tab_id: originalTab.id,
          keep_tab: true,
        });
        const retainedTab = await chrome.tabs.get(originalTab.id!);
        const alreadyClosed = await close('close-unbound');
        const closed = await close('close-reused', { current_tab_id: originalTab.id });
        const removed = await chrome.tabs.get(originalTab.id!).then(() => false, () => true);
        const repeated = await close('close-repeated');

        const missingTab = await chrome.tabs.create({ url: 'about:blank', active: false });
        await chrome.tabs.remove(missingTab.id!);
        const missing = await close('close-missing-tab', { current_tab_id: missingTab.id });

        await (globalThis as any).__rznTestFlushWorkflowSessionPersistence();
        const stored = await chrome.storage.local.get('workflow_sessions');
        return { retained, retainedTabId: retainedTab.id, alreadyClosed, closed, removed, repeated, missing,
          storedSession: stored.workflow_sessions?.[sessionId] ?? null };
      });

      expect(result.retained).toMatchObject({ success: true, result: { tab_closed: false, tab_retained: true } });
      expect(result.retainedTabId).toBe(result.retained.result.tab_id);
      expect(result.alreadyClosed).toMatchObject({ success: true, result: { tab_closed: false } });
      expect(result.closed).toMatchObject({ success: true, result: { tab_closed: true, tab_retained: false } });
      expect(result.removed).toBe(true);
      expect(result.repeated).toMatchObject({ success: true, result: { tab_closed: false } });
      expect(result.missing).toMatchObject({ success: true, result: { tab_closed: false } });
      expect(result.storedSession).toBeNull();
    } finally {
      await context.close();
      fs.rmSync(userDataDir, { recursive: true, force: true });
    }
  });

  test('session_close prunes idle state for many unique sessions', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist/chrome');
    const userDataDir = fs.mkdtempSync(path.resolve(__dirname, '../../.pw-user-data-session-prune-'));
    let context: BrowserContext | undefined;
    try {
      context = await chromium.launchPersistentContext(userDataDir, {
        headless: process.env.RZN_PW_HEADFUL !== '1',
        channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
        args: [
          `--disable-extensions-except=${extensionPath}`,
          `--load-extension=${extensionPath}`,
        ],
      });
      const worker = await waitForBackgroundHelper(context, '__rznTestHasWorkflowSession');
      const leftoverSessions = await worker.evaluate(async () => {
        const close = (globalThis as any).__rznTestHandleBrokerMessageWithWatchdog;
        const hasSession = (globalThis as any).__rznTestHasWorkflowSession;
        const sessionIds = Array.from({ length: 40 }, (_, index) => `prune-regression-${index}`);
        for (const [index, sessionId] of sessionIds.entries()) {
          const response = await close({
            cmd: 'session_close',
            req_id: `prune-close-${index}`,
            payload: { session_id: sessionId },
          }, sessionId);
          if (!response?.success) throw new Error(`session_close failed for ${sessionId}`);
        }
        await new Promise(resolve => setTimeout(resolve, 0));
        return sessionIds.filter(sessionId => hasSession(sessionId));
      });
      expect(leftoverSessions).toEqual([]);
    } finally {
      await context?.close();
      fs.rmSync(userDataDir, { recursive: true, force: true });
    }
  });

  test('late tab metadata lookup cannot resurrect a closed and rebound session', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist/chrome');
    const userDataDir = fs.mkdtempSync(path.resolve(__dirname, '../../.pw-user-data-session-late-refresh-'));
    let context: BrowserContext | undefined;
    try {
      context = await chromium.launchPersistentContext(userDataDir, {
        headless: process.env.RZN_PW_HEADFUL !== '1',
        channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
        args: [
          `--disable-extensions-except=${extensionPath}`,
          `--load-extension=${extensionPath}`,
        ],
      });
      const worker = await waitForBackgroundHelper(context, '__rznTestSetWorkflowTabId');
      const result = await worker.evaluate(async () => {
        const sessionId = 'session-close-late-refresh';
        const oldTab = await chrome.tabs.create({ url: 'about:blank', active: false });
        const newTab = await chrome.tabs.create({ url: 'about:blank', active: false });
        let resolveOldLookup: ((tab: any) => void) | undefined;
        chrome.tabs.get = ((tabId: number) => {
          if (tabId === oldTab.id) {
            return new Promise(resolve => { resolveOldLookup = resolve; });
          }
          return Promise.resolve({ id: tabId, url: 'https://replacement.example/', discarded: false } as any);
        }) as typeof chrome.tabs.get;

        const setTab = (globalThis as any).__rznTestSetWorkflowTabId;
        setTab(sessionId, oldTab.id);
        if (!resolveOldLookup) throw new Error('old tab metadata lookup did not start');

        const closed = await (globalThis as any).__rznTestHandleBrokerMessageWithWatchdog({
          cmd: 'session_close',
          req_id: 'close-late-refresh',
          payload: { session_id: sessionId },
        }, sessionId);
        await new Promise(resolve => setTimeout(resolve, 0));
        const prunedBeforeRebind = !(globalThis as any).__rznTestHasWorkflowSession(sessionId);

        setTab(sessionId, newTab.id);
        await new Promise(resolve => setTimeout(resolve, 0));
        const replacementBeforeLateResult = (globalThis as any).__rznTestGetWorkflowSession(sessionId);
        resolveOldLookup({ id: oldTab.id, url: 'https://stale.example/', discarded: false });
        await new Promise(resolve => setTimeout(resolve, 0));
        await (globalThis as any).__rznTestFlushWorkflowSessionPersistence();
        return {
          closed,
          oldTabId: oldTab.id,
          newTabId: newTab.id,
          prunedBeforeRebind,
          replacementBeforeLateResult,
          finalSession: (globalThis as any).__rznTestGetWorkflowSession(sessionId),
        };
      });
      expect(result.closed.success).toBe(true);
      expect(result.prunedBeforeRebind).toBe(true);
      expect(result.replacementBeforeLateResult).toMatchObject({
        workflowTabId: result.newTabId,
        currentUrl: 'https://replacement.example/',
      });
      expect(result.finalSession).toMatchObject({
        workflowTabId: result.newTabId,
        currentUrl: 'https://replacement.example/',
      });
      expect(result.finalSession.workflowTabId).not.toBe(result.oldTabId);
    } finally {
      await context?.close();
      fs.rmSync(userDataDir, { recursive: true, force: true });
    }
  });

  test('session_close preserves queued work and reuse order before pruning idle state', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist/chrome');
    const userDataDir = fs.mkdtempSync(path.resolve(__dirname, '../../.pw-user-data-session-close-queue-'));
    let context: BrowserContext | undefined;
    try {
      context = await chromium.launchPersistentContext(userDataDir, {
        headless: process.env.RZN_PW_HEADFUL !== '1',
        channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
        args: [
          `--disable-extensions-except=${extensionPath}`,
          `--load-extension=${extensionPath}`,
        ],
      });
      const worker = await waitForBackgroundHelper(context, '__rznTestHandleBrokerMessageWithWatchdog');
      const result = await worker.evaluate(async () => {
        const handle = (globalThis as any).__rznTestHandleBrokerMessageWithWatchdog;
        const sessionId = 'session-close-queued-reuse';
        (globalThis as any).__rznTestLateMutationCount = 0;
        const staleRequest = handle({
          cmd: '__rzn_test_sleep_then_mutate',
          req_id: 'close-queued-stale-request',
          payload: { session_id: sessionId, timeout_ms: 10_000, sleep_ms: 400 },
        }, sessionId);
        await new Promise(resolve => setTimeout(resolve, 50));
        const completionOrder: string[] = [];
        const queuedBeforeClose = handle({
          cmd: '__rzn_test_sleep_then_mutate',
          req_id: 'close-queued-before-close',
          payload: { session_id: sessionId, timeout_ms: 5000, sleep_ms: 250 },
        }, sessionId).then((response: any) => {
          completionOrder.push('queued-before-close');
          return response;
        });
        const closed = await handle({
          cmd: 'session_close',
          req_id: 'close-queued-close',
          payload: { session_id: sessionId },
        }, sessionId);
        const reusedAfterClose = handle({
          cmd: '__rzn_test_sleep_then_mutate',
          req_id: 'close-queued-reuse-after-close',
          payload: { session_id: sessionId, timeout_ms: 5000, sleep_ms: 0 },
        }, sessionId).then((response: any) => {
          completionOrder.push('reuse-after-close');
          return response;
        });
        const [stale, queued, reused] = await Promise.all([staleRequest, queuedBeforeClose, reusedAfterClose]);
        await new Promise(resolve => setTimeout(resolve, 0));
        return {
          closed,
          stale,
          queued,
          reused,
          completionOrder,
          lateMutationCount: Number((globalThis as any).__rznTestLateMutationCount || 0),
          sessionStillPresent: (globalThis as any).__rznTestHasWorkflowSession(sessionId),
        };
      });
      expect(result.closed.success).toBe(true);
      expect(result.stale).toMatchObject({ success: false, error_code: 'STALE_BROKER_LEASE' });
      expect(result.queued).toMatchObject({ success: true, result: { mutated: true } });
      expect(result.reused).toMatchObject({ success: true, result: { mutated: true } });
      expect(result.completionOrder).toEqual(['queued-before-close', 'reuse-after-close']);
      expect(result.lateMutationCount).toBe(2);
      expect(result.sessionStillPresent).toBe(false);
    } finally {
      await context?.close();
      fs.rmSync(userDataDir, { recursive: true, force: true });
    }
  });

  test('runs two sessions concurrently with isolated tab state', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist/chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data-multi-workflow');
    fs.rmSync(userDataDir, { recursive: true, force: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const srv = await startServer(CONCURRENCY_HTML);
    const page = await context.newPage();
    await page.goto(`${srv.url}?bootstrap=1`);
    await page.waitForFunction(() => typeof (window as any).__rznExecuteStep === 'function', { timeout: 10000 });

    const worker = await waitForBackgroundHelper(context, '__rznTestRunWorkflowSteps');

    const jobStates = await worker.evaluate(async ({ baseUrl }) => {
      const runSteps = (globalThis as any).__rznTestRunWorkflowSteps;
      if (typeof runSteps !== 'function') {
        throw new Error('Missing __rznTestRunWorkflowSteps');
      }

      await Promise.all([
        runSteps(
          [
            { type: 'navigate_to_url', url: `${baseUrl}?job=a`, wait: 'domcontentloaded' },
            { type: 'fill_input_field', selector: '#name', value: 'SessionA', clear_first: true },
          ],
          { session_id: 'session-a' },
        ),
        runSteps(
          [
            { type: 'navigate_to_url', url: `${baseUrl}?job=b`, wait: 'domcontentloaded' },
            { type: 'fill_input_field', selector: '#name', value: 'SessionB', clear_first: true },
          ],
          { session_id: 'session-b' },
        ),
      ]);

      await new Promise(resolve => setTimeout(resolve, 300));

      const tabs = await chrome.tabs.query({});
      const workflowTabs = tabs.filter(
        (tab) => typeof tab.url === 'string' && tab.url.startsWith(baseUrl) && tab.url.includes('job='),
      );

      const states: Array<{ search: string; value: string; tabId: number }> = [];
      for (const tab of workflowTabs) {
        if (typeof tab.id !== 'number') continue;
        const [injected] = await chrome.scripting.executeScript({
          target: { tabId: tab.id },
          func: () => {
            const input = document.getElementById('name') as HTMLInputElement | null;
            return {
              search: window.location.search,
              value: input?.value || '',
            };
          },
        });
        const result = injected?.result as { search?: string; value?: string } | undefined;
        states.push({
          search: result?.search || '',
          value: result?.value || '',
          tabId: tab.id,
        });
      }

      return states;
    }, { baseUrl: srv.url });

    const bySearch = new Map<string, string>();
    for (const item of jobStates) {
      bySearch.set(item.search, item.value);
    }

    expect(bySearch.get('?job=a')).toBe('SessionA');
    expect(bySearch.get('?job=b')).toBe('SessionB');

    await context.close();
    await srv.close();
  });

  test('queues concurrent CDP broker commands without cross-session failures', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist/chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data-cdp-queue');
    fs.rmSync(userDataDir, { recursive: true, force: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const srv = await startServer(CONCURRENCY_HTML);
    const page = await context.newPage();
    await page.goto(`${srv.url}?bootstrap=cdp`);
    await page.waitForFunction(() => typeof (window as any).__rznExecuteStep === 'function', { timeout: 10000 });

    await waitForBackgroundHelper(context, '__rznTestRunWorkflowSteps');
    const worker = await waitForBackgroundHelper(context, '__rznTestHandleBrokerMessage');

    const responses = await worker.evaluate(async ({ baseUrl }) => {
      const runSteps = (globalThis as any).__rznTestRunWorkflowSteps;
      const handleBroker = (globalThis as any).__rznTestHandleBrokerMessage;
      if (typeof runSteps !== 'function' || typeof handleBroker !== 'function') {
        throw new Error('Missing test helpers');
      }

      await Promise.all([
        runSteps([{ type: 'navigate_to_url', url: `${baseUrl}?cdp=a`, wait: 'domcontentloaded' }], { session_id: 'cdp-a' }),
        runSteps([{ type: 'navigate_to_url', url: `${baseUrl}?cdp=b`, wait: 'domcontentloaded' }], { session_id: 'cdp-b' }),
      ]);

      const [enableA, enableB] = await Promise.all([
        handleBroker({ cmd: 'enable_debug', req_id: 'enable-a', payload: { ttl_ms: 10000 }, data: { session_id: 'cdp-a' } }),
        handleBroker({ cmd: 'enable_debug', req_id: 'enable-b', payload: { ttl_ms: 10000 }, data: { session_id: 'cdp-b' } }),
      ]);

      const [contextA, contextB] = await Promise.all([
        handleBroker({
          cmd: 'get_cdp_context',
          req_id: 'ctx-a',
          payload: { options: { includeAccessibility: false, includeStyles: false } },
          data: { session_id: 'cdp-a' },
        }),
        handleBroker({
          cmd: 'get_cdp_context',
          req_id: 'ctx-b',
          payload: { options: { includeAccessibility: false, includeStyles: false } },
          data: { session_id: 'cdp-b' },
        }),
      ]);

      await Promise.all([
        handleBroker({ cmd: 'disable_debug', req_id: 'disable-a', data: { session_id: 'cdp-a' } }),
        handleBroker({ cmd: 'disable_debug', req_id: 'disable-b', data: { session_id: 'cdp-b' } }),
      ]);

      return { enableA, enableB, contextA, contextB };
    }, { baseUrl: srv.url });

    expect(responses.enableA?.success).toBeTruthy();
    expect(responses.enableB?.success).toBeTruthy();
    expect(responses.contextA?.success).toBeTruthy();
    expect(responses.contextB?.success).toBeTruthy();
    expect(responses.contextA?.result).toBeTruthy();
    expect(responses.contextB?.result).toBeTruthy();

    await context.close();
    await srv.close();
  });

  test('watchdog releases same-session queue before timed-out handler unwinds', async () => {
    test.setTimeout(120000);
    const extensionPath = path.resolve(__dirname, '../../dist/chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data-watchdog-queue');
    fs.rmSync(userDataDir, { recursive: true, force: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    let srv: Awaited<ReturnType<typeof startServer>> | null = null;
    try {
      srv = await startServer(CONCURRENCY_HTML);
      const page = await context.newPage();
      await page.goto(`${srv.url}?bootstrap=watchdog`);
      await page.waitForFunction(() => typeof (window as any).__rznExecuteStep === 'function', { timeout: 30000 });

      const worker = await waitForBackgroundHelper(
        context,
        '__rznTestHandleBrokerMessageWithWatchdog',
        30000,
      );

      await worker.evaluate(() => {
        (globalThis as any).__rznTestLateMutationCount = 0;
        (globalThis as any).__rznTestFirstWatchdogResult = null;
        (globalThis as any).__rznTestNativeControlPlaneHealthy = true;
        const sessionId = 'watchdog-queue-session';
        const handle = (globalThis as any).__rznTestHandleBrokerMessageWithWatchdog;
        if (typeof handle !== 'function') {
          throw new Error('Missing __rznTestHandleBrokerMessageWithWatchdog');
        }
        void handle({
          cmd: '__rzn_test_sleep_then_mutate',
          req_id: 'watchdog-zombie-a',
          payload: {
            session_id: sessionId,
            timeout_ms: 1200,
            sleep_ms: 3000,
          },
        }, sessionId)
          .then((response: any) => {
            (globalThis as any).__rznTestFirstWatchdogResult = { response };
          })
          .catch((error: any) => {
            (globalThis as any).__rznTestFirstWatchdogResult = {
              error: error?.message || String(error),
            };
          });
      });

      await new Promise(resolve => setTimeout(resolve, 50));
      const secondStartedAt = Date.now();
      await worker.evaluate(() => {
        (globalThis as any).__rznTestSecondWatchdogResult = null;
        const handle = (globalThis as any).__rznTestHandleBrokerMessageWithWatchdog;
        if (typeof handle !== 'function') {
          throw new Error('Missing __rznTestHandleBrokerMessageWithWatchdog');
        }
        void handle({
          cmd: 'ping',
          req_id: 'watchdog-next-b',
          payload: {
            session_id: 'watchdog-queue-session',
            timeout_ms: 5000,
          },
        }, 'watchdog-queue-session')
          .then((response: any) => {
            (globalThis as any).__rznTestSecondWatchdogResult = { response };
          })
          .catch((error: any) => {
            (globalThis as any).__rznTestSecondWatchdogResult = {
              error: error?.message || String(error),
            };
          });
      });
      const second = await (async () => {
        const deadline = Date.now() + 5000;
        while (Date.now() < deadline) {
          const result = await worker.evaluate(() => (globalThis as any).__rznTestSecondWatchdogResult || null);
          if (result) return { timedOut: false, ...result } as any;
          await new Promise(resolve => setTimeout(resolve, 100));
        }
        return { timedOut: true } as any;
      })();
      const secondElapsedMs = Date.now() - secondStartedAt;

      const firstResponse = await (async () => {
        const deadline = Date.now() + 5000;
        while (Date.now() < deadline) {
          const result = await worker.evaluate(() => (globalThis as any).__rznTestFirstWatchdogResult || null);
          if (result) return { timedOut: false, ...result } as any;
          await new Promise(resolve => setTimeout(resolve, 100));
        }
        return { timedOut: true } as any;
      })();

      const postWatchdogHealth = await (async () => {
        const deadline = Date.now() + 5000;
        while (Date.now() < deadline) {
          const response = await worker.evaluate(async () => {
            const handle = (globalThis as any).__rznTestHandleBrokerMessageWithWatchdog;
            return await handle({
              cmd: 'ping',
              req_id: `watchdog-health-${Date.now()}`,
              payload: {
                session_id: 'watchdog-queue-session',
                timeout_ms: 1000,
              },
            }, 'watchdog-queue-session');
          });
          if (Number(response?.result?.broker_watchdog_quarantine_count || 0) >= 1) {
            return response;
          }
          await new Promise(resolve => setTimeout(resolve, 100));
        }
        return null;
      })();

      await new Promise(resolve => setTimeout(resolve, 2300));
      const lateMutationCount = await worker.evaluate(() =>
        Number((globalThis as any).__rznTestLateMutationCount || 0)
      );

      const result = {
        first: firstResponse.response,
        firstTimedOut: firstResponse.timedOut,
        firstError: firstResponse.error,
        second: second.response,
        secondTimedOut: second.timedOut,
        secondElapsedMs,
        postWatchdogHealth,
        lateMutationCount,
      };

      expect(result.firstTimedOut).toBe(false);
      expect(result.secondTimedOut).toBe(false);
      expect(result.first?.success).toBe(false);
      expect(result.first?.error_code).toBe('BROKER_WATCHDOG_TIMEOUT');
      expect(result.second?.success).toBe(true);
      expect(result.second?.result?.capabilities?.control_plane_queue_bypass).toBe(true);
      expect(result.secondElapsedMs).toBeLessThan(1000);
      expect(result.postWatchdogHealth?.success).toBe(true);
      expect(result.postWatchdogHealth?.result?.broker_watchdog_quarantine_count).toBeGreaterThanOrEqual(1);
      expect(result.postWatchdogHealth?.result?.broker_watchdog_bridge_restart_count).toBe(0);
      expect(result.lateMutationCount).toBe(0);

    } finally {
      await context.close().catch(() => {});
      fs.rmSync(userDataDir, { recursive: true, force: true });
      if (srv) {
        await srv.close().catch(() => {});
      }
    }
  });
});
