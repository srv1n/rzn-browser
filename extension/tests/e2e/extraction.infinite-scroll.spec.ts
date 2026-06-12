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
  <title>Infinite Scroll Feed</title>
  <style>
    body { margin: 0; }
    #feed { padding: 16px; }
    .item { padding: 6px; border-bottom: 1px solid #eee; }
    .spacer { height: 1000px; }
  </style>
  <script>
    let batchesAppended = 0;
    const BATCH_SIZE = 5;
    const MAX_BATCHES = 3;
    let nextIndex = 0;

    function appendBatch() {
      const feed = document.getElementById('feed');
      for (let i = 0; i < BATCH_SIZE; i++) {
        const div = document.createElement('div');
        div.className = 'item';
        div.setAttribute('data-index', String(nextIndex));
        div.textContent = 'Item ' + nextIndex;
        feed.appendChild(div);
        nextIndex++;
      }
      batchesAppended++;
    }

    window.addEventListener('DOMContentLoaded', () => {
      // Initial items
      appendBatch(); // 0..4
      // Add spacer to enable scrolling
      const spacer = document.createElement('div');
      spacer.className = 'spacer';
      document.body.appendChild(spacer);
    });

    window.addEventListener('scroll', () => {
      const nearBottom = window.innerHeight + window.scrollY >= document.body.scrollHeight - 2;
      if (nearBottom && batchesAppended <= MAX_BATCHES) {
        appendBatch();
      }
    });
  </script>
</head>
<body>
  <div id=\"feed\"></div>
</body>
</html>
`;

test.describe('Extraction - Infinite scroll list', () => {
  test('scrolls to load more and extracts all items', async () => {
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

    // Scroll to load up to 3 more batches (total up to 4 batches including initial)
    const scrollResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'infinite_scroll',
      max_scrolls: 3,
      scroll_delay: 250,
      target_selector: '#feed .item',
      target_count: 20
    }));
    expect(scrollResp.success ?? true).toBeTruthy();

    // Now extract all items
    const extractResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'extract_structured_data',
      force_legacy: true,
      item_selector: '#feed .item',
      fields: [
        { name: 'text', selector: '*' },
        { name: 'index', selector: '*', attribute: 'data-index' },
      ],
    }));

    expect(extractResp.success).toBeTruthy();
    const items = extractResp.result as Array<{ text: string; index: string }>;
    expect(Array.isArray(items)).toBeTruthy();
    // Expect initial 5 + up to 3 batches (5 each) = 20 items
    expect(items.length).toBe(20);
    expect(items.some(i => i.text === 'Item 0')).toBeTruthy();
    expect(items.some(i => i.text === 'Item 10')).toBeTruthy();

    await context.close();
    await srv.close();
  });
});
