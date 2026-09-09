import { chromium, test, expect } from '@playwright/test';
import http from 'http';
import path from 'path';
import { fileURLToPath } from 'url';
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

function startServer(): Promise<{ url: string; close: () => Promise<void> }> {
  return new Promise(resolve => {
    const server = http.createServer((_req, res) => {
      res.writeHead(200, { 'Content-Type': 'text/html' });
      res.end(`<main>${Array.from({ length: 80 }, (_, i) => `<button>Button ${i}</button>`).join('')}</main>`);
    });
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      const port = typeof address === 'object' && address ? address.port : 0;
      resolve({ url: `http://127.0.0.1:${port}/`, close: () => new Promise(done => server.close(() => done())) });
    });
  });
}

test.describe('Extension content script', () => {
  test('injects captureEnhancedDOMSnapshot on pages', async () => {
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
    const server = await startServer();
    await page.goto(server.url);

    // Wait for content script to expose capture function
    await page.waitForFunction(() => typeof (window as any).captureEnhancedDOMSnapshot === 'function', { timeout: 5000 });
    const hasCapture = await page.evaluate(() => typeof (window as any).captureEnhancedDOMSnapshot === 'function');
    expect(hasCapture).toBeTruthy();

    const [small, large] = await page.evaluate(() => Promise.all([
      (window as any).captureEnhancedDOMSnapshot({ maxElements: 20 }),
      (window as any).captureEnhancedDOMSnapshot({ maxElements: 80 }),
    ]));
    expect(small.elements).toHaveLength(20);
    expect(large.elements).toHaveLength(80);
    expect(typeof large.hash).toBe('string');

    await context.close();
    await server.close();
  });
});
