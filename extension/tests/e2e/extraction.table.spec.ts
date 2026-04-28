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
  <title>Extraction - Table</title>
  <style>
    table { border-collapse: collapse; }
    td, th { border: 1px solid #ccc; padding: 4px; }
  </style>
</head>
<body>
  <h1>Prices</h1>
  <table id=\"prices\">
    <thead>
      <tr><th>Name</th><th>Value</th></tr>
    </thead>
    <tbody>
      <tr><td>Alpha</td><td>$10.00</td></tr>
      <tr><td>Beta</td><td>$20.00</td></tr>
    </tbody>
  </table>
</body>
</html>
`;

test.describe('Extraction - Table rows', () => {
  test('extract_structured_data returns rows with label/value', async () => {
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
      item_selector: '#prices tbody tr',
      fields: [
        { name: 'label', selector: 'td:first-child' },
        { name: 'value', selector: 'td:last-child' },
      ],
    }));

    expect(resp.success).toBeTruthy();
    const rows = resp.result as Array<{ label: string; value: string }>;
    expect(Array.isArray(rows)).toBeTruthy();
    expect(rows).toEqual([
      { label: 'Alpha', value: '$10.00' },
      { label: 'Beta', value: '$20.00' },
    ]);

    await context.close();
    await srv.close();
  });
});
