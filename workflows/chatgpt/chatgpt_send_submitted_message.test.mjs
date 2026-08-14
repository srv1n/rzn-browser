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
const document = { body: { innerText: '' }, title: 'test' };
const location = { href: `https://chatgpt.com/c/${chatId}`, pathname: `/c/${chatId}` };
const window = {
  __rzn_chatgpt_picker: { desiredModel: 'GPT-5.6 Sol', desiredEffort: 'Pro' },
  __rzn_chatgpt_send_marker: { pre_user_id: 'old-user-id', message_text: 'target prompt', clicked_at: 1000 },
};
const run = new Function('document', 'window', 'location', 'URLSearchParams', 'fetch', `return (async()=>{${script}})()`);
const result = await run(document, window, location, URLSearchParams, fetch);

assert.equal(result.chat_id, chatId);
assert.equal(result.chat_url, location.href);
assert.equal(result.submitted_message_id, 'submitted-user-id');
assert.deepEqual(calls, ['/api/auth/session', `/backend-api/conversation/${chatId}`, `/backend-api/conversation/${chatId}`]);

conversationNodes = Array(8).fill('concurrent-assistant');
const unavailable = await run(document, window, location, URLSearchParams, fetch);
assert.equal(unavailable.success, false);
assert.equal(unavailable.error_code, 'MESSAGE_BOUNDARY_UNAVAILABLE');
assert.match(unavailable.error_msg, /does not match send marker/);

const submit = wf.steps.find((step) => step.id === 's15');
assert.deepEqual(submit.action.inputs.args, ['{message_text}', '{chat_id}']);
assert.ok(submit.action.inputs.script.indexOf('clicked_at=Date.now()/1000') < submit.action.inputs.script.indexOf('realClick(send)'));
console.log('chatgpt_send submitted message extraction: check passed');
