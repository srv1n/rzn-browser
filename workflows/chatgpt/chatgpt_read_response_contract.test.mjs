// Runnable response-contract check. Run: node workflows/chatgpt/chatgpt_read_response_contract.test.mjs
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const workflow = JSON.parse(fs.readFileSync(path.join(here, 'chatgpt_read.json'), 'utf8'));
const script = workflow.steps.find((step) => step.id === 's2').action.inputs.script;
const chatId = 'chat-response-contract';
const msg = (id, role, extra = {}) => ({ id, author: { role }, recipient: 'all', ...extra });
const node = (parent, children, message) => ({ parent, children, message });
const assistant = (id, extra = {}) => msg(id, 'assistant', { status: 'finished_successfully', end_turn: true, content: { content_type: 'text', parts: ['answer'] }, ...extra });

function fixture(kind) {
  const mapping = { root: node(null, ['boundary'], undefined), boundary: node('root', [], msg('boundary-message', 'user', { content: { content_type: 'text', parts: ['prompt'] } })) };
  let current_node = 'boundary';
  const append = (key, message) => { mapping[current_node].children.push(key); mapping[key] = node(current_node, [], message); current_node = key; };
  if (kind === 'completed') {
    append('preamble', assistant('preamble-message', { metadata: { is_thinking_preamble_message: true }, content: { content_type: 'text', parts: ['thinking preamble'] } }));
    append('answer', assistant('answer-message', { content: { content_type: 'text', parts: ['final', 'answer'] } }));
    mapping.boundary.children.push('inactive');
    mapping.inactive = node('boundary', [], assistant('inactive-message', { content: { content_type: 'text', parts: ['wrong branch'] } }));
  } else if (kind === 'ack-then-streaming') {
    append('ack', assistant('ack-message', { content: { content_type: 'text', parts: ['acknowledged'] } }));
    append('stream', assistant('stream-message', { status: 'in_progress', end_turn: false, content: { content_type: 'text', parts: ['working'] } }));
  } else if (kind === 'streaming') append('answer', assistant('answer-message', { status: 'in_progress', end_turn: false, content: { content_type: 'text', parts: ['partial'] } }));
  else if (kind === 'cancelled') append('answer', assistant('answer-message', { metadata: { reasoning_status: 'reasoning_cancelled' }, content: { content_type: 'reasoning_recap', parts: [] } }));
  else if (kind === 'superseded') append('later-user', msg('later-user-message', 'user', { content: { content_type: 'text', parts: ['new prompt'] } }));
  else if (kind === 'not-active') { mapping.inactive = node('root', [], msg('inactive-message', 'user')); }
  return { title: 'Fixture', mapping, current_node };
}

async function run(kind, after = 'boundary-message') {
  const conversation = fixture(kind);
  const calls = [];
  const location = { href: `https://chatgpt.com/c/${chatId}` };
  const params = { download_attachments: false };
  if (after !== null) params.after_message_id = after;
  const window = { location, __rzn_params: params };
  const fetch = async (url) => {
    calls.push(url);
    if (url === '/api/auth/session') return { json: async () => kind === 'no-token' ? ({}) : ({ accessToken: 'test-token' }) };
    assert.equal(url, `/backend-api/conversation/${chatId}`);
    if (kind === 'http-429') return { ok: false, status: 429 };
    if (kind === 'http-500') return { ok: false, status: 500 };
    if (kind === 'fetch-failure') throw new Error('network down');
    return { ok: true, json: async () => conversation };
  };
  const result = await new Function('window', 'fetch', 'arg0', 'arg1', `return (async()=>{${script}})()`)(window, fetch, chatId, 'latest');
  assert.deepEqual(calls, kind === 'no-token' ? ['/api/auth/session'] : ['/api/auth/session', `/backend-api/conversation/${chatId}`], `${kind}: no attachment fetch/download`);
  return result;
}

const completed = await run('completed');
assert.equal(completed.response_state, 'completed');
assert.equal(completed.selected_message_id, 'answer-message');
assert.deepEqual(completed.messages.map((item) => item.id), ['boundary-message', 'preamble-message', 'answer-message']);
assert.equal(completed.messages.find((item) => item.id === completed.selected_message_id).text, 'final\n\nanswer');
assert.deepEqual(completed.attachments_downloaded, []);
const latest = await run('completed', null);
assert.equal('response_state' in latest, false);
assert.equal(latest.mode, 'latest');
assert.ok(Array.isArray(latest.messages));
for (const kind of ['not-started', 'streaming', 'ack-then-streaming']) {
  const result = await run(kind);
  assert.equal(result.response_state, kind === 'not-started' ? 'not_started' : 'streaming');
  assert.equal(result.selected_message_id, null);
}
const cancelled = await run('cancelled');
assert.equal(cancelled.response_state, 'cancelled');
assert.equal(cancelled.response_terminal_reason, 'reasoning_cancelled');
assert.equal(cancelled.selected_message_id, null);
for (const [kind, after, code] of [['superseded', 'boundary-message', 'BOUNDARY_SUPERSEDED'], ['not-started', 'missing-message', 'BOUNDARY_NOT_FOUND'], ['not-active', 'inactive-message', 'BOUNDARY_NOT_ACTIVE']]) {
  const failure = await run(kind, after);
  assert.equal(failure.success, false);
  assert.equal(failure.error_code, code);
}
for (const [kind, code] of [['no-token', 'AUTH_REQUIRED'], ['http-429', 'RATE_LIMITED'], ['http-500', 'CONVERSATION_API_FAILED'], ['fetch-failure', 'CONVERSATION_API_FAILED']]) {
  const failure = await run(kind);
  assert.equal(failure.success, false);
  assert.equal(failure.error_code, code);
}
console.log('chatgpt_read after_message_id response contract: check passed');
