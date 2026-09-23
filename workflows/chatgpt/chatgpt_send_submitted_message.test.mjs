// Runnable regression check for s17's active-chain message extraction.
// Run: node workflows/chatgpt/chatgpt_send_submitted_message.test.mjs
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const wf = JSON.parse(fs.readFileSync(path.join(here, 'chatgpt_send.json'), 'utf8'));
const script = wf.steps.find((step) => step.id === 's17').action.inputs.script;
const chatId = 'chat-branch';
const mapping = {
  root: { parent: null, children: ['old-user'] },
  'old-user': { parent: 'root', children: ['old-assistant'], message: { id: 'old-user-id', author: { role: 'user' }, create_time: 990, content: { parts: ['old prompt'] } } },
  'old-assistant': { parent: 'old-user', children: ['concurrent-user', 'stale-user'], message: { id: 'old-assistant-id', author: { role: 'assistant' } } },
  'concurrent-user': { parent: 'old-assistant', children: ['concurrent-assistant'], message: { id: 'concurrent-user-id', author: { role: 'user' }, create_time: 999, content: { parts: ['target prompt'] } } },
  'concurrent-assistant': { parent: 'concurrent-user', children: ['active-user'], message: { id: 'concurrent-assistant-id', author: { role: 'assistant' } } },
  'active-user': { parent: 'concurrent-assistant', children: ['active-assistant'], message: { id: 'submitted-user-id', author: { role: 'user' }, create_time: 1002, content: { parts: ['target prompt'] } } },
  'stale-user': { parent: 'old-assistant', children: ['stale-assistant'], message: { id: 'stale-user-id', author: { role: 'user' }, create_time: 1001, content: { parts: ['wrong branch'] } } },
  'active-assistant': { parent: 'active-user', children: [], message: { id: 'active-assistant-id', author: { role: 'assistant' } } },
  'stale-assistant': { parent: 'stale-user', children: [], message: { id: 'stale-assistant-id', author: { role: 'assistant' } } },
};

const calls = [];
let conversationNodes = ['concurrent-assistant', 'active-assistant'];
const fetch = async (url) => {
  calls.push(url);
  if (url === '/api/auth/session') return { json: async () => ({ accessToken: 'test-token' }) };
  assert.equal(url, `/backend-api/conversation/${chatId}`);
  return { ok: true, json: async () => ({ mapping, current_node: conversationNodes.shift() }) };
};
let routeReadyAt = null;
const document = { body: { innerText: '' }, title: 'test' };
const location = {
  get href() { return routeReadyAt !== null && clock < routeReadyAt ? 'https://chatgpt.com/' : `https://chatgpt.com/c/${chatId}`; },
  get pathname() { return routeReadyAt !== null && clock < routeReadyAt ? '/' : `/c/${chatId}`; },
};
const window = {
  __rzn_chatgpt_picker: { desiredModel: 'Latest', desiredEffort: 'Pro', modelSelection: { applied: false, model_selected: 'GPT-5.6 Sol', effort_selected: 'High', model_display: '5.6 High' } },
  __rzn_chatgpt_send_marker: { pre_user_id: 'old-user-id', message_text: 'target prompt', clicked_at: 1000 },
};
// Drive the poll loop off a fake clock so the deadline path runs instantly.
// Timers are queued, not fired inline: the abort timers tfetch arms are cleared
// in its finally block, so only the real backoff sleeps ever advance the clock.
let clock = 1_000_000;
let timers = [];
let timerSeq = 0;
const Date_ = { now: () => clock };
const setTimeout_ = (cb, ms) => { const id = ++timerSeq; timers.push({ id, at: clock + (ms || 0), cb }); return id; };
const clearTimeout_ = (id) => { timers = timers.filter((timer) => timer.id !== id); };
const run = new Function('document', 'window', 'location', 'URLSearchParams', 'fetch', 'Date', 'setTimeout', 'clearTimeout', 'AbortController', `return (async()=>{${script}})()`);

// Run the script to completion, advancing the clock to the next due timer
// whenever it parks. Mirrors what an event loop does, minus the waiting.
const call = async () => {
  timers = [];
  let settled = false;
  let value;
  let failure;
  const pending = run(document, window, location, URLSearchParams, fetch, Date_, setTimeout_, clearTimeout_, AbortController)
    .then((result) => { value = result; }, (error) => { failure = error; })
    .finally(() => { settled = true; });
  for (let guard = 0; guard < 10_000 && !settled; guard += 1) {
    for (let drain = 0; drain < 50; drain += 1) await Promise.resolve();
    if (settled || !timers.length) break;
    const next = timers.reduce((earliest, timer) => (timer.at < earliest.at ? timer : earliest));
    timers = timers.filter((timer) => timer.id !== next.id);
    clock = Math.max(clock, next.at);
    next.cb();
  }
  await pending;
  assert.ok(settled, 'script never settled');
  if (failure) throw failure;
  return value;
};
const result = await call();

assert.equal(result.chat_id, chatId);
assert.equal(result.chat_url, location.href);
assert.equal(result.submitted_message_id, 'submitted-user-id');
assert.deepEqual(calls, ['/api/auth/session', `/backend-api/conversation/${chatId}`, `/backend-api/conversation/${chatId}`]);

// A new-chat route can appear after s16's six-second wait; s17 must keep
// waiting for it, then apply the same exact active-chain/message boundary.
calls.length = 0;
conversationNodes = ['active-assistant'];
const routeRaceStartedAt = clock;
routeReadyAt = routeRaceStartedAt + 7_000;
const routeRace = await call();
routeReadyAt = null;
assert.ok(clock - routeRaceStartedAt > 6_000, 'route race must cross the old s16 boundary');
assert.equal(routeRace.chat_id, chatId);
assert.equal(routeRace.model_observed, 'GPT-5.6 Sol / High', 'report observed values, never echo the request as proof');
assert.deepEqual(routeRace.model_selection, window.__rzn_chatgpt_picker.modelSelection);
assert.equal(routeRace.submitted_message_id, 'submitted-user-id');
assert.deepEqual(calls, ['/api/auth/session', `/backend-api/conversation/${chatId}`]);

// The turn never lands: the loop must give up on its own deadline, not run forever,
// and must back off rather than hammering the conversation API.
calls.length = 0;
const startedAt = clock;
conversationNodes = Array(500).fill('concurrent-assistant');
const unavailable = await call();
assert.equal(unavailable.success, false);
assert.equal(unavailable.error_code, 'MESSAGE_BOUNDARY_UNAVAILABLE');
assert.match(unavailable.error_msg, /predates the send click/);
const elapsed = clock - startedAt;
assert.ok(elapsed <= 20_000, `poll loop overran its deadline: ${elapsed}ms`);
const polls = calls.filter((url) => url !== '/api/auth/session').length;
assert.ok(polls <= 15, `poll loop hit the conversation API ${polls} times`);
assert.ok(polls >= 5, `poll loop gave up after only ${polls} attempts`);

// The composer is a ProseMirror surface: what comes back from the API is
// re-serialized, so line breaks and spacing need not survive byte-for-byte.
// Whitespace differences must still bind; different prose must not.
mapping['active-user'].message.content.parts = ['target\n\n  prompt  '];
conversationNodes = ['active-assistant'];
const reflowed = await call();
assert.equal(reflowed.submitted_message_id, 'submitted-user-id', 'whitespace-only differences must still bind');

// ChatGPT's rich-text renderer expands only identical-label links and adds its
// own Markdown escapes while flattening a submitted multiline prompt.
const representative = [
  'Title: Incremental editor maintenance',
  '@GitHub https://github.com/srv1n/cinta',
  'Contact <sarav@gmx.com>; run build_tiptap.sh and check_tiptap_bundle_fresh.sh.',
  '```json',
  '{ "pr_url": "https://github.com/OWNER/REPO/pull/123" }',
  '```',
].join('\n\n');
const rendered = [
  'Title: Incremental editor maintenance',
  '@GitHub [https://github.com/srv1n/cinta](https://github.com/srv1n/cinta)',
  'Contact \\<sarav\\@gmx.com>; run build\\_tiptap.sh and check\\_tiptap\\_bundle\\_fresh.sh.',
  '\\`\\`\\`json',
  '{ "pr\\_url": "[https://github.com/OWNER/REPO/pull/123](https://github.com/OWNER/REPO/pull/123)" }',
  '\\`\\`\\`',
].join('  ');
window.__rzn_chatgpt_send_marker.message_text = representative;
mapping['active-user'].message.content.parts = [rendered];
conversationNodes = ['active-assistant'];
const serialized = await call();
assert.equal(serialized.submitted_message_id, 'submitted-user-id', 'known rich-text serialization must still bind');

// Serialization is directional but each observed renderer change is optional:
// links can expand while code fences stay literal and only underscores escape.
const mixed = rendered.replace(/\\(?=`)/g, '').replace(/\\</g, '<').replace(/\\@/g, '@');
mapping['active-user'].message.content.parts = [mixed];
conversationNodes = ['active-assistant'];
const mixedSerialization = await call();
assert.equal(mixedSerialization.submitted_message_id, 'submitted-user-id', 'mixed rich-text serialization must bind');

// The same directional rule covers the complete ASCII punctuation range used
// by ChatGPT's Markdown serializer, including headings and list markers seen
// in the failed handoff (not just links, underscores, and code fences).
const punctuationPrompt = 'Heading # Task\n\n- bullet\n\nPunctuation: ! * + = : ? [x] {y}';
const punctuationRendered = 'Heading \\# Task  \\- bullet  Punctuation: \\! \\* \\+ \\= \\: \\? \\[x\\] \\{y\\}';
window.__rzn_chatgpt_send_marker.message_text = punctuationPrompt;
mapping['active-user'].message.content.parts = [punctuationRendered];
conversationNodes = ['active-assistant'];
const punctuationSerialization = await call();
assert.equal(punctuationSerialization.submitted_message_id, 'submitted-user-id', 'ASCII punctuation escapes must bind');

// The same metadata must make an after-click mismatch recoverable without
// leaking the full prompt or pretending submission was confirmed.
mapping['active-user'].message.content.parts = [rendered.replace('Incremental editor', 'Different editor')];
conversationNodes = Array(500).fill('active-assistant');
const postClickMismatch = await call();
assert.equal(postClickMismatch.success, false);
assert.equal(postClickMismatch.error_code, 'MESSAGE_BOUNDARY_UNAVAILABLE');
assert.equal(postClickMismatch.send_clicked, true);
assert.equal(postClickMismatch.chat_id, chatId);
assert.equal(postClickMismatch.chat_url, location.href);
assert.match(postClickMismatch.error_msg, /text does not match send marker/);
assert.ok(postClickMismatch.error_msg.length < 320, 'mismatch diagnostics stay bounded');
assert.ok(!postClickMismatch.error_msg.includes(representative), 'mismatch diagnostics must not include the full prompt');

// A literal backslash in the requested prompt cannot be silently deleted.
window.__rzn_chatgpt_send_marker.message_text = 'literal \\_ marker';
mapping['active-user'].message.content.parts = ['literal _ marker'];
conversationNodes = ['active-assistant'];
const deletedLiteralEscape = await call();
assert.equal(deletedLiteralEscape.error_code, 'MESSAGE_BOUNDARY_UNAVAILABLE');
assert.equal(deletedLiteralEscape.send_clicked, true);

// A link with the right label but a different destination is not equivalent.
window.__rzn_chatgpt_send_marker.message_text = 'See https://example.com/a';
mapping['active-user'].message.content.parts = ['See [https://example.com/a](https://example.com/b)'];
conversationNodes = ['active-assistant'];
const changedLinkDestination = await call();
assert.equal(changedLinkDestination.error_code, 'MESSAGE_BOUNDARY_UNAVAILABLE');
assert.equal(changedLinkDestination.send_clicked, true);

// A marker without a click timestamp is never proof of an external send.
window.__rzn_chatgpt_send_marker.message_text = 'target prompt';
window.__rzn_chatgpt_send_marker.clicked_at = null;
mapping['active-user'].message.content.parts = ['target prompt'];
conversationNodes = ['active-assistant'];
const unclicked = await call();
assert.equal(unclicked.success, false);
assert.equal(unclicked.send_clicked, false);
assert.equal(unclicked.chat_id, chatId);
assert.equal(unclicked.chat_url, location.href);
window.__rzn_chatgpt_send_marker.clicked_at = 1000;

mapping['active-user'].message.content.parts = ['a completely different prompt'];
conversationNodes = Array(500).fill('active-assistant');
const mismatched = await call();
assert.equal(mismatched.error_code, 'MESSAGE_BOUNDARY_UNAVAILABLE');
assert.match(mismatched.error_msg, /text does not match send marker/);
mapping['active-user'].message.content.parts = ['target prompt'];

// The reason this step failed in the field: the script's own worst case has to
// fit inside the step budget, or the harness kills it mid-poll instead of
// letting it return a readable MESSAGE_BOUNDARY_UNAVAILABLE.
const bind = wf.steps.find((step) => step.id === 's17');
const routeWaitMs = Number(/const routeDeadline=Date\.now\(\)\+(\d+)/.exec(script)?.[1]);
const deadlineMs = Number(/const deadline=Date\.now\(\)\+(\d+)/.exec(script)?.[1]);
assert.ok(Number.isFinite(routeWaitMs), 's17 must bound its route wait with an explicit deadline');
assert.ok(Number.isFinite(deadlineMs), 's17 must bound its poll loop with an explicit deadline');
const slowestFetchMs = Math.max(...[...script.matchAll(/tfetch\([^;]*?,\s*(\d+)\)/g)].map((m) => Number(m[1])));
assert.ok(
  routeWaitMs + deadlineMs + slowestFetchMs <= bind.timeout_ms,
  `s17 can run ${routeWaitMs + deadlineMs + slowestFetchMs}ms but the step allows ${bind.timeout_ms}ms`,
);

const submit = wf.steps.find((step) => step.id === 's15');
assert.deepEqual(submit.action.inputs.args, ['{message_text}', '{chat_id}']);
assert.ok(submit.action.inputs.script.indexOf('clicked_at=Date.now()/1000') < submit.action.inputs.script.indexOf('realClick(send)'));
console.log('chatgpt_send submitted message extraction: check passed');
