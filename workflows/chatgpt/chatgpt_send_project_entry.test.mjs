// Runnable regression check for Project landing activation.
// Run: node workflows/chatgpt/chatgpt_send_project_entry.test.mjs
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const wf = JSON.parse(fs.readFileSync(path.join(here, 'chatgpt_send.json'), 'utf8'));
const routeStep = wf.steps.find((candidate) => candidate.id === 's2-route');
const step = wf.steps.find((candidate) => candidate.id === 's3');
const routeScript = routeStep.action.inputs.script;
const script = step.action.inputs.script;

class Element {
  constructor(tag, attrs = {}, text = '') {
    this.tag = tag;
    this.attrs = { ...attrs };
    this.text = text;
    this.events = [];
    this.onclick = null;
  }
  get tagName() { return this.tag.toUpperCase(); }
  get textContent() { return this.text; }
  getAttribute(name) { return Object.hasOwn(this.attrs, name) ? String(this.attrs[name]) : null; }
  scrollIntoView() {}
  getBoundingClientRect() { return { width: 100, height: 20 }; }
  click() { this.events.push({ type: 'click' }); this.onclick?.({ type: 'click' }); }
  dispatchEvent(event) { this.events.push(event); if (event.type === 'click') this.onclick?.(event); return true; }
}
class Event_ { constructor(type) { this.type = type; } }

const run = async ({ projectId = '', nodes = [] } = {}) => {
  const document = {
    querySelector: (selector) => selector === '#prompt-textarea' ? nodes.find((node) => node.attrs.id === 'prompt-textarea') || null : null,
    querySelectorAll: (selector) => nodes.filter((node) => selector.split(',').some((part) => {
      const candidate = part.trim();
      if (candidate.includes("[data-testid='create-new-chat-button']")) return node.attrs['data-testid'] === 'create-new-chat-button';
      if (candidate.includes("[data-testid='project-new-chat-button']")) return node.attrs['data-testid'] === 'project-new-chat-button';
      if (candidate.includes("[aria-label='New chat'")) return /^new chat$/i.test(node.attrs['aria-label'] || '');
      if (candidate.includes("[aria-label^='New chat in '")) return /^new chat in /i.test(node.attrs['aria-label'] || '');
      if (candidate === 'button') return node.tag === 'button';
      if (candidate === 'a') return node.tag === 'a';
      if (candidate === '[role=button]') return node.attrs.role === 'button';
      if (candidate === 'div') return node.tag === 'div';
      if (candidate === 'textarea') return node.tag === 'textarea';
      return false;
    })),
  };
  const location = { pathname: `/g/${projectId}/project` };
  const getComputedStyle = () => ({ visibility: 'visible', display: 'block' });
  const runScript = new Function('document', 'location', 'URLSearchParams', 'Element', 'getComputedStyle', 'PointerEvent', 'MouseEvent', 'arg0', 'arg1', `return (async()=>{${script}})()`);
  return runScript(document, location, URLSearchParams, Element, getComputedStyle, Event_, Event_, '', projectId);
};

const projectId = 'g-p-6a1d013afa788191bb0fe587a68f0b68';
const projectUrl = `https://chatgpt.com/g/${projectId}-kurpod/project`;
const projectRow = new Element('div', { role: 'button' }, 'Kurpod');
const projectHome = new Element('button', { 'aria-label': 'Open project home' });
projectRow.closest = () => ({ querySelector: () => projectHome });
let projectHomeClicks = 0;
projectHome.onclick = () => { projectHomeClicks += 1; };
const routeDocument = {
  querySelectorAll: (selector) => selector.includes('[role=button]') ? [projectRow] : [],
};
const runRouteScript = new Function('document', 'location', 'URL', 'getComputedStyle', 'arg0', 'arg1', 'arg2', `return (async()=>{${routeScript}})()`);
const routed = await runRouteScript(routeDocument, { assign() {} }, URL, () => ({ visibility: 'visible', display: 'block' }), '', projectId, projectUrl);
assert.equal(wf.steps[0].action.inputs.url, 'https://chatgpt.com/', 'workflow enters through ChatGPT home');
assert.equal(routed.triggered, 'clicked_project_home');
assert.equal(projectHomeClicks, 1, 'Project home is opened through the sidebar once');

const newButton = new Element('button', { 'data-testid': 'create-new-chat-button' }, 'New chat in Kurpod');
let clicks = 0;
newButton.onclick = () => { clicks += 1; };
const activated = await run({ projectId, nodes: [newButton] });
assert.equal(activated.triggered, 'clicked_project_new_chat');
assert.equal(clicks, 1, 'Project New control is clicked once');

const textControl = new Element('div', {}, 'New chat in Kurpod');
let textClicks = 0;
textControl.onclick = () => { textClicks += 1; };
const activatedText = await run({ projectId, nodes: [textControl] });
assert.equal(activatedText.triggered, 'clicked_project_new_chat');
assert.equal(textClicks, 1, 'screenshot-shaped New chat control is clicked once');

const composer = new Element('div', { id: 'prompt-textarea' });
const alreadyReady = await run({ projectId, nodes: [composer, newButton] });
assert.equal(alreadyReady.triggered, 'already_on_project_composer');
assert.equal(clicks, 1, 'an already-mounted composer does not click New again');

await assert.rejects(
  () => run({ projectId, nodes: [new Element('button', {}, 'Try again')] }),
  /project_new_chat_not_found/,
  'a stuck landing fails with an actionable Project-New error',
);

console.log('chatgpt_send Project entry: all checks passed');
