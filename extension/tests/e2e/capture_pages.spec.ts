// Capture of opened pages (BFL-T-0025, BFL-T-0026), end to end in a headless Chromium with the
// extension loaded unpacked, the native host and a supervisor: a page of a site the owner
// switched on is read in place and pushed, in one batch, to the push door. An X post, a LinkedIn
// post and a Reddit thread are read by their site readers (saved fixture pages served at the
// sites' own URLs); every other site by web/page.
//
//   RZN_E2E_CAPTURE=1 bun x playwright test tests/e2e/capture_pages.spec.ts
//
// Fixture mode (default): a local server serves tests/fixtures/capture/article.html and plays the
// push door and the account's token endpoint (the helper is signed in through account.json, the
// index and pushed source are Settings). Live mode: RZN_E2E_CAPTURE_LIVE_URL names a public page,
// RZN_E2E_CAPTURE_ACCOUNT a signed-in account.json and RZN_E2E_CAPTURE_TARGET a JSON
// {index_id, source_key} naming a pushed source of a test index;
// the batch goes to that server. RZN_E2E_CAPTURE_LIVE_REDDIT names a public Reddit thread to open
// logged out in the readers test (one page). Never run against a browser profile of a person.
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
const fixtures = path.resolve(__dirname, '../fixtures/capture');
const fixture = path.join(fixtures, 'article.html');
const liveReddit = process.env.RZN_E2E_CAPTURE_LIVE_REDDIT;
const enabled = process.env.RZN_E2E_CAPTURE === '1';
const liveUrl = process.env.RZN_E2E_CAPTURE_LIVE_URL;
const liveTarget = process.env.RZN_E2E_CAPTURE_TARGET;
const liveAccount = process.env.RZN_E2E_CAPTURE_ACCOUNT;
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
    // A live site challenges the HeadlessChrome user agent; a live run sends an ordinary one.
    ...(liveReddit ? { userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36' } : {}),
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
    if (req.method === 'POST' && req.url === '/oauth/token') {
      req.resume();
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ access_token: 'test-token', expires_in: 3600, refresh_token: 'test-refresh' }));
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


/** A supervisor, the native host and a headless Chromium with the extension, reaching each other. */
/** Where pushes go: the signed-in session (`account.json`) and the index and source in Settings. */
type Target = { account: string; index_id: string; source_key: string };

function fixtureTarget(origin: string): Target {
  return { account: JSON.stringify({ version: 'rzn.account.session', server_url: origin, refresh_token: 'test-refresh' }), index_id: 'capture-test', source_key: 'main' };
}

async function harness(target: Target) {
  const root = await fs.promises.mkdtemp('/tmp/rzn-capture-');
  const home = path.join(root, 'home');
  const appBase = path.join(root, 'app-base');
  await fs.promises.mkdir(appBase, { recursive: true });
  const env = envFor(home, appBase);
  fs.writeFileSync(path.join(appBase, 'account.json'), target.account, { mode: 0o600 });
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
  const close = async () => { await context?.close(); supervisor.kill(); };
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
  } catch (error) {
    await close();
    throw error;
  }
  /** The owner switches sites on in the dashboard (the extension's own settings path). */
  const switchOn = async (sites: string[]) => {
    const dashboard = await context!.newPage();
    await dashboard.goto(`chrome-extension://${id}/dashboard.html`);
    const saved = await dashboard.evaluate(async (patch) => await chrome.runtime.sendMessage({
      cmd: 'supervisor_rpc', payload: { method: 'settings.set', params: { patch } },
    }), { capture_sites: sites, capture_index: target.index_id, capture_source: target.source_key });
    expect([...(saved?.result?.capture_sites ?? [])].sort()).toEqual([...sites].sort());
    await dashboard.close();
    await sleep(1_000);
  };
  return { root, appBase, env, context: context!, switchOn, close, call: (method: string, params: object = {}) => call(env, appBase, method, params) };
}

test.describe('capture of opened pages', () => {
  test.skip(!enabled, 'set RZN_E2E_CAPTURE=1 (needs target/debug binaries and dist/chromium)');

  test('a page of a switched-on site is read in place and pushed in one batch; off, nothing is read', async () => {
    test.setTimeout(240_000);
    const local = liveUrl ? null : await fixtureServer();
    const pageUrl = liveUrl || `${local!.origin}/blog/offline-notes`;
    const site = new URL(pageUrl).hostname.replace(/^www\./, '');
    const target = liveTarget && liveAccount
      ? { ...JSON.parse(fs.readFileSync(liveTarget, 'utf8')), account: fs.readFileSync(liveAccount, 'utf8') }
      : fixtureTarget(local!.origin);
    const h = await harness(target);
    const { appBase, context } = h;
    try {
      // Off (the default): the page loads and nothing is read or sent.
      const off = await context.newPage();
      await off.goto(pageUrl, { waitUntil: 'load' });
      await sleep(5_000);
      expect(activity(appBase).filter((e) => e.event === 'page_read')).toEqual([]);
      expect((await h.call('capture.flush')).items).toBe(0);
      if (local) expect(local.posts).toEqual([]);
      await off.close();

      await h.switchOn([site]);

      const opened = await context.newPage();
      await opened.goto(pageUrl, { waitUntil: 'load' });
      const read = await waitFor('the page read', () => activity(appBase).find((e) => e.event === 'page_read'), 60_000);
      console.log('page_read', JSON.stringify(read));
      if (read.error) console.log('targets', JSON.stringify(await h.call('browser.targets')), 'logs in', h.root);
      expect(read.error).toBeUndefined();
      expect(read.items).toBe(1);
      expect(read.workflow_id).toBe('web/page');
      // In place: the owner's page stayed where it was, unscrolled.
      expect(opened.url()).toBe(pageUrl);
      expect(await opened.evaluate(() => window.scrollY)).toBe(0);

      expect((await h.call('capture.flush')).items).toBe(1);
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
        fs.writeFileSync(path.join(h.root, 'pushed-item.json'), JSON.stringify(item, null, 2));
        console.log('pushed item written to', path.join(h.root, 'pushed-item.json'));
      }
    } finally {
      await h.close();
      local?.server.close();
    }
  });

  test('an X post, a LinkedIn post and a Reddit thread are read in place by their site readers', async () => {
    test.setTimeout(360_000);
    const local = await fixtureServer();
    const h = await harness(fixtureTarget(local.origin));
    const { appBase, context } = h;
    // The saved pages at the sites' own URLs; nothing of these sites is fetched from the network.
    const pages: Record<string, string> = {
      'https://x.com/ada/status/1890000000000000002': 'x_post.html',
      'https://x.com/home': 'x_post.html',
      'https://www.linkedin.com/feed/update/urn:li:activity:7400000000000000001/': 'linkedin_post.html',
      'https://www.linkedin.com/feed/update/urn:li:activity:7400000000000000002/': 'linkedin_post_collapsed.html',
      'https://www.reddit.com/r/rust/comments/1y9abcd/offline_notes_that_survive_the_train/': 'reddit_thread.html',
      'https://old.reddit.com/r/rust/comments/1y9abcd/offline_notes_that_survive_the_train/': 'reddit_thread_old.html',
    };
    // In a live run www.reddit.com is not routed at all (its challenge page fails under routing).
    const routed = liveReddit ? /^https:\/\/(x\.com|www\.linkedin\.com|old\.reddit\.com)\// : /^https:\/\/(x\.com|www\.linkedin\.com|(www|old)\.reddit\.com)\//;
    await context.route(routed, async (route) => {
      const url = route.request().url();
      const file = pages[url];
      if (!file) return route.fulfill({ status: 404, body: '' });
      return route.fulfill({ status: 200, contentType: 'text/html; charset=utf-8', body: fs.readFileSync(path.join(fixtures, file)) });
    });
    const items = new Map<string, any>();
    const reads: any[] = [];
    const open = async (opened: string, settle?: string) => {
      const page = await context.newPage();
      await page.goto(opened, { waitUntil: 'load' });
      // A live site may answer with its own challenge page first and then go on to the thread by
      // itself; the read that counts is the one of the page it settles on.
      if (settle) await page.waitForSelector(settle, { timeout: 60_000 });
      const url = page.url();
      const read = await waitFor(`the read of ${url}`, () => activity(appBase).find((e) => e.event === 'page_read' && e.url === url), 90_000);
      console.log('page_read', JSON.stringify(read));
      // In place: the owner's page stayed where it was, unscrolled, nothing clicked.
      expect(page.url()).toBe(url);
      expect(await page.evaluate(() => [window.scrollY, document.body.dataset.clicked ?? null])).toEqual([0, null]);
      reads.push(read);
      await page.close();
      return read;
    };
    try {
      await h.switchOn(['x.com', 'linkedin.com', 'reddit.com']);

      // A feed is not an item page: not read at all.
      const feed = await context.newPage();
      await feed.goto('https://x.com/home', { waitUntil: 'load' });
      await sleep(5_000);
      expect(activity(appBase).filter((e) => e.event === 'page_read')).toEqual([]);
      await feed.close();

      const x = await open('https://x.com/ada/status/1890000000000000002');
      expect(x).toMatchObject({ workflow_id: 'x/open', items: 1, keys: ['x:post:1890000000000000001'] });
      const li = await open('https://www.linkedin.com/feed/update/urn:li:activity:7400000000000000001/');
      expect(li).toMatchObject({ workflow_id: 'linkedin/open', items: 1, keys: ['linkedin:post:7400000000000000001'] });
      const cut = await open('https://www.linkedin.com/feed/update/urn:li:activity:7400000000000000002/');
      expect(cut).toMatchObject({ workflow_id: 'linkedin/open', items: 0, truncated: [{ key: 'linkedin:post:7400000000000000002', reason: 'text_collapsed' }] });
      const old = await open('https://old.reddit.com/r/rust/comments/1y9abcd/offline_notes_that_survive_the_train/');
      expect(old).toMatchObject({ workflow_id: 'reddit/thread', items: 1, keys: ['reddit:thread:1y9abcd'] });
      expect((await h.call('capture.flush')).items).toBe(3);
      await waitFor('the first batch', () => (local.posts.length ? true : undefined), 60_000);
      for (const item of local.posts.flatMap((p) => p.body.items)) items.set(item.key, item);
      const oldItem = items.get('reddit:thread:1y9abcd');

      const redditUrl = liveReddit || 'https://www.reddit.com/r/rust/comments/1y9abcd/offline_notes_that_survive_the_train/';
      const thread = await open(redditUrl, liveReddit ? 'shreddit-post' : undefined);
      expect(thread.error).toBeUndefined();
      expect(thread.workflow_id).toBe('reddit/thread');
      expect((await h.call('capture.flush')).items).toBe(thread.items);
      if (thread.items) await waitFor('the second batch', () => (local.posts.length > 1 ? true : undefined), 60_000);
      const newItems = local.posts.slice(1).flatMap((p) => p.body.items);
      fs.writeFileSync(path.join(h.root, 'reader-items.json'), JSON.stringify({ reads, items: local.posts.flatMap((p) => p.body.items) }, null, 2));
      console.log('reader items written to', path.join(h.root, 'reader-items.json'));

      const post = items.get('x:post:1890000000000000001');
      expect(post).toMatchObject({
        type: 'rzn.item/v2', kind: 'post', url: 'https://x.com/ada/status/1890000000000000001',
        author: { name: 'Ada Example', id: 'ada', url: 'https://x.com/ada' }, published_at: '2026-11-20T09:00:00Z',
        published_in: { name: 'X', id: 'x.com', url: 'https://x.com' }, fields: { lang: 'en' },
        counts: { replies: 12, reposts: 30, likes: 200, bookmarks: 5, views: 10234 },
      });
      expect(post.truncated).toBeUndefined();
      expect(post.blocks.map((b: any) => [b.id, b.kind, b.text])).toEqual([
        ['1890000000000000001', 'post', 'A notes app that needs the network loses the thought you had on the train. 🚆\nThread on how we fixed it: https://voicebraindump.com/offline'],
        ['q-1880000000000000009', 'quoted', 'Write it down before it is gone.'],
        ['1890000000000000002', 'post', 'Transcription runs on the phone, so nothing leaves it.'],
      ]);
      expect(post.blocks[1].author).toEqual({ name: 'Grace Hopper', id: 'grace', url: 'https://x.com/grace' });
      expect(post.blocks[2]).toMatchObject({ published_at: '2026-11-20T09:01:00Z',
        attachments: [{ kind: 'image', url: 'https://pbs.twimg.com/media/GdAbCdEf.jpg', alt: 'A phone transcribing offline' }] });

      expect(items.get('linkedin:post:7400000000000000001')).toMatchObject({
        kind: 'post', url: 'https://www.linkedin.com/feed/update/urn:li:activity:7400000000000000001/',
        author: { name: 'Ada Example', id: 'ada-example', url: 'https://www.linkedin.com/in/ada-example/' },
        published_in: { name: 'LinkedIn', id: 'linkedin.com' }, counts: { reactions: 128, comments: 14, reposts: 3 },
        blocks: [{ id: '7400000000000000001', kind: 'post', text: 'A notes app that needs the network loses the thought you had on the train.\n\nWe moved transcription onto the phone.' }],
      });

      expect(oldItem).toMatchObject({ kind: 'thread', url: 'https://www.reddit.com/r/rust/comments/1y9abcd/offline_notes_that_survive_the_train/',
        author: { name: 'u/ada_example', id: 'ada_example' }, published_in: { name: 'r/rust', id: 'rust' } });
      expect(oldItem.blocks.map((b: any) => [b.id, b.parent ?? null, b.fields?.depth ?? null, b.fields?.is_op ?? null, b.counts?.score ?? null])).toEqual([
        ['1y9abcd', null, null, null, null], ['19aaaa1', '1y9abcd', 0, false, 12], ['19aaaa2', '19aaaa1', 1, true, 7],
      ]);

      if (liveReddit) {
        // A live thread: its own key and comments; a thread whose page left comments unloaded is
        // held back as truncated (the push door takes only whole items).
        const id = (liveReddit.match(/\/comments\/([a-z0-9]+)/i) || [])[1];
        if (thread.items) {
          const live = newItems.find((i) => i.key === `reddit:thread:${id}`);
          expect(live).toBeTruthy();
          const comments = live.blocks.filter((b: any) => b.kind === 'comment');
          expect(comments.length).toBeGreaterThan(0);
          for (const c of comments) expect(Number.isInteger(c.fields.depth) && typeof c.fields.is_op === 'boolean' && c.author && c.published_at).toBeTruthy();
        } else {
          expect(thread.truncated?.[0]).toMatchObject({ key: `reddit:thread:${id}`, reason: 'more_comments_not_fetched' });
        }
      } else {
        // The new page reads to the same item as the old one, comments with depth, score and OP.
        const fresh = newItems.find((i) => i.key === 'reddit:thread:1y9abcd');
        expect(fresh).toMatchObject({ kind: 'thread', url: oldItem.url, author: oldItem.author, published_in: oldItem.published_in,
          published_at: '2026-11-27T15:11:36+00:00', fields: { flair: '🛠️ project', domain: 'self.rust', is_self: true, over_18: false },
          counts: { score: 42, upvote_ratio: 0.95, comments: 3 } });
        expect(fresh.blocks.map((b: any) => [b.id, b.parent ?? null, b.fields?.depth ?? null, b.fields?.is_op ?? null, b.counts?.score ?? null, b.published_at ?? null])).toEqual([
          ['1y9abcd', null, null, null, null, null], ['19aaaa1', '1y9abcd', 0, false, 12, '2026-11-27T16:00:00+00:00'],
          ['19aaaa2', '19aaaa1', 1, true, 7, '2026-11-27T16:05:00+00:00'], ['19aaaa3', '1y9abcd', 0, false, -2, '2026-11-27T17:00:00+00:00'],
        ]);
        expect(fresh.blocks[2].url).toBe('https://www.reddit.com/r/rust/comments/1y9abcd/offline_notes_that_survive_the_train/19aaaa2/');
        expect(fresh.blocks[0].text).toBe('We moved transcription onto the phone.\n\nHere is how.');
      }
    } finally {
      await h.close();
      local.server.close();
    }
  });
});
