import { chromium, test, expect } from '@playwright/test';
import fs from 'fs';
import http from 'http';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const CONTENT_SCRIPT_PROTOCOL_PATTERN = /^rzn-cs-\d{4}-\d{2}-\d{2}-\d+$/;

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
  <title>RZN Phase 2 Test Page</title>
  <style>
    body { font-family: sans-serif; }
    #spacer { height: 2000px; background: linear-gradient(#fff, #eee); }
    #bottom { margin-top: 40px; padding: 8px; background: #def; }
    .item { padding: 4px; }
  </style>
  <script>
    window.addEventListener('DOMContentLoaded', () => {
      // Insert delayed element
      setTimeout(() => {
        const d = document.createElement('div');
        d.id = 'delayed';
        d.textContent = 'I appeared later!';
        document.body.appendChild(d);
      }, 400);

      // Click handler
      const btn = document.getElementById('go');
      if (btn) {
        btn.addEventListener('click', () => {
          document.getElementById('status').textContent = 'clicked';
        });
      }

      // Key handler
      const keyTarget = document.getElementById('keytarget');
      if (keyTarget) {
        keyTarget.addEventListener('keydown', (e) => {
          if (e.key === 'Enter') {
            keyTarget.setAttribute('data-keyed', 'yes');
          }
        });
      }
    });
  </script>
  </head>
  <body>
    <h1>RZN Actions Test</h1>
    <input id="name" placeholder="Your name" />
    <button id="go">Go</button>
    <div id="status"></div>

    <input id="keytarget" placeholder="Press Enter here" />

    <div id="spacer"></div>
    <div id="bottom">Bottom element</div>

    <div id="list">
      <div class="item"><span class="title">Alpha</span><a class="link" href="/alpha">A</a></div>
      <div class="item"><span class="title">Beta</span><a class="link" href="/beta">B</a></div>
      <div class="item"><span class="title">Gamma</span><a class="link" href="/gamma">C</a></div>
    </div>
  </body>
  </html>
`;

const DEBUG_HTML = `
<!doctype html>
<html>
<head>
  <meta charset="utf-8" />
  <title>RZN Debug Tools Fixture</title>
  <style>
    body { font-family: sans-serif; }
    #panel[hidden] { display: none; }
    #panel {
      position: fixed;
      right: 16px;
      bottom: 16px;
      width: 280px;
      padding: 12px;
      border: 1px solid #ccc;
      background: white;
      box-shadow: 0 4px 12px rgba(0, 0, 0, 0.18);
    }
  </style>
  <script>
    window.__mainWorldState = { secret: 'main-ready' };
    window.addEventListener('DOMContentLoaded', () => {
      const toggle = document.getElementById('openPanel');
      const panel = document.getElementById('panel');
      const message = document.getElementById('message');
      toggle?.addEventListener('click', () => {
        panel?.removeAttribute('hidden');
        message?.focus();
      });
    });
  </script>
</head>
<body>
  <h1>Debug Fixture</h1>
  <a id="chatLink" href="/chat/user/123" target="_blank">
    <span id="chatLabel" class="label">Start Chat</span>
  </a>
  <button id="openPanel">Open Panel</button>
  <section id="panel" role="dialog" aria-modal="true" hidden>
    <label for="message">Message</label>
    <textarea id="message" name="message" placeholder="Message"></textarea>
  </section>
</body>
</html>
`;

const TRUST_BOUNDARY_HTML = `
<!doctype html>
<html>
<head>
  <meta charset="utf-8" />
  <title>RZN Trust Boundary Fixture</title>
  <script>
    window.clickLog = [];
    window.typeLog = [];
    window.uploadLog = [];
    window.addEventListener('DOMContentLoaded', () => {
      for (const id of ['jsClick', 'cdpClick']) {
        document.getElementById(id).addEventListener('click', (event) => {
          window.clickLog.push({ id, isTrusted: event.isTrusted });
        });
      }
      for (const id of ['domText', 'cdpText']) {
        document.getElementById(id).addEventListener('input', (event) => {
          window.typeLog.push({ id, value: event.target.value, isTrusted: event.isTrusted });
        });
      }
      document.getElementById('file').addEventListener('change', (event) => {
        const file = event.target.files && event.target.files[0];
        window.uploadLog.push({
          isTrusted: event.isTrusted,
          count: event.target.files ? event.target.files.length : 0,
          name: file ? file.name : null,
        });
      });
    });
  </script>
</head>
<body>
  <button id="jsClick">JS Click</button>
  <button id="cdpClick">CDP Click</button>
  <input id="domText" />
  <input id="cdpText" />
  <input id="file" type="file" />
</body>
</html>
`;

const SHADOW_TEXTAREA_HTML = `
<!doctype html>
<html>
<head>
  <meta charset="utf-8" />
  <title>RZN Shadow Input Fixture</title>
  <style>
    body { font-family: sans-serif; }
    #decoy {
      display: none;
    }
  </style>
  <script>
    class ShadowLevelThree extends HTMLElement {
      constructor() {
        super();
        const root = this.attachShadow({ mode: 'open' });
        const textarea = document.createElement('textarea');
        textarea.name = 'message';
        textarea.placeholder = 'Message';
        textarea.setAttribute('aria-label', 'Write message');
        textarea.id = 'shadow-message';
        root.appendChild(textarea);
      }
    }

    class ShadowLevelTwo extends HTMLElement {
      constructor() {
        super();
        const root = this.attachShadow({ mode: 'open' });
        root.appendChild(document.createElement('shadow-level-three'));
      }
    }

    class ShadowLevelOne extends HTMLElement {
      constructor() {
        super();
        const root = this.attachShadow({ mode: 'open' });
        root.appendChild(document.createElement('shadow-level-two'));
      }
    }

    customElements.define('shadow-level-one', ShadowLevelOne);
    customElements.define('shadow-level-two', ShadowLevelTwo);
    customElements.define('shadow-level-three', ShadowLevelThree);

    window.addEventListener('DOMContentLoaded', () => {
      document.body.appendChild(document.createElement('shadow-level-one'));
    });
  </script>
</head>
<body>
  <h1>Shadow Fixture</h1>
  <textarea id="decoy" name="message" placeholder="Message" aria-label="Write message"></textarea>
</body>
</html>
`;

test.describe('Enhanced actions e2e', () => {
  test('fill_input_field appends when clear_first is false and emits typing events', async () => {
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

    await page.waitForFunction(() => typeof (window as any).__rznExecuteStep === 'function', { timeout: 10000 });

    await page.locator('#name').fill('Seed');
    await page.evaluate(() => {
      const input = document.getElementById('name') as HTMLInputElement | null;
      if (!input) return;
      (window as any).__rznTypingStats = { input: 0, keydown: 0 };
      input.addEventListener('input', () => {
        (window as any).__rznTypingStats.input += 1;
      });
      input.addEventListener('keydown', () => {
        (window as any).__rznTypingStats.keydown += 1;
      });
    });

    const fillResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'fill_input_field',
      selector: '#name',
      value: 'XYZ',
      clear_first: false,
      simulate_typing: true,
      delay_ms: 0,
      force_legacy: true,
    }));

    expect(fillResp.success).toBeTruthy();
    await expect(page.locator('#name')).toHaveValue('SeedXYZ');

    const stats = await page.evaluate(() => (window as any).__rznTypingStats);
    expect(stats.input).toBeGreaterThanOrEqual(3);
    expect(stats.keydown).toBeGreaterThanOrEqual(3);

    await context.close();
    await srv.close();
  });

  test('wait_for_element and fill_input_field can pierce shadow DOM and prefer the visible match', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data-shadow-fill');
    fs.rmSync(userDataDir, { recursive: true, force: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const page = await context.newPage();
    const srv = await startServer(SHADOW_TEXTAREA_HTML);
    await page.goto(srv.url);

    await page.waitForFunction(() => typeof (window as any).__rznExecuteStep === 'function', { timeout: 10000 });

    const waitResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'wait_for_element',
      selector: "textarea[name='message']",
      pierce_shadow: true,
      visible: true,
      force_legacy: true,
      timeout_ms: 2000,
    }));
    expect(waitResp.success).toBeTruthy();

    const fillResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'fill_input_field',
      selector: "textarea[name='message']",
      value: "O'Reilly",
      pierce_shadow: true,
      force_legacy: true,
    }));
    expect(fillResp.success).toBeTruthy();

    const values = await page.evaluate(() => {
      const decoy = document.querySelector('#decoy') as HTMLTextAreaElement | null;
      const host1 = document.querySelector('shadow-level-one') as HTMLElement | null;
      const root1 = host1?.shadowRoot;
      const host2 = root1?.querySelector('shadow-level-two') as HTMLElement | null;
      const root2 = host2?.shadowRoot;
      const host3 = root2?.querySelector('shadow-level-three') as HTMLElement | null;
      const root3 = host3?.shadowRoot;
      const shadow = root3?.querySelector('#shadow-message') as HTMLTextAreaElement | null;
      return {
        decoy: decoy?.value || '',
        shadow: shadow?.value || '',
      };
    });

    expect(values.decoy).toBe('');
    expect(values.shadow).toBe("O'Reilly");

    await context.close();
    await srv.close();
  });

  test('click/fill/press/wait/scroll/extract work via enhanced handlers', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data');

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      // Use Chrome channel only if requested
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const page = await context.newPage();

    // Serve the test page locally over http to allow content script injection
    const srv = await startServer(TEST_HTML);
    await page.goto(srv.url);

    // Wait for the test bridge and helpers
    await page.waitForFunction(() => typeof (window as any).__rznExecuteStep === 'function', { timeout: 10000 });
    await page.waitForFunction(() => typeof (window as any).captureEnhancedDOMSnapshot === 'function', { timeout: 10000 });

    // Fill input
    const fillResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'fill_input_field_enhanced',
      selector: '#name',
      value: 'Alice',
    }));
    expect(fillResp.success).toBeTruthy();
    await expect(page.locator('#name')).toHaveValue('Alice');

    // Click button
    const clickResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'click_element_enhanced',
      selector: '#go',
    }));
    expect(clickResp.success).toBeTruthy();
    await expect(page.locator('#status')).toHaveText('clicked');

    // Wait for delayed element
    const waitResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'wait_for_element',
      selector: '#delayed',
      timeout_ms: 5000,
    }));
    expect(waitResp.success).toBeTruthy();

    // Scroll into view
    const beforeInView = await page.evaluate(() => {
      const el = document.getElementById('bottom');
      const rect = el!.getBoundingClientRect();
      return rect.top >= 0 && rect.bottom <= window.innerHeight;
    });
    expect(beforeInView).toBeFalsy();

    const scrollResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'scroll_element_into_view_enhanced',
      selector: '#bottom',
    }));
    expect(scrollResp.success).toBeTruthy();

    const afterInView = await page.evaluate(() => {
      const el = document.getElementById('bottom');
      const rect = el!.getBoundingClientRect();
      return rect.top >= 0 && rect.bottom <= window.innerHeight;
    });
    expect(afterInView).toBeTruthy();

    // Focus input via enhanced click then press Enter via CDP-based press_key
    await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'click_element_enhanced',
      selector: '#keytarget',
    }));

    const pressResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'press_key',
      key: 'Enter',
    }));
    expect(pressResp.success).toBeTruthy();

    const keyed = await page.getAttribute('#keytarget', 'data-keyed');
    expect(keyed).toBe('yes');

    // Extract structured data
    const extractResp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'extract_structured_data_enhanced',
      selector: '#list',
      fields: [
        { name: 'title', selector: '.title' },
        { name: 'href', selector: '.link', attribute: 'href' },
      ],
    }));
    expect(extractResp.success).toBeTruthy();
    // Basic shape validation
    // @ts-ignore
    const items = extractResp.result as Array<any>;
    expect(Array.isArray(items) || typeof items === 'object').toBeTruthy();

    // Cleanup: close browser first to release HTTP connection, then stop server
    await context.close();
    await srv.close();
  });

  test('take_screenshot returns a data URL', async () => {
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

    await page.waitForFunction(() => typeof (window as any).__rznExecuteStep === 'function', { timeout: 10000 });

    const resp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'take_screenshot',
      format: 'png',
      full_page: false,
    }));

    expect(resp.success).toBeTruthy();
    // @ts-ignore
    expect(typeof resp.result?.data_url).toBe('string');
    // @ts-ignore
    expect(resp.result.data_url.startsWith('data:image/')).toBeTruthy();

    await context.close();
    await srv.close();
  });

  test('debug primitives expose eval, inspection, verification, and semantic actions', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data-debug-tools');
    fs.rmSync(userDataDir, { recursive: true, force: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const page = await context.newPage();
    const srv = await startServer(DEBUG_HTML);
    await page.goto(srv.url);

    await page.waitForFunction(() => typeof (window as any).__rznExecuteStep === 'function', { timeout: 10000 });
    await page.waitForFunction(() => typeof (window as any).__rznEvalMainWorld === 'function', { timeout: 10000 });

    const isolatedEval = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'eval_isolated_world',
      script: "document.querySelector('#message').value = 'Hello from isolated'; return document.querySelector('#message').value;",
    }));
    expect(isolatedEval.success).toBeTruthy();
    expect(isolatedEval.result?.result).toBe('Hello from isolated');

    const mainEval = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'eval_main_world',
      script: 'return window.__mainWorldState.secret;',
    }));
    expect(mainEval.success, JSON.stringify(mainEval)).toBeTruthy();
    expect(mainEval.result?.result).toBe('main-ready');

    const bridgeMainEval = await page.evaluate(async () => (window as any).__rznEvalMainWorld({
      script: 'return window.__mainWorldState.secret;',
    }));
    expect(bridgeMainEval.success).toBeTruthy();
    expect(bridgeMainEval.result?.result).toBe('main-ready');

    const legacyExecute = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'execute_javascript',
      script: 'document.querySelector(\"#message\").value',
    }));
    expect(legacyExecute.success).toBeTruthy();
    expect(legacyExecute.result?.world).toBe('main');
    expect(['page_bridge_main_world_compat', 'chrome_scripting_main_world']).toContain(
      legacyExecute.result?.execution_backend
    );
    expect(legacyExecute.result?.result).toBe('Hello from isolated');

    const safeParamsEval = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'execute_javascript',
      script: 'return window.__rzn_params.message_body;',
      params: {
        message_body: "O'Reilly",
      },
    }));
    expect(safeParamsEval.success).toBeTruthy();
    expect(safeParamsEval.result?.result).toBe("O'Reilly");

    const surface = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'inspect_click_surface',
      selector: '#chatLabel',
    }));
    expect(surface.success).toBeTruthy();
    expect(surface.result?.click_surface?.href).toContain('/chat/user/123');
    expect(surface.result?.actionable_ancestor?.tag).toBe('a');

    const semantic = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'semantic_action',
      action: 'click',
      selector: '#openPanel',
      postcondition: {
        selector: '#panel',
        condition: 'visible',
      },
      timeout_ms: 3000,
    }));
    expect(semantic.success).toBeTruthy();
    expect(semantic.result?.postcondition_verified).toBeTruthy();

    const verify = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'verify_ui_change',
      selector: '#panel',
      condition: 'visible',
      active_selector: '#message',
      timeout_ms: 1000,
    }));
    expect(verify.success).toBeTruthy();

    const readValue = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'read_field_value',
      selector: '#message',
    }));
    expect(readValue.success).toBeTruthy();
    expect(readValue.result?.value).toBe('Hello from isolated');

    const bundle = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'capture_ui_bundle',
      selector: '#panel',
      include_dom_snapshot: true,
    }));
    expect(bundle.success).toBeTruthy();
    expect(bundle.result?.url).toContain('127.0.0.1');
    expect(bundle.result?.target_element?.tag).toBe('section');
    expect(Array.isArray(bundle.result?.visible_overlays)).toBeTruthy();
    expect(bundle.result?.dom_snapshot?.elements?.length).toBeGreaterThan(0);

    const inspect = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'inspect_element',
      selector: '#message',
    }));
    expect(inspect.success).toBeTruthy();
    expect(inspect.result?.element?.attributes?.name).toBe('message');

    await context.close();
    await srv.close();
  });

  test('close_current_tab closes the workflow tab (background executor)', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
    // Use a dedicated profile for this test and clear it to avoid stale extension workers.
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data-close-tab');
    fs.rmSync(userDataDir, { recursive: true, force: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const page = await context.newPage();
    await page.goto('https://example.com');

    const worker =
      context.serviceWorkers().find(w => w.url().includes('background')) ||
      (await context.waitForEvent('serviceworker'));

    await expect.poll(async () => {
      return worker.evaluate(() => typeof (globalThis as any).__rznTestRunWorkflowSteps === 'function');
    }, { timeout: 5000 }).toBeTruthy();

    const closed = await worker.evaluate(async () => {
      // Create a tab to close
      const tab = await chrome.tabs.create({ url: 'about:blank', active: true });
      const tabId = tab.id!;

      // Run the close_current_tab step via the same workflow executor used by the broker.
      const runSteps = (globalThis as any).__rznTestRunWorkflowSteps;
      if (typeof runSteps !== 'function') return false;

      await runSteps([{ type: 'close_current_tab', tab_identifier: null }], { current_tab_id: tabId });
      await new Promise(r => setTimeout(r, 250));

      try {
        await chrome.tabs.get(tabId);
        return false;
      } catch {
        return true;
      }
    });

    expect(closed).toBeTruthy();

    await context.close();
  });

  test('open_new_tab waits for a real http page before content-script actions run', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data-open-new-tab');
    fs.rmSync(userDataDir, { recursive: true, force: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const page = await context.newPage();
    await page.goto('https://example.com');

    const srv = await startServer(DEBUG_HTML);
    const worker =
      context.serviceWorkers().find(w => w.url().includes('background')) ||
      (await context.waitForEvent('serviceworker'));

    await expect.poll(async () => {
      return worker.evaluate(() => typeof chrome.tabs?.query === 'function');
    }, { timeout: 5000 }).toBeTruthy();

    const observed = await worker.evaluate(async ({ url }) => {
      const delay = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
      const runSteps = (globalThis as any).__rznTestRunWorkflowSteps;
      if (typeof runSteps !== 'function') {
        return { ok: false, reason: 'missing-run-steps' };
      }

      await runSteps([{ type: 'open_new_tab', url }]);

      const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
      if (!tab?.id) {
        return { ok: false, reason: 'no-active-tab' };
      }

      let handshake: any = null;
      for (let i = 0; i < 10; i += 1) {
        try {
          handshake = await chrome.tabs.sendMessage(tab.id, { cmd: 'rzn_handshake_v1' });
          if (handshake?.success) break;
        } catch {}
        await delay(150);
      }

      let execute: any = null;
      if (handshake?.success) {
        execute = await chrome.tabs.sendMessage(tab.id, {
          cmd: 'rzn_execute_step_v1',
          req_id: 'pw-open-new-tab-exec-js',
          payload: {
            step: {
              type: 'eval_main_world',
              script: 'return document.title;',
            },
          },
        });
      }

      const finalTab = await chrome.tabs.get(tab.id);
      return {
        ok: handshake?.success === true && execute?.success === true,
        handshake,
        execute,
        title: finalTab.title || '',
        url: finalTab.url || finalTab.pendingUrl || '',
      };
    }, { url: srv.url });

    expect(observed.ok).toBeTruthy();
    expect(observed.url).toContain('127.0.0.1');
    expect(observed.handshake?.protocol_version).toMatch(CONTENT_SCRIPT_PROTOCOL_PATTERN);
    expect(observed.execute?.result?.result).toBe('RZN Debug Tools Fixture');
    expect(observed.title).toBe('RZN Debug Tools Fixture');

    await context.close();
    await srv.close();
  });

  test('background can handshake with and drive versioned content-script commands', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data-handshake');
    fs.rmSync(userDataDir, { recursive: true, force: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const page = await context.newPage();
    const srv = await startServer(DEBUG_HTML);
    await page.goto(srv.url);

    const worker =
      context.serviceWorkers().find(w => w.url().includes('background')) ||
      (await context.waitForEvent('serviceworker'));

    const expectedTitle = await page.title();

    await expect.poll(async () => {
      return worker.evaluate(() => typeof chrome.tabs?.query === 'function');
    }, { timeout: 5000 }).toBeTruthy();

    let observed: any = null;
    await expect
      .poll(async () => {
        observed = await worker.evaluate(async ({ expectedTitle, protocolPattern }) => {
          const delay = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
          const contentScriptProtocolPattern = new RegExp(protocolPattern);
          if (typeof chrome.tabs?.query !== 'function') {
            return { ok: false, reason: 'tabs-api-unavailable' };
          }
          const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
          if (!tab?.id) return { ok: false, reason: 'no-active-tab' };

          let handshake: any = null;
          for (let i = 0; i < 10; i += 1) {
            try {
              handshake = await chrome.tabs.sendMessage(tab.id, { cmd: 'rzn_handshake_v1' });
              break;
            } catch {
              await delay(150);
            }
          }

          if (!handshake?.success) {
            return { ok: false, reason: 'handshake-failed', handshake };
          }

          const execute = await chrome.tabs.sendMessage(tab.id, {
            cmd: 'rzn_execute_step_v1',
            req_id: 'pw-versioned-execute-step',
            payload: {
              step: {
                type: 'eval_main_world',
                script: 'return document.title;',
              },
            },
          });

          const domSnapshot = await chrome.tabs.sendMessage(tab.id, {
            cmd: 'rzn_get_dom_snapshot_v1',
            req_id: 'pw-versioned-dom-snapshot',
            payload: {
              options: {
                maxElements: 20,
                highlightElements: false,
              },
            },
          });

          return {
            ok:
              contentScriptProtocolPattern.test(handshake?.protocol_version || '') &&
              execute?.success === true &&
              execute?.result?.result === expectedTitle &&
              domSnapshot?.success === true &&
              !!domSnapshot?.dom_hash,
            handshake,
            execute,
            domSnapshot,
          };
        }, { expectedTitle, protocolPattern: CONTENT_SCRIPT_PROTOCOL_PATTERN.source });
        return observed?.ok === true;
      }, { timeout: 10000 })
      .toBeTruthy();

    expect(observed.handshake?.protocol_version).toMatch(CONTENT_SCRIPT_PROTOCOL_PATTERN);
    expect(observed.execute?.result?.result).toBe(expectedTitle);
    expect(observed.domSnapshot?.success).toBeTruthy();
    expect(typeof observed.domSnapshot?.dom_hash).toBe('string');

    await context.close();
    await srv.close();
  });

  test('broker-style background execute_step preserves execute_javascript results', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data-broker-exec-js');
    fs.rmSync(userDataDir, { recursive: true, force: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const page = await context.newPage();
    const srv = await startServer(DEBUG_HTML);
    await page.goto(srv.url);

    const worker =
      context.serviceWorkers().find(w => w.url().includes('background')) ||
      (await context.waitForEvent('serviceworker'));

    const expectedTitle = await page.title();

    await expect.poll(async () => {
      return worker.evaluate(() => typeof chrome.tabs?.query === 'function');
    }, { timeout: 5000 }).toBeTruthy();

    const resp = await worker.evaluate(async () => {
      const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
      if (!tab?.id) {
        return { success: false, error: 'no-active-tab' };
      }

      // @ts-ignore
      const defaultEval = await globalThis.__rznTestHandleBrokerMessage({
        cmd: 'execute_step',
        req_id: 'pw-broker-execute-js',
        payload: {
          use_current_tab: true,
          step: {
            type: 'execute_javascript',
            script: 'document.title',
          },
        },
      }, tab.id);

      // @ts-ignore
      const conditionalFalseEval = await globalThis.__rznTestHandleBrokerMessage({
        cmd: 'execute_step',
        req_id: 'pw-broker-execute-js-conditional-false',
        payload: {
          use_current_tab: true,
          step: {
            type: 'execute_javascript',
            script: 'document.title',
            args: ['false'],
            use_cdp_eval_when_arg_truthy: 0,
          },
        },
      }, tab.id);

      // @ts-ignore
      const conditionalTrueEval = await globalThis.__rznTestHandleBrokerMessage({
        cmd: 'execute_step',
        req_id: 'pw-broker-execute-js-conditional-true',
        payload: {
          use_current_tab: true,
          step: {
            type: 'execute_javascript',
            script: 'document.title',
            args: ['true'],
            use_cdp_eval_when_arg_truthy: 0,
          },
        },
      }, tab.id);

      return { defaultEval, conditionalFalseEval, conditionalTrueEval };
    });

    expect(resp.defaultEval?.success, JSON.stringify(resp.defaultEval)).toBeTruthy();
    expect(resp.defaultEval?.result?.success).toBeTruthy();
    expect(resp.defaultEval?.result?.world).toBe('main');
    expect(resp.defaultEval?.result?.execution_backend).toBe('chrome_scripting_main_world');
    expect(resp.defaultEval?.result?.result).toBe(expectedTitle);
    expect(resp.conditionalFalseEval?.success).toBeTruthy();
    expect(resp.conditionalFalseEval?.result?.execution_backend).toBe('chrome_scripting_main_world');
    expect(resp.conditionalTrueEval?.success).toBeTruthy();
    expect(resp.conditionalTrueEval?.result?.execution_backend).toBe('cdp_runtime_evaluate');

    await context.close();
    await srv.close();
  });

  test('broker-style execute_step can open a new tab and keep using that session tab', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data-broker-open-new-tab');
    fs.rmSync(userDataDir, { recursive: true, force: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const page = await context.newPage();
    const srv = await startServer(DEBUG_HTML);
    await page.goto('https://example.com');

    const worker =
      context.serviceWorkers().find(w => w.url().includes('background')) ||
      (await context.waitForEvent('serviceworker'));

    await expect.poll(async () => {
      return worker.evaluate(() => typeof chrome.tabs?.query === 'function');
    }, { timeout: 5000 }).toBeTruthy();

    const resp = await worker.evaluate(async ({ url }) => {
      // @ts-ignore
      const openResp = await globalThis.__rznTestHandleBrokerMessage({
        cmd: 'execute_step',
        req_id: 'pw-broker-open-new-tab',
        payload: {
          session_id: 'pw-open-new-tab-session',
          step: {
            type: 'open_new_tab',
            url,
          },
        },
      });

      // @ts-ignore
      const evalResp = await globalThis.__rznTestHandleBrokerMessage({
        cmd: 'execute_step',
        req_id: 'pw-broker-open-new-tab-eval',
        payload: {
          session_id: 'pw-open-new-tab-session',
          step: {
            type: 'eval_main_world',
            script: 'return document.title;',
          },
        },
      });

      return { openResp, evalResp };
    }, { url: srv.url });

    expect(resp.openResp?.success).toBeTruthy();
    expect(resp.openResp?.current_url).toContain('127.0.0.1');
    expect(resp.openResp?.current_tab_id).toBeTruthy();
    expect(resp.evalResp?.success).toBeTruthy();
    expect(resp.evalResp?.result?.success).toBeTruthy();
    expect(resp.evalResp?.result?.result).toBe('RZN Debug Tools Fixture');
    expect(resp.evalResp?.current_tab_id).toBe(resp.openResp?.current_tab_id);

    await context.close();
    await srv.close();
  });

  test('broker-style eval_with_cdp returns scalar results without content-script execute_step', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data-broker-eval-cdp');
    fs.rmSync(userDataDir, { recursive: true, force: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const page = await context.newPage();
    const srv = await startServer(DEBUG_HTML);
    await page.goto(srv.url);

    const worker =
      context.serviceWorkers().find(w => w.url().includes('background')) ||
      (await context.waitForEvent('serviceworker'));

    const expectedTitle = await page.title();

    await expect.poll(async () => {
      return worker.evaluate(() => typeof chrome.tabs?.query === 'function');
    }, { timeout: 5000 }).toBeTruthy();

    const resp = await worker.evaluate(async () => {
      const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
      if (!tab?.id) {
        return { success: false, error: 'no-active-tab' };
      }

      // @ts-ignore
      return await globalThis.__rznTestHandleBrokerMessage({
        cmd: 'eval_with_cdp',
        req_id: 'pw-broker-eval-with-cdp',
        payload: {
          script: 'document.title',
          world: 'main',
          return_value: true,
        },
      }, tab.id);
    });

    expect(resp.success).toBeTruthy();
    expect(resp.result?.success).toBeTruthy();
    expect(resp.result?.result).toBe(expectedTitle);
    expect(resp.result?.execution_backend).toBe('cdp_runtime_evaluate');

    await context.close();
    await srv.close();
  });

  test('broker-style action routing keeps JS default and CDP explicit', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data-trust-boundary');
    fs.rmSync(userDataDir, { recursive: true, force: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const page = await context.newPage();
    const srv = await startServer(TRUST_BOUNDARY_HTML);
    await page.goto(srv.url);

    const worker =
      context.serviceWorkers().find(w => w.url().includes('background')) ||
      (await context.waitForEvent('serviceworker'));

    await expect.poll(async () => {
      return worker.evaluate(() => typeof chrome.tabs?.query === 'function');
    }, { timeout: 5000 }).toBeTruthy();

    const fixturePath = path.resolve(__dirname, '../../../test/fixtures/upload_test.txt');
    const resp = await worker.evaluate(async ({ fixturePath }) => {
      const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
      if (!tab?.id) {
        return { success: false, error: 'no-active-tab' };
      }

      const send = async (req_id: string, step: any) => {
        // @ts-ignore
        return await globalThis.__rznTestHandleBrokerMessage({
          cmd: 'execute_step',
          req_id,
          payload: {
            use_current_tab: true,
            step,
          },
        }, tab.id);
      };

      const jsClick = await send('pw-js-click', { type: 'click_element', selector: '#jsClick' });
      const cdpClick = await send('pw-cdp-click', { type: 'click_element', selector: '#cdpClick', use_cdp: true });
      const domType = await send('pw-dom-type', { type: 'type_text', selector: '#domText', text: 'dom' });
      const cdpType = await send('pw-cdp-type', { type: 'type_text', selector: '#cdpText', text: 'cdp', use_cdp: true });
      const upload = await send('pw-upload', { type: 'upload_file', selector: '#file', file_path: fixturePath });

      return { jsClick, cdpClick, domType, cdpType, upload };
    }, { fixturePath });

    expect(resp.jsClick?.success).toBeTruthy();
    expect(resp.cdpClick?.success).toBeTruthy();
    expect(resp.cdpClick?.result?.action).toBe('click_element_cdp');
    expect(resp.domType?.success).toBeTruthy();
    expect(resp.cdpType?.success, JSON.stringify(resp.cdpType)).toBeTruthy();
    expect(resp.cdpType?.result?.action).toBe('type_text');
    expect(resp.upload?.success).toBeTruthy();
    expect(resp.upload?.result?.action).toBe('upload_file_cdp');
    expect(resp.upload?.result?.file_count).toBe(1);
    expect(resp.upload?.result?.files).toContain('upload_test.txt');

    const pageState = await page.evaluate(() => ({
      clickLog: (window as any).clickLog,
      typeLog: (window as any).typeLog,
      uploadLog: (window as any).uploadLog,
      domText: (document.querySelector('#domText') as HTMLInputElement).value,
      cdpText: (document.querySelector('#cdpText') as HTMLInputElement).value,
      fileName: (document.querySelector('#file') as HTMLInputElement).files?.[0]?.name || null,
    }));

    expect(pageState.clickLog.find((entry: any) => entry.id === 'jsClick')?.isTrusted).toBe(false);
    expect(pageState.clickLog.find((entry: any) => entry.id === 'cdpClick')?.isTrusted).toBe(true);
    expect(pageState.domText).toBe('dom');
    expect(pageState.cdpText).toBe('cdp');
    expect(pageState.typeLog.some((entry: any) => entry.id === 'domText' && entry.isTrusted === false)).toBe(true);
    expect(pageState.typeLog.some((entry: any) => entry.id === 'cdpText' && entry.isTrusted === true)).toBe(true);
    expect(pageState.fileName).toBe('upload_test.txt');
    expect(pageState.uploadLog[0]?.count).toBe(1);

    await context.close();
    await srv.close();
  });

  test('broker-style execute_step isolates dedicated tabs across workflow sessions', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data-session-isolation');
    fs.rmSync(userDataDir, { recursive: true, force: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const page = await context.newPage();
    const srv = await startServer(DEBUG_HTML);
    await page.goto(srv.url);

    const worker =
      context.serviceWorkers().find(w => w.url().includes('background')) ||
      (await context.waitForEvent('serviceworker'));

    await expect.poll(async () => {
      return worker.evaluate(() => typeof chrome.tabs?.query === 'function');
    }, { timeout: 5000 }).toBeTruthy();

    const resp = await worker.evaluate(async () => {
      const [activeTab] = await chrome.tabs.query({ active: true, currentWindow: true });
      const activeTabId = activeTab?.id;
      if (!activeTabId) {
        return { success: false, error: 'no-active-tab' };
      }

      // @ts-ignore
      const first = await globalThis.__rznTestHandleBrokerMessage({
        cmd: 'execute_step',
        req_id: 'pw-session-a-exec-js',
        payload: {
          session_id: 'pw-session-a',
          step: {
            type: 'execute_javascript',
            script: 'document.title',
          },
        },
      });

      // @ts-ignore
      const second = await globalThis.__rznTestHandleBrokerMessage({
        cmd: 'execute_step',
        req_id: 'pw-session-b-exec-js',
        payload: {
          session_id: 'pw-session-b',
          step: {
            type: 'execute_javascript',
            script: 'document.title',
          },
        },
      });

      return { activeTabId, first, second };
    });

    expect(resp.first?.success).toBeTruthy();
    expect(resp.second?.success).toBeTruthy();
    expect(resp.first?.current_tab_id).toBeTruthy();
    expect(resp.second?.current_tab_id).toBeTruthy();
    expect(resp.first?.current_tab_id).not.toBe(resp.activeTabId);
    expect(resp.second?.current_tab_id).not.toBe(resp.activeTabId);
    expect(resp.first?.current_tab_id).not.toBe(resp.second?.current_tab_id);

    await context.close();
    await srv.close();
  });

  test('broker-style eval_with_cdp fails closed for session-scoped requests without a dedicated tab', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data-session-eval-closed');
    fs.rmSync(userDataDir, { recursive: true, force: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const page = await context.newPage();
    const srv = await startServer(DEBUG_HTML);
    await page.goto(srv.url);

    const worker =
      context.serviceWorkers().find(w => w.url().includes('background')) ||
      (await context.waitForEvent('serviceworker'));

    await expect.poll(async () => {
      return worker.evaluate(() => typeof chrome.tabs?.query === 'function');
    }, { timeout: 5000 }).toBeTruthy();

    const resp = await worker.evaluate(async () => {
      // @ts-ignore
      return await globalThis.__rznTestHandleBrokerMessage({
        cmd: 'eval_with_cdp',
        req_id: 'pw-session-eval-with-cdp',
        payload: {
          session_id: 'pw-strict-session',
          script: 'document.title',
          world: 'main',
          return_value: true,
        },
      });
    });

    expect(resp.success).toBeFalsy();
    expect(resp.error_code).toBe('EVAL_ERROR');
    expect(resp.error_msg).toContain('No dedicated workflow tab available for eval_with_cdp');

    await context.close();
    await srv.close();
  });

  test('broker-style eval_with_cdp honors explicit use_current_tab for session-scoped requests', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data-session-eval-current-tab');
    fs.rmSync(userDataDir, { recursive: true, force: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const page = await context.newPage();
    const srv = await startServer(DEBUG_HTML);
    await page.goto(srv.url);

    const worker =
      context.serviceWorkers().find(w => w.url().includes('background')) ||
      (await context.waitForEvent('serviceworker'));

    await expect.poll(async () => {
      return worker.evaluate(() => typeof chrome.tabs?.query === 'function');
    }, { timeout: 5000 }).toBeTruthy();

    const expectedTitle = await page.title();

    const resp = await worker.evaluate(async () => {
      const [activeTab] = await chrome.tabs.query({ active: true, currentWindow: true });
      if (!activeTab?.id) {
        return { success: false, error: 'no-active-tab' };
      }

      // @ts-ignore
      return await globalThis.__rznTestHandleBrokerMessage({
        cmd: 'eval_with_cdp',
        req_id: 'pw-session-eval-current-tab',
        payload: {
          session_id: 'pw-current-tab-session',
          script: 'document.title',
          world: 'main',
          return_value: true,
          use_current_tab: true,
        },
      });
    });

    expect(resp.success).toBeTruthy();
    expect(resp.result?.success).toBeTruthy();
    expect(resp.result?.result).toBe(expectedTitle);
    expect(resp.current_tab_id).toBeTruthy();

    await context.close();
    await srv.close();
  });

  test('wait_for_element respects timeout_ms in legacy handler', async () => {
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

    await page.waitForFunction(() => typeof (window as any).__rznExecuteStep === 'function', { timeout: 10000 });

    const startedAt = Date.now();
    const resp = await page.evaluate(async () => (window as any).__rznExecuteStep({
      type: 'wait_for_element',
      selector: '#delayed',
      timeout_ms: 100,
      force_legacy: true,
    }));
    const elapsedMs = Date.now() - startedAt;

    expect(resp.success).toBeFalsy();
    expect(elapsedMs).toBeLessThan(2000);

    await context.close();
    await srv.close();
  });
});
