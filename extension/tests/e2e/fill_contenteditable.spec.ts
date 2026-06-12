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
    <title>Contenteditable Fill Test</title>
    <style>
      body { font-family: sans-serif; padding: 16px; }
      #editor {
        border: 1px solid #ccc;
        min-height: 80px;
        padding: 8px;
        border-radius: 6px;
      }
      #submit[disabled] { opacity: 0.5; }
    </style>
    <script>
      window.addEventListener('DOMContentLoaded', () => {
        const editor = document.getElementById('editor');
        const submit = document.getElementById('submit');
        const status = document.getElementById('status');
        window.__editorInputCount = 0;

        editor.addEventListener('input', () => {
          window.__editorInputCount += 1;
          const txt = (editor.textContent || '').trim();
          submit.disabled = txt.length === 0;
        });

        submit.addEventListener('click', (e) => {
          e.preventDefault();
          status.textContent = 'submitted:' + (editor.textContent || '');
        });
      });
    </script>
  </head>
  <body>
    <h1>Contenteditable Fill Test</h1>
    <div id="editor" contenteditable="true" role="textbox" aria-label="Add a comment"></div>
    <button id="submit" disabled>Comment</button>
    <div id="status"></div>
  </body>
</html>
`;

const DRAFT_LIKE_HTML = `
<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <title>Draft-Like Contenteditable Fill Test</title>
    <style>
      body { font-family: sans-serif; padding: 16px; }
      #editor {
        border: 1px solid #ccc;
        min-height: 80px;
        padding: 8px;
        border-radius: 6px;
      }
      #submit[disabled] { opacity: 0.5; }
    </style>
    <script>
      window.addEventListener('DOMContentLoaded', () => {
        const editor = document.getElementById('editor');
        const submit = document.getElementById('submit');
        const status = document.getElementById('status');
        window.__editorInputCount = 0;

        editor.addEventListener('input', () => {
          window.__editorInputCount += 1;
          const txt = (editor.textContent || '').trim();
          submit.disabled = txt.length === 0;
        });

        submit.addEventListener('click', (e) => {
          e.preventDefault();
          status.textContent = 'submitted:' + (editor.textContent || '');
        });
      });
    </script>
  </head>
  <body>
    <h1>Draft-Like Contenteditable Fill Test</h1>
    <div id="editor" contenteditable="true" role="textbox" aria-label="Post text" data-testid="tweetTextarea_0">
      <div data-contents="true">
        <div data-block="true" data-offset-key="abc-0-0">
          <div data-offset-key="abc-0-0">
            <span data-offset-key="abc-0-0"><br data-text="true"></span>
          </div>
        </div>
      </div>
    </div>
    <button id="submit" disabled>Reply</button>
    <div id="status"></div>
  </body>
</html>
`;

test.describe('Static fill_input_field contenteditable e2e', () => {
  test('fills a contenteditable textbox and enables submit', async () => {
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

    await page.waitForFunction(() => typeof (window as any).__rznExecuteStep === 'function', { timeout: 10000 });

    const fillResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'fill_input_field',
      selector: '#editor',
      value: 'that is just great',
      force_legacy: true,
    }));
    expect(fillResp.success).toBeTruthy();

    await expect(page.locator('#editor')).toHaveText('that is just great');
    await expect(page.locator('#submit')).toBeEnabled();
    await expect.poll(() => page.evaluate(() => (window as any).__editorInputCount)).toBe(1);

    const clickResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'click_element',
      selector: '#submit',
      force_legacy: true,
    }));
    expect(clickResp.success).toBeTruthy();

    await expect(page.locator('#status')).toHaveText('submitted:that is just great');

    await context.close();
    await srv.close();
  });

  test('fills a draft-like contenteditable without flattening its structure', async () => {
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
    const srv = await startServer(DRAFT_LIKE_HTML);
    await page.goto(srv.url);

    await page.waitForFunction(() => typeof (window as any).__rznExecuteStep === 'function', { timeout: 10000 });

    const fillResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'fill_input_field',
      selector: '#editor',
      value: 'hello world',
      force_legacy: true,
    }));
    expect(fillResp.success).toBeTruthy();

    await expect(page.locator('#editor')).toContainText('hello world');
    await expect(page.locator('#editor [data-contents="true"]')).toHaveCount(1);
    await expect(page.locator('#submit')).toBeEnabled();
    await expect.poll(() => page.evaluate(() => (window as any).__editorInputCount)).toBeGreaterThan(0);

    await context.close();
    await srv.close();
  });

  test('type_text inserts a single printable character into a draft-like contenteditable', async () => {
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
    const srv = await startServer(DRAFT_LIKE_HTML);
    await page.goto(srv.url);

    await page.waitForFunction(() => typeof (window as any).__rznExecuteStep === 'function', { timeout: 10000 });
    await page.locator('#editor').click();

    const typeResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'type_text',
      selector: '#editor',
      text: 'x',
    }));
    expect(typeResp.success).toBeTruthy();

    await expect.poll(() => page.evaluate(() => {
      const editor = document.querySelector('#editor');
      return (editor?.textContent || '').replace(/\u00a0/g, ' ').replace(/\s+/g, ' ').trim();
    })).toBe('x');
    await expect(page.locator('#submit')).toBeEnabled();
    await expect.poll(() => page.evaluate(() => (window as any).__editorInputCount)).toBe(1);

    await context.close();
    await srv.close();
  });
});
