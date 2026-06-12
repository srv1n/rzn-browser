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
  <meta charset=\"utf-8\" />
  <title>Observe Hybrid</title>
  <style>
    #cards { display: grid; grid-template-columns: repeat(2, 1fr); gap: 8px; }
    .card { border: 1px solid #ccc; padding: 8px; }
  </style>
</head>
<body>
  <div id=\"cards\">
    <div class=\"card\"><h3 class=\"title\">Alpha</h3><span class=\"price\">$10</span></div>
    <div class=\"card\"><h3 class=\"title\">Beta</h3><span class=\"price\">$20</span></div>
  </div>
</body>
</html>
`;

test.describe('Observe → Extract hybrid', () => {
  test('finds item selector and extracts fields', async () => {
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

    const obs = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'observe',
      instruction: 'find product cards',
      scope_selector: '#cards',
      max_items: 3
    }));

    expect(obs.success).toBeTruthy();
    const cands = obs.result.candidates as Array<{ selector: string; kind: string; score: number }>;
    expect(Array.isArray(cands)).toBeTruthy();
    const best = cands[0];
    expect(best.selector).toContain('.card');

    const extractResp = await page.evaluate((sel) => (window as any).__rznExecuteStep({
      type: 'extract_structured_data',
      force_legacy: true,
      item_selector: sel,
      fields: [
        { name: 'title', selector: '.title' },
        { name: 'price', selector: '.price' }
      ],
    }), best.selector);

    expect(extractResp.success).toBeTruthy();
    const items = extractResp.result as Array<{ title: string; price: string }>;
    expect(items).toEqual([
      { title: 'Alpha', price: '$10' },
      { title: 'Beta', price: '$20' },
    ]);

    await context.close();
    await srv.close();
  });
});

