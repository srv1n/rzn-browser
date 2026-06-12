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

// A search-like page with a clear list structure under #search.
const TEST_HTML = `
<!doctype html>
<html>
<head>
  <meta charset=\"utf-8\" />
  <title>Extraction - Search-like</title>
  <style>
    .result { margin: 8px 0; }
  </style>
</head>
<body>
  <div id=\"search\">
    <div class=\"result\"><h3>Alpha Result</h3><div><a href=\"/alpha\">Alpha Link</a></div><p class=\"VwiC3b\">Alpha snippet here</p></div>
    <div class=\"result\"><h3>Beta Result</h3><div><a href=\"/beta\">Beta Link</a></div><p class=\"VwiC3b\">Beta snippet here</p></div>
  </div>
</body>
</html>
`;

test.describe('Extraction - Search-like page', () => {
  test('extracts results with an explicit item selector', async () => {
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
      item_selector: '#search .result',
      fields: [
        { name: 'title', selector: 'h3' },
        { name: 'url', selector: 'a', attribute: 'href' },
        { name: 'snippet', selector: 'p' }
      ],
    }));

    expect(resp.success).toBeTruthy();
    const items = resp.result as Array<{ title: string; url: string; snippet: string }>;
    expect(Array.isArray(items)).toBeTruthy();
    expect(items.length).toBe(2);
    // Titles may be exact as in h3 text
    expect(items[0].title).toContain('Alpha');
    expect(items[0].url).toMatch(/\/alpha$/);
    expect(items[0].snippet).toContain('Alpha');
    expect(items[1].title).toContain('Beta');
    expect(items[1].url).toMatch(/\/beta$/);
    expect(items[1].snippet).toContain('Beta');

    await context.close();
    await srv.close();
  });
});
