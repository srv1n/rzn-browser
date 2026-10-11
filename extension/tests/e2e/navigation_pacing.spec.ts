import { chromium, test, expect } from '@playwright/test';
import fs from 'fs';
import http from 'http';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const PAGES: Record<string, string> = {
  '/a': '<title>A</title><a href="/b">b</a>',
  '/b': '<title>B</title><p>page b</p>',
  // A wall that stays: the step must fail with site_challenge.
  '/wall': '<title>Just a moment...</title><p>Checking your browser</p>',
  // Cloudflare-style automatic check that clears itself: the step must pass.
  '/flash': '<title>Just a moment...</title><script>setTimeout(() => location.replace("/b"), 1500)</script>',
};

function startServer(): Promise<{ port: number; close: () => Promise<void> }> {
  return new Promise((resolve) => {
    const server = http.createServer((req, res) => {
      const body = PAGES[(req.url || '').split('?')[0]];
      res.writeHead(body ? 200 : 404, { 'Content-Type': 'text/html' });
      res.end(body || 'missing');
    });
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      resolve({
        port: typeof address === 'object' && address ? address.port : 0,
        close: () => new Promise<void>((done) => server.close(() => done())),
      });
    });
  });
}

test('navigate_to_url stays in the page on the same site and stops on a bot check', async () => {
  test.setTimeout(90_000);
  const extensionPath = path.resolve(__dirname, '../../dist/chrome');
  const userDataDir = path.resolve(__dirname, '../../.pw-user-data-navigation-pacing');
  fs.rmSync(userDataDir, { recursive: true, force: true });

  const context = await chromium.launchPersistentContext(userDataDir, {
    headless: process.env.RZN_PW_HEADFUL !== '1',
    channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
    args: [`--disable-extensions-except=${extensionPath}`, `--load-extension=${extensionPath}`],
  });
  const srv = await startServer();
  const worker =
    context.serviceWorkers().find((w) => w.url().includes('background')) ||
    (await context.waitForEvent('serviceworker'));
  await expect.poll(() => worker.evaluate(() => typeof (globalThis as any).__rznTestHandleBrokerMessage), {
    timeout: 10000,
  }).toBe('function');

  const resp = await worker.evaluate(async ({ port }) => {
    const send = (step: any) =>
      (globalThis as any).__rznTestHandleBrokerMessage({
        cmd: 'execute_step',
        payload: { session_id: 'pw-navigation-pacing', step },
      });
    const local = `http://127.0.0.1:${port}`;
    const referrer = { type: 'eval_main_world', script: 'return document.referrer;' };
    const first = await send({ type: 'navigate_to_url', url: `${local}/a` });
    const inPage = await send({ type: 'navigate_to_url', url: `${local}/b` });
    const inPageReferrer = await send(referrer);
    const crossSite = await send({ type: 'navigate_to_url', url: `http://localhost:${port}/b` });
    const crossSiteReferrer = await send(referrer);
    const flash = await send({ type: 'navigate_to_url', url: `http://localhost:${port}/flash` });
    const wall = await send({ type: 'navigate_to_url', url: `http://localhost:${port}/wall` });
    return { first, inPage, inPageReferrer, crossSite, crossSiteReferrer, flash, wall };
  }, { port: srv.port });

  expect(resp.first?.success).toBeTruthy();
  expect(resp.inPage?.success).toBeTruthy();
  // The reply names the tab, so the CLI can park it for the next run.
  expect(resp.first?.current_tab_id).toBeTruthy();
  expect(resp.inPage?.current_tab_id).toBe(resp.first?.current_tab_id);
  // Same site: the page navigated itself, so the request carries its referrer.
  expect(resp.inPageReferrer?.result?.result).toBe(`http://127.0.0.1:${srv.port}/a`);
  expect(resp.crossSite?.success).toBeTruthy();
  // Another site: a plain browser navigation, no referrer.
  expect(resp.crossSiteReferrer?.result?.result).toBe('');
  expect(resp.flash?.success).toBeTruthy();
  expect(resp.wall?.success).toBeFalsy();
  expect(resp.wall?.error_code).toBe('SITE_CHALLENGE');
  expect(resp.wall?.error_msg).toContain('site_challenge');

  await context.close();
  await srv.close();
});
