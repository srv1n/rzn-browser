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
            { type: 'fill_input_field', selector: '#name', value: 'SessionA', clear_first: true, force_legacy: true },
          ],
          { session_id: 'session-a' },
        ),
        runSteps(
          [
            { type: 'navigate_to_url', url: `${baseUrl}?job=b`, wait: 'domcontentloaded' },
            { type: 'fill_input_field', selector: '#name', value: 'SessionB', clear_first: true, force_legacy: true },
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
