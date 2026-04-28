import { chromium, test, expect } from '@playwright/test';
import fs from 'fs';
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

function readFixture(name: string): string {
  return fs.readFileSync(path.resolve(__dirname, '../fixtures', name), 'utf8');
}

test.describe('Observe → Extract → Act → Extract (local fixtures)', () => {
  test('search-like page: observe list, extract rows, click open, extract detail', async () => {
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
    const srv = await startServer(readFixture('search_like.html'));
    await page.goto(srv.url);

    await page.waitForFunction(() => typeof (window as any).__rznExecuteStep === 'function');

    const obs = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'observe',
      instruction: 'find repeated results',
      max_items: 10,
    }));

    expect(obs.success).toBeTruthy();
    const candidates = obs.result.candidates as Array<{ selector: string; kind: string; score: number }>;
    expect(Array.isArray(candidates)).toBeTruthy();
    expect(candidates.length).toBeGreaterThan(0);
    expect(candidates[0].selector).toContain('.result');

    const itemSelector = candidates[0].selector;

    const extracted = await page.evaluate(async (sel) => (window as any).__rznExecuteStep({
      type: 'execute_extraction_plan',
      plan: {
        version: 1,
        mode: 'list',
        item_selector: sel,
        limit: 10,
        fields: [
          { name: 'title', selector: '.title' },
          { name: 'href', selector: 'a.link', attribute: 'href' },
        ],
      }
    }), itemSelector);

    expect(extracted.success).toBeTruthy();
    expect(extracted.result).toEqual([
      { title: 'Alpha result', href: '#alpha' },
      { title: 'Beta result', href: '#beta' },
      { title: 'Gamma result', href: '#gamma' },
    ]);

    const clickResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'click_element_enhanced',
      selector: '#results .result:nth-child(1) a.link',
    }));
    expect(clickResp.success).toBeTruthy();

    const waitDetail = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'wait_for_element',
      selector: '#detail .detail-title',
      timeout_ms: 5000,
    }));
    expect(waitDetail.success).toBeTruthy();

    const detail = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'execute_extraction_plan',
      plan: {
        version: 1,
        mode: 'single',
        scope: { css: '#detail' },
        fields: [
          { name: 'title', selector: '.detail-title' },
          { name: 'body', selector: '.detail-body' },
          { name: 'open_id', selector: '.open-id', attribute: 'data-open-id' },
        ],
      }
    }));

    expect(detail.success).toBeTruthy();
    expect(detail.result.title).toBe('Alpha result');
    expect(detail.result.open_id).toBe('alpha');

    await context.close();
    await srv.close();
  });

  test('nested table: extraction plan list mode parses rows', async () => {
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
    const srv = await startServer(readFixture('nested_table.html'));
    await page.goto(srv.url);

    await page.waitForFunction(() => typeof (window as any).__rznExecuteStep === 'function');

    const extracted = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'execute_extraction_plan',
      plan: {
        version: 1,
        mode: 'list',
        item_selector: '#products tbody tr',
        fields: [
          { name: 'name', selector: '.name' },
          { name: 'price', selector: '.price' },
        ],
      }
    }));

    expect(extracted.success).toBeTruthy();
    expect(extracted.result).toEqual([
      { name: 'Widget A', price: '$10' },
      { name: 'Widget B', price: '$20' },
    ]);

    await context.close();
    await srv.close();
  });
});
