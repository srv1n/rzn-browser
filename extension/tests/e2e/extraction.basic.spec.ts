import { chromium, test, expect } from '@playwright/test';
import http from 'http';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

function startServer(html: string): Promise<{ url: string; close: () => Promise<void> }> {
  return new Promise((resolve) => {
    const server = http.createServer((req, res) => {
      res.writeHead(200, { 'Content-Type': 'text/html' });
      res.end(html);
    });
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      const port = typeof address === 'object' && address ? address.port : 0;
      resolve({
        url: `http://127.0.0.1:${port}/`,
        close: () => new Promise<void>((r) => server.close(() => r())),
      });
    });
  });
}

const TEST_HTML = `
<!doctype html>
<html>
<head>
  <meta charset="utf-8" />
  <title>Extraction - Basic List</title>
  <style>
    .item { padding: 4px; }
  </style>
</head>
<body>
  <h1>Products</h1>
  <div id="list">
    <div class="item"><span class="title">Alpha</span><a class="link" href="/alpha">open</a></div>
    <div class="item"><span class="title">Beta</span><a class="link" href="/beta">open</a></div>
    <div class="item"><span class="title">Gamma</span><a class="link" href="/gamma">open</a></div>
  </div>
</body>
</html>
`;

test.describe('Extraction - Basic single-layer list', () => {
  test('extract_structured_data returns array of objects', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist/chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data');

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const page = await context.newPage();
    const srv = await startServer(TEST_HTML);
    await page.goto(srv.url);

    await page.waitForFunction(() => typeof (window as any).__rznExecuteStep === 'function');

    const resp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'extract_structured_data',
      force_legacy: true,
      item_selector: '#list .item',
      fields: [
        { name: 'title', selector: '.title' },
        { name: 'url', selector: '.link', attribute: 'href' },
      ],
    }));

    expect(resp.success).toBeTruthy();
    const items = resp.result as Array<{ title: string; url: string }>;
    expect(Array.isArray(items)).toBeTruthy();
    expect(items.length).toBe(3);
    expect(items[0]).toEqual({ title: 'Alpha', url: '/alpha' });
    expect(items[1]).toEqual({ title: 'Beta', url: '/beta' });
    expect(items[2]).toEqual({ title: 'Gamma', url: '/gamma' });

    await context.close();
    await srv.close();
  });
});
