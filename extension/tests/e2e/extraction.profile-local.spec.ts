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

// Local page structured for deterministic extraction plan tests.
const TEST_HTML = `
<!doctype html>
<html>
<head>
  <meta charset=\"utf-8\" />
  <title>Profile-backed search</title>
  <style>
    #test-search { padding: 8px; }
    .result { margin: 8px 0; }
  </style>
</head>
<body>
  <div id=\"test-search\">
    <div class=\"result\">
      <h2 class=\"title\">Alpha Phone</h2>
      <a class=\"link\" href=\"/alpha\">visit</a>
      <p class=\"snippet\">Alpha snippet here</p>
      <span class=\"price\">$199</span>
      <span class=\"rating\">4.3</span>
    </div>
    <div class=\"result\">
      <h2 class=\"title\">Beta Phone</h2>
      <a class=\"link\" href=\"/beta\">visit</a>
      <p class=\"snippet\">Beta snippet here</p>
      <span class=\"price\">$149</span>
      <span class=\"rating\">4.0</span>
    </div>
  </div>
</body>
</html>
`;

test.describe('Validated extraction plan (local fixture)', () => {
  test('extracts list items deterministically', async () => {
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
      type: 'execute_extraction_plan',
      version: 1,
      mode: 'list',
      scope: { css: '#test-search' },
      item_selector: '.result',
      limit: 10,
      fields: [
        { name: 'title', selector: '.title' },
        { name: 'url', selector: '.link', attribute: 'href' },
        { name: 'snippet', selector: '.snippet' },
        { name: 'price', selector: '.price' },
        { name: 'rating', selector: '.rating' }
      ]
    }));

    expect(resp.success).toBeTruthy();
    const items = resp.result as Array<{ title: string; url: string; price: string; rating: string; snippet: string }>;
    expect(Array.isArray(items)).toBeTruthy();
    expect(items.length).toBe(2);
    expect(items[0].title).toContain('Alpha');
    expect(items[0].url).toMatch(/\/alpha$/);
    expect(items[0].price).toContain('$');
    expect(items[0].rating).toMatch(/\d/);
    expect(items[0].snippet).toContain('Alpha');

    await context.close();
    await srv.close();
  });
});
