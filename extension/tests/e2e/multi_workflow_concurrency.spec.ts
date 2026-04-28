import { chromium, test, expect } from '@playwright/test';
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
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
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

    const worker =
      context.serviceWorkers().find(w => w.url().includes('background')) ||
      (await context.waitForEvent('serviceworker'));

    await expect.poll(async () => {
      return worker.evaluate(() => typeof (globalThis as any).__rznTestRunWorkflowSteps === 'function');
    }, { timeout: 10000 }).toBeTruthy();

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
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
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

    const worker =
      context.serviceWorkers().find(w => w.url().includes('background')) ||
      (await context.waitForEvent('serviceworker'));

    await expect.poll(async () => {
      return worker.evaluate(() => ({
        runWorkflow: typeof (globalThis as any).__rznTestRunWorkflowSteps === 'function',
        handleBroker: typeof (globalThis as any).__rznTestHandleBrokerMessage === 'function',
      }));
    }, { timeout: 10000 }).toMatchObject({ runWorkflow: true, handleBroker: true });

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
});
