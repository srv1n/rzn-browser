// Capture of opened pages (BFL-T-0025), end to end in a headless Chromium with the extension
// loaded unpacked, the native host and a supervisor: a page of a site the owner switched on is
// read in place with web/page and pushed, in one batch, to the push door.
//
//   RZN_E2E_CAPTURE=1 bun x playwright test tests/e2e/capture_pages.spec.ts
//
// Fixture mode (default): a local server serves tests/fixtures/capture/article.html and plays the
// push door. Live mode: RZN_E2E_CAPTURE_LIVE_URL names a public page and RZN_E2E_CAPTURE_TARGET a
// capture.json (server_url, index_id, source_key, token) for a pushed source of a test index;
// the batch goes to that server. Never run against a browser profile of a person.
import { chromium, test, expect, type BrowserContext } from '@playwright/test';
import { execFile, spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import type { AddressInfo } from 'node:net';
import path from 'node:path';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';

const execFileAsync = promisify(execFile);
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, '../../..');
const extensionDir = path.resolve(__dirname, '../../dist/chromium');
const fixture = path.resolve(__dirname, '../fixtures/capture/article.html');
const enabled = process.env.RZN_E2E_CAPTURE === '1';
const liveUrl = process.env.RZN_E2E_CAPTURE_LIVE_URL;
const liveTarget = process.env.RZN_E2E_CAPTURE_TARGET;
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function binary(name: 'rzn-browser' | 'rzn-native-host'): string {
  const env = name === 'rzn-browser' ? process.env.RZN_BROWSER_BIN : process.env.RZN_NATIVE_HOST_BIN;
  return env || path.join(repoRoot, 'target/debug', name);
}

function envFor(home: string, appBase: string): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {
    ...process.env, HOME: home, XDG_CONFIG_HOME: path.join(home, '.config'), XDG_DATA_HOME: path.join(home, '.local/share'),
    RZN_APP_BASE_DIR: appBase, RZN_SUPERVISOR_APP_BASE: appBase, RZN_WORKFLOWS_DIR: path.join(repoRoot, 'workflows'),
  };
  for (const key of ['RZN_LOCAL_RUNTIME_SOCKET_PATH', 'RZN_SUPERVISOR_SOCKET_PATH', 'RZN_LOCAL_RUNTIME_TOKEN_PATH', 'RZN_SUPERVISOR_TOKEN_PATH']) delete env[key];
  return env;
}

async function cli(env: NodeJS.ProcessEnv, args: string[]): Promise<any> {
  const { stdout } = await execFileAsync(binary('rzn-browser'), args, { cwd: repoRoot, env, timeout: 30_000 });
  return JSON.parse(stdout);
}

const call = (env: NodeJS.ProcessEnv, appBase: string, method: string, params: object = {}) =>
  cli(env, ['supervisor', 'call', '--app-base', appBase, '--json', method, '--params', JSON.stringify(params)]);

async function launch(profile: string, env: NodeJS.ProcessEnv): Promise<BrowserContext> {
  return await chromium.launchPersistentContext(profile, {
    headless: process.env.RZN_PW_HEADFUL !== '1', channel: 'chromium', env,
    ignoreDefaultArgs: ['--disable-extensions'],
    args: [`--disable-extensions-except=${extensionDir}`, `--load-extension=${extensionDir}`],
  });
}

async function extensionId(context: BrowserContext): Promise<string> {
  const worker = context.serviceWorkers()[0] ?? (await context.waitForEvent('serviceworker', { timeout: 10_000 }));
  return new URL(worker.url()).host;
}

function activity(appBase: string): any[] {
  try {
    return fs.readFileSync(path.join(appBase, 'capture', 'activity.jsonl'), 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
  } catch {
    return [];
  }
}

async function waitFor<T>(what: string, probe: () => T | undefined, ms = 30_000): Promise<T> {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    const value = probe();
    if (value !== undefined) return value;
    await sleep(250);
  }
  throw new Error(`timed out waiting for ${what}`);
}

/** The fixture site and a stand-in push door on one local port. */
async function fixtureServer() {
  const posts: { path: string; auth?: string; body: any }[] = [];
  const server = http.createServer((req, res) => {
    if (req.method === 'GET' && req.url?.startsWith('/blog/')) {
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
      res.end(fs.readFileSync(fixture));
      return;
    }
    if (req.method === 'POST' && /^\/api\/indexes\/[^/]+\/sources\/[^/]+\/items$/.test(req.url || '')) {
      let raw = '';
      req.on('data', (chunk) => { raw += chunk; });
      req.on('end', () => {
        posts.push({ path: req.url!, auth: req.headers.authorization, body: JSON.parse(raw) });
        res.writeHead(202, { 'content-type': 'application/json' });
        res.end(JSON.stringify({ job_id: `job-${posts.length}`, documents: [] }));
      });
      return;
    }
    res.writeHead(404).end();
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  return { server, posts, origin };
}

test.describe('capture of opened pages', () => {
  test.skip(!enabled, 'set RZN_E2E_CAPTURE=1 (needs target/debug binaries and dist/chromium)');

  test('a page of a switched-on site is read in place and pushed in one batch; off, nothing is read', async () => {
    test.setTimeout(240_000);
    const root = await fs.promises.mkdtemp('/tmp/rzn-capture-');
    const home = path.join(root, 'home');
    const appBase = path.join(root, 'app-base');
    await fs.promises.mkdir(appBase, { recursive: true });
    const env = envFor(home, appBase);
    const local = liveUrl ? null : await fixtureServer();
    const pageUrl = liveUrl || `${local!.origin}/blog/offline-notes`;
    const site = new URL(pageUrl).hostname.replace(/^www\./, '');
    const target = liveTarget
      ? fs.readFileSync(liveTarget, 'utf8')
      : JSON.stringify({ server_url: local!.origin, index_id: 'capture-test', source_key: 'main', token: 'test-token' });
    fs.writeFileSync(path.join(appBase, 'capture.json'), target, { mode: 0o600 });

    const first = await launch(path.join(root, 'profile-first'), env);
    const id = await extensionId(first);
    await first.close();
    const installed = await cli(env, ['native-host', 'install', '--browser', 'chromium', '--extension-origin', `chrome-extension://${id}/`,
      '--native-host-path', binary('rzn-native-host'), '--json']);
    // macOS Chromium looks for hosts under the real home, not $HOME; the profile's own
    // NativeMessagingHosts folder keeps the registration inside this test's profile.
    const manifest = installed.reports?.[0]?.manifest_path;
    const profile = path.join(root, 'profile-live');
    await fs.promises.mkdir(path.join(profile, 'NativeMessagingHosts'), { recursive: true });
    await fs.promises.copyFile(manifest, path.join(profile, 'NativeMessagingHosts', path.basename(manifest)));
    const log = fs.openSync(path.join(root, 'supervisor.log'), 'a');
    const supervisor = spawn(binary('rzn-browser'), ['supervisor', 'serve', '--app-base', appBase], {
      cwd: repoRoot, env: { ...env, RUST_LOG: env.RUST_LOG || 'info' }, stdio: ['ignore', log, log],
    });
    let context: BrowserContext | null = null;
    try {
      await sleep(750);
      context = await launch(profile, env);
      // Ready once the bridge reports the browser instance a tab_ref names (it arrives with the
      // supervisor's first health ping of the extension).
      const deadline = Date.now() + 90_000;
      for (;;) {
        const targets = await call(env, appBase, 'browser.targets').catch(() => null);
        if (JSON.stringify(targets?.targets ?? []).match(/"browser_instance_id":"[0-9a-f-]{36}"/)) break;
        if (Date.now() > deadline) throw new Error(`the extension never reached the supervisor: ${JSON.stringify(targets)}`);
        await sleep(500);
      }

      // Off (the default): the page loads and nothing is read or sent.
      const off = await context.newPage();
      await off.goto(pageUrl, { waitUntil: 'load' });
      await sleep(5_000);
      expect(activity(appBase).filter((e) => e.event === 'page_read')).toEqual([]);
      expect((await call(env, appBase, 'capture.flush')).items).toBe(0);
      if (local) expect(local.posts).toEqual([]);
      await off.close();

      // The owner switches the site on in the dashboard (the extension's own settings path).
      const dashboard = await context.newPage();
      await dashboard.goto(`chrome-extension://${id}/dashboard.html`);
      const saved = await dashboard.evaluate(async (sites) => await chrome.runtime.sendMessage({
        cmd: 'supervisor_rpc', payload: { method: 'settings.set', params: { patch: { capture_sites: sites } } },
      }), [site]);
      expect(saved?.result?.capture_sites).toEqual([site]);
      await sleep(1_000);

      const opened = await context.newPage();
      await opened.goto(pageUrl, { waitUntil: 'load' });
      const read = await waitFor('the page read', () => activity(appBase).find((e) => e.event === 'page_read'), 60_000);
      console.log('page_read', JSON.stringify(read));
      if (read.error) console.log('targets', JSON.stringify(await call(env, appBase, 'browser.targets')), 'logs in', root);
      expect(read.error).toBeUndefined();
      expect(read.items).toBe(1);
      expect(read.workflow_id).toBe('web/page');
      // In place: the owner's page stayed where it was, unscrolled.
      expect(opened.url()).toBe(pageUrl);
      expect(await opened.evaluate(() => window.scrollY)).toBe(0);

      expect((await call(env, appBase, 'capture.flush')).items).toBe(1);
      const sent = await waitFor('the batch', () => activity(appBase).find((e) => e.event === 'batch_sent'), 60_000);
      console.log('batch_sent', JSON.stringify(sent));
      expect(sent.items).toBe(1);
      expect(sent.keys).toEqual(read.keys);
      if (local) {
        expect(local.posts).toHaveLength(1);
        const post = local.posts[0];
        expect(post.path).toBe('/api/indexes/capture-test/sources/main/items');
        expect(post.auth).toBe('Bearer test-token');
        const [item] = post.body.items;
        expect(post.body.type).toBe('rzn.items/v2');
        expect(item.key).toBe(`web:page:${pageUrl}`);
        expect(item.url).toBe(pageUrl);
        expect(item.title).toBe('Notes that work offline');
        expect(item.author).toEqual({ name: 'Ada Example' });
        expect(item.published_at).toBe('2026-11-20T09:00:00Z');
        expect(item.audience).toBe('public');
        expect(item.blocks.map((b: any) => b.text)).toEqual([
          'Notes that work offline',
          'A notes app that needs the network loses the thought you had on the train.',
          'Transcription on the device',
          'Offline transcription runs the speech model on the phone, so nothing leaves it.',
        ]);
        fs.writeFileSync(path.join(root, 'pushed-item.json'), JSON.stringify(item, null, 2));
        console.log('pushed item written to', path.join(root, 'pushed-item.json'));
      }
    } finally {
      await context?.close();
      supervisor.kill();
      local?.server.close();
    }
  });
});
