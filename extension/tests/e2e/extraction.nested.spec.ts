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
  <title>Nested Cards</title>
  <style>
    #cards { display: grid; grid-template-columns: repeat(3, 1fr); gap: 8px; }
    .card { border: 1px solid #ccc; padding: 8px; }
    .tags { list-style: none; padding: 0; margin: 0; display: flex; gap: 6px; }
  </style>
</head>
<body>
  <div id=\"cards\">
    <div class=\"card\">
      <h3 class=\"title\">Widget Alpha</h3>
      <div class=\"meta\">
        <span class=\"price\">$10.00</span>
      </div>
      <ul class=\"tags\"><li>small</li><li>blue</li></ul>
    </div>
    <div class=\"card\">
      <h3 class=\"title\">Widget Beta</h3>
      <div class=\"meta\">
        <span class=\"price\">$20.50</span>
      </div>
      <ul class=\"tags\"><li>medium</li><li>red</li></ul>
    </div>
  </div>
</body>
</html>
`;

test.describe('Extraction - Nested cards', () => {
  test('extracts title, price, first tag from cards', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
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
      item_selector: '#cards .card',
      fields: [
        { name: 'title', selector: '.title' },
        { name: 'price', selector: '.price' },
        { name: 'first_tag', selector: '.tags li:first-child' }
      ],
    }));

    expect(resp.success).toBeTruthy();
    const items = resp.result as Array<{ title: string; price: string; first_tag: string }>;
    expect(items).toEqual([
      { title: 'Widget Alpha', price: '$10.00', first_tag: 'small' },
      { title: 'Widget Beta', price: '$20.50', first_tag: 'medium' },
    ]);

    await context.close();
    await srv.close();
  });
});

