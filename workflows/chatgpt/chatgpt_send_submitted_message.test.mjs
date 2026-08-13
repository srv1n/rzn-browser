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
  'old-user': { parent: 'root', children: ['old-assistant'], message: { id: 'old-user-id', author: { role: 'user' } } },
  'old-assistant': { parent: 'old-user', children: ['active-user', 'stale-user'], message: { id: 'old-assistant-id', author: { role: 'assistant' } } },
  'active-user': { parent: 'old-assistant', children: ['active-assistant'], message: { id: 'submitted-user-id', author: { role: 'user' } } },
  'stale-user': { parent: 'old-assistant', children: ['stale-assistant'], message: { id: 'stale-user-id', author: { role: 'user' } } },
  'active-assistant': { parent: 'active-user', children: [], message: { id: 'active-assistant-id', author: { role: 'assistant' } } },
  'stale-assistant': { parent: 'stale-user', children: [], message: { id: 'stale-assistant-id', author: { role: 'assistant' } } },
};

const calls = [];
let conversationNodes = ['old-assistant', 'active-assistant'];
const fetch = async (url) => {
  calls.push(url);
  if (url === '/api/auth/session') return { json: async () => ({ accessToken: 'test-token' }) };
  assert.equal(url, `/backend-api/conversation/${chatId}`);
  return { ok: true, json: async () => ({ mapping, current_node: conversationNodes.shift() }) };
};
const document = { body: { innerText: '' }, title: 'test' };
const location = { href: `https://chatgpt.com/c/${chatId}`, pathname: `/c/${chatId}` };
const window = { __rzn_chatgpt_picker: { desiredModel: 'GPT-5.6 Sol', desiredEffort: 'Pro' }, __rzn_chatgpt_pre_send_user_id: 'old-user-id' };
const run = new Function('document', 'window', 'location', 'URLSearchParams', 'fetch', `return (async()=>{${script}})()`);
const result = await run(document, window, location, URLSearchParams, fetch);

assert.equal(result.chat_id, chatId);
assert.equal(result.chat_url, location.href);
assert.equal(result.submitted_message_id, 'submitted-user-id');
assert.deepEqual(calls, ['/api/auth/session', `/backend-api/conversation/${chatId}`, `/backend-api/conversation/${chatId}`]);

conversationNodes = ['old-assistant', 'old-assistant', 'old-assistant', 'old-assistant', 'old-assistant', 'old-assistant', 'old-assistant', 'old-assistant'];
const unavailable = await run(document, window, location, URLSearchParams, fetch);
assert.equal(unavailable.success, false);
assert.equal(unavailable.error_code, 'MESSAGE_BOUNDARY_UNAVAILABLE');
assert.match(unavailable.error_msg, /unchanged from pre-send boundary/);
console.log('chatgpt_send submitted message extraction: check passed');
