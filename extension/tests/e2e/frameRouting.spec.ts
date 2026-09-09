import { chromium, expect, test, type BrowserContext, type Worker } from '@playwright/test';
import http from 'http';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

function startServer(host: string, html: string): Promise<{ url: string; close: () => Promise<void> }> {
  return new Promise(resolve => {
    const server = http.createServer((_req, res) => {
      res.writeHead(200, { 'Content-Type': 'text/html' });
      res.end(html);
    });
    server.listen(0, host, () => {
      const address = server.address();
      const port = typeof address === 'object' && address ? address.port : 0;
      resolve({ url: `http://${host}:${port}/`, close: () => new Promise(done => server.close(() => done())) });
    });
  });
}

async function background(context: BrowserContext): Promise<Worker> {
  let worker = context.serviceWorkers().find(item => item.url().includes('/background.js'));
  if (!worker) worker = await context.waitForEvent('serviceworker', { timeout: 10_000 });
  await expect.poll(() => worker!.evaluate(() =>
    typeof (globalThis as any).__rznTestFrameRoutes === 'function'), { timeout: 10_000 }).toBeTruthy();
  return worker;
}

test('routes main and cross-origin frames through Chrome targets across navigation and close', async () => {
  const child = await startServer('localhost', '<button>Child frame</button>');
  const parent = await startServer('127.0.0.1', `<button>Main frame</button><iframe src="${child.url}"></iframe>`);
  const extensionPath = path.resolve(__dirname, '../../dist/chrome');
  const userDataDir = path.resolve(__dirname, '../../.pw-user-data-frame-routing');
  fs.rmSync(userDataDir, { recursive: true, force: true });
  const context = await chromium.launchPersistentContext(userDataDir, {
    headless: process.env.RZN_PW_HEADFUL !== '1',
    channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
    args: [`--disable-extensions-except=${extensionPath}`, `--load-extension=${extensionPath}`, '--site-per-process'],
  });
  try {
    const page = await context.newPage();
    await page.goto(parent.url);
    await expect.poll(() => page.frames().some(frame => frame.url() === child.url), { timeout: 10_000 }).toBe(true);
    const worker = await background(context);
    const tabId = await worker.evaluate(async prefix => {
      const tabs = await chrome.tabs.query({ url: `${prefix}*` });
      return tabs[0]?.id;
    }, parent.url);
    expect(tabId).toEqual(expect.any(Number));

    await worker.evaluate(id => (globalThis as any).__rznTestFrameRoutes(id), tabId);
    await page.reload();
    await expect.poll(async () => {
      const items = await worker.evaluate(id => (globalThis as any).__rznTestFrameRoutes(id), tabId);
      return items.some((item: any) => item.target.sessionId && item.url === child.url);
    }, { timeout: 15_000 }).toBe(true);
    const routes = await worker.evaluate(id => (globalThis as any).__rznTestFrameRoutes(id), tabId);
    expect(routes).toContainEqual(expect.objectContaining({ target: { tabId }, url: parent.url }));

    await page.goto(`${parent.url}?navigated=1`);
    const afterNavigation = await worker.evaluate(id => (globalThis as any).__rznTestFrameRoutes(id), tabId);
    expect(afterNavigation).toContainEqual(expect.objectContaining({ target: { tabId }, url: `${parent.url}?navigated=1` }));
    await page.close();
    await expect.poll(id => worker.evaluate(tab => (globalThis as any).__rznTestFrameRouterAttached(tab), id), { timeout: 10_000 }).toBe(false);
  } finally {
    await context.close();
    await parent.close();
    await child.close();
  }
});
