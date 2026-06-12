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
  <title>Enhanced Object Extraction</title>
  <style>
    #card { padding: 8px; border: 1px solid #ccc; width: 320px; }
    .title { font-weight: bold; }
    .price { color: #090; }
  </style>
</head>
<body>
  <div id=\"card\">
    <h2 class=\"title\">Widget Ultra</h2>
    <span class=\"price\" data-currency=\"USD\">$12.34</span>
  </div>
</body>
</html>
`;

test.describe('Enhanced extraction (single object)', () => {
  test('extract_structured_data_enhanced returns an object with fields', async () => {
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
      type: 'extract_structured_data_enhanced',
      selector: '#card',
      fields: [
        { name: 'title', selector: '.title' },
        { name: 'price', selector: '.price' },
        { name: 'currency', selector: '.price', attribute: 'data-currency' },
      ],
    }));

    expect(resp.success).toBeTruthy();
    const obj = resp.result as { title: string; price: string; currency: string };
    expect(typeof obj).toBe('object');
    expect(obj.title).toBe('Widget Ultra');
    expect(obj.price).toBe('$12.34');
    expect(obj.currency).toBe('USD');

    await context.close();
    await srv.close();
  });
});

