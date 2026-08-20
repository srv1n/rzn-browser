// Fake-DOM check for chatgpt_read's attachment capture: every sandbox link in the
// thread (not just the last turn, not just the rendered ones) must be resolved,
// fetched, and zipped, and a failing file must not take the read down with it.
import assert from 'node:assert';
import { readFileSync } from 'node:fs';

const manifest = JSON.parse(readFileSync(new URL('./chatgpt_read.json', import.meta.url), 'utf8'));
const readScript = manifest.steps[2].action.inputs.script;
const zipScript = manifest.steps[3].action.inputs.script;

const CHAT_ID = 'chat-1';
const mapping = {
  root: { parent: null, children: ['u1'], message: null },
  u1: {
    parent: 'root', children: ['a1'],
    message: { id: 'u1', author: { role: 'user' }, content: { content_type: 'text', parts: ['do it'] },
               metadata: { attachments: [{ id: 'file-up1', name: 'upload.zip' }, { id: 'file-up2', name: 'huge.pdf' }] } },
  },
  // mid-thread turn with two links — the case the DOM scraper used to miss
  a1: {
    parent: 'u1', children: ['a2'],
    message: { id: 'a1', author: { role: 'assistant' }, content: { content_type: 'text',
      parts: ['early drop:\n- [Plan](sandbox:/mnt/data/plan.md)\n- [Data](sandbox:/mnt/data/mid%20file.csv)'] },
      metadata: {}, status: 'finished_successfully', end_turn: true, recipient: 'all' },
  },
  a2: {
    parent: 'a1', children: [],
    message: { id: 'a2', author: { role: 'assistant' }, content: { content_type: 'text',
      parts: ['final: [Bundle](sandbox:/mnt/data/out.zip) and [Broken](sandbox:/mnt/data/gone.txt), plus [Plan](sandbox:/mnt/data/plan.md) again.'] },
      metadata: {}, status: 'finished_successfully', end_turn: true, recipient: 'all' },
  },
};

const calls = [];
globalThis.fetch = async (url) => {
  const u = String(url);
  calls.push(u);
  const ok = (body) => ({ ok: true, status: 200, json: async () => body, headers: { get: () => '' } });
  if (u.includes('/api/auth/session')) return ok({ accessToken: 'tok' });
  if (u.includes('/backend-api/conversation/' + CHAT_ID) && !u.includes('interpreter')) {
    return ok({ mapping, current_node: 'a2', title: 'T', default_model_slug: 'm' });
  }
  if (u.includes('interpreter/download')) {
    const path = decodeURIComponent(new URL(u, 'https://chatgpt.com').searchParams.get('sandbox_path'));
    if (path.endsWith('gone.txt')) return { ok: false, status: 404, json: async () => ({}) };
    return ok({ status: 'success', download_url: 'https://chatgpt.com/backend-api/estuary/content?fn=' + encodeURIComponent(path), metadata: { file_id: 'file-' + path } });
  }
  if (u.includes('/files/file-up1/download')) return { ok: false, status: 403, json: async () => ({}) };
  if (u.includes('/files/file-up2/download')) return ok({ file_name: 'huge.pdf', file_size_bytes: 218_000_000, download_url: 'https://chatgpt.com/backend-api/estuary/content?fn=huge.pdf' });
  if (u.includes('estuary/content')) {
    return { ok: true, status: 200, headers: { get: () => '' }, arrayBuffer: async () => new TextEncoder().encode('body of ' + u).buffer };
  }
  throw new Error('unexpected fetch ' + u);
};

let downloaded = null;
globalThis.window = { __rzn_params: {}, location: { href: 'https://chatgpt.com/c/' + CHAT_ID } };
globalThis.AbortController = globalThis.AbortController;
globalThis.Blob = class { constructor(parts) { this.size = parts.reduce((n, p) => n + (p.length || p.byteLength || 0), 0); } };
globalThis.URL.createObjectURL = () => 'blob:x';
globalThis.URL.revokeObjectURL = () => {};
globalThis.document = {
  createElement: () => ({ style: {}, click() { downloaded = this.download; }, remove() {} }),
  body: { appendChild() {} },
};

const runRead = new Function('arg0', 'arg1', 'return (async()=>{' + readScript + '})()');
const read = await runRead(CHAT_ID, 'transcript');
assert.equal(read.attachments_zip, null, 'the read step returns before anything is zipped');
assert.ok(globalThis.window.__rzn_read_pending.length > 0, 'resolved urls are handed to the zip step');

// The zip step runs in its own eval window, off the state the read step left behind.
const runZip = new Function('return (async()=>{' + zipScript + '})()');
const out = await runZip();

const names = out.attachments_downloaded.filter(a => a.source === 'sandbox').map(a => a.name).sort();
assert.deepEqual(names, ['gone.txt', 'mid file.csv', 'out.zip', 'plan.md'], 'every sandbox link in the thread is enumerated, mid-thread ones included');

const resolved = out.attachments_downloaded.filter(a => a.source === 'sandbox' && a.status === 'resolved');
assert.equal(resolved.length, 3, 'the three reachable files resolve');
assert.equal(resolved.find(a => a.name === 'plan.md').message_id, 'a1', 'each file records the message it came from');

const failed = out.attachments_downloaded.find(a => a.name === 'gone.txt');
assert.equal(failed.status, 'failed', 'an unreachable file is reported, not thrown');
assert.match(failed.error, /404/);

assert.equal(out.attachments_downloaded.filter(a => a.sandbox_path === '/mnt/data/plan.md').length, 1, 'a file linked twice is fetched once');
assert.equal(out.attachments_downloaded.find(a => a.name === 'upload.zip').status, 'failed', 'a 403 user upload degrades gracefully');
assert.equal(out.attachments_downloaded.find(a => a.name === 'huge.pdf').status, 'resolved', 'every upload is fetched, whatever its size');

assert.equal(out.attachments_zip.file_count, 4, 'the reachable files land in one zip');
assert.equal(out.attachments_zip.errors.length, 0);
assert.equal(downloaded, 'chatgpt-attachments-' + CHAT_ID + '.zip', 'exactly one download is triggered');
assert.equal(out.attachments_error, null, 'no stage error');
assert.ok(out.messages.length >= 3, 'the transcript still comes back');

assert.equal(globalThis.window.__rzn_read_envelope, undefined, 'the zip step clears the handoff state');

console.log('chatgpt_read attachment capture: check passed');
