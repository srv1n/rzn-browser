// Runnable regression check for s15's ProseMirror entry and pre-send boundary.
// Run: node workflows/chatgpt/chatgpt_send_composer.test.mjs
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const wf = JSON.parse(fs.readFileSync(path.join(here, 'chatgpt_send.json'), 'utf8'));
const script = wf.steps.find((step) => step.id === 's15').action.inputs.script;

class Element {
  constructor(tag, attrs = {}, text = '') {
    this.tag = tag;
    this.attrs = { ...attrs };
    this.text = text;
    this.childNodes = [];
    this.parentElement = null;
    this.events = [];
    this.onclick = null;
  }
  get nodeType() { return 1; }
  get tagName() { return this.tag.toUpperCase(); }
  get textContent() { return this.childNodes.length ? this.childNodes.map((node) => node.textContent).join('') : this.text; }
  set textContent(value) { this.childNodes = value ? [new TextNode(String(value))] : []; this.text = ''; }
  getAttribute(name) { return Object.hasOwn(this.attrs, name) ? String(this.attrs[name]) : null; }
  setAttribute(name, value) { this.attrs[name] = String(value); }
  hasAttribute(name) { return Object.hasOwn(this.attrs, name); }
  append(...nodes) { nodes.forEach((node) => { node.parentElement = this; this.childNodes.push(node); }); return this; }
  replaceChildren(...nodes) { this.childNodes = []; this.text = ''; return this.append(...nodes); }
  contains(node) { for (let current = node; current; current = current.parentElement) if (current === this) return true; return false; }
  closest(selector) { return selector === 'form' ? this.ancestor('form') : null; }
  ancestor(tag) { for (let current = this.parentElement; current; current = current.parentElement) if (current.tag === tag) return current; return null; }
  matches(selector) { return selector === ':disabled' && this.attrs.disabled !== undefined; }
  querySelectorAll(selector) { return descendants(this).filter((node) => matches(node, selector)); }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  scrollIntoView() {}
  focus() { this.events.push({ type: 'focus' }); }
  getBoundingClientRect() { return { left: 0, top: 0, width: 100, height: 20 }; }
  dispatchEvent(event) { this.events.push(event); if (event.type === 'click' && this.onclick) this.onclick(event); return true; }
}

class TextNode {
  constructor(value) { this.nodeValue = value; this.parentElement = null; }
  get nodeType() { return 3; }
  get textContent() { return this.nodeValue; }
}

class TextArea extends Element {}
class Input extends Element {}
class InputEvent_ { constructor(type, init = {}) { this.type = type; Object.assign(this, init); } }
class Event_ { constructor(type, init = {}) { this.type = type; Object.assign(this, init); } }
class MouseEvent_ extends Event_ {}
class PointerEvent_ extends Event_ {}

const descendants = (node) => (node.childNodes || []).flatMap((child) => [child, ...descendants(child)]);
function matches(node, selector) {
  if (!node.attrs) return false;
  return selector.split(',').some((part) => {
    const p = part.trim();
    if (p === '#prompt-textarea') return node.attrs.id === 'prompt-textarea';
    if (p === 'div.ProseMirror[contenteditable=true]' || p === 'div[role=textbox][contenteditable=true]') {
      return node.tag === 'div' && node.attrs.contenteditable === 'true' && (node.attrs.class || '').split(/\s+/).includes('ProseMirror') || node.tag === 'div' && node.attrs.role === 'textbox' && node.attrs.contenteditable === 'true';
    }
    if (p === 'textarea[placeholder*="Message" i]') return node.tag === 'textarea';
    if (p === 'button[data-testid="send-button"]') return node.tag === 'button' && node.attrs['data-testid'] === 'send-button';
    if (p === 'button[aria-label="Send prompt" i]' || p === 'button[aria-label="Send message" i]') return node.tag === 'button' && /^send (prompt|message)$/i.test(node.attrs['aria-label'] || '');
    if (p === 'button' || p === '[role=button]') return node.tag === 'button' || node.attrs.role === 'button';
    return false;
  });
}

function makePage({ serialized, execResult = true, textarea = false } = {}) {
  const doc = new Element('body');
  const form = new Element('form');
  const composer = textarea
    ? new TextArea('textarea', { id: 'prompt-textarea' })
    : new Element('div', { id: 'prompt-textarea', class: 'ProseMirror', contenteditable: 'true', role: 'textbox' });
  if (textarea) composer.append(new TextNode('stale default value'));
  const send = new Element('button', { 'data-testid': 'send-button', 'aria-label': 'Send prompt' }, 'Send');
  let clicked = 0;
  send.onclick = () => { clicked += 1; };
  form.append(composer, send);
  doc.append(form);
  const setSerialized = (value) => composer.replaceChildren(...value.split('\n').map((line) => new Element('p').append(...(line ? [new TextNode(line)] : []))));
  const document = {
    body: doc,
    querySelector: (selector) => doc.querySelector(selector),
    querySelectorAll: (selector) => doc.querySelectorAll(selector),
    execCommand: (command, _showUi, value) => {
      assert.equal(command, 'insertText');
      if (!execResult) return false;
      setSerialized(serialized ?? value);
      composer.dispatchEvent(new InputEvent_('input', { inputType: 'insertText', data: value }));
      return true;
    },
    createRange: () => ({ selectNodeContents() {}, }),
  };
  const window = { getSelection: () => ({ removeAllRanges() {}, addRange() {} }) };
  const location = { href: 'https://chatgpt.com/', pathname: '/' };
  const run = new Function('document', 'window', 'location', 'Element', 'PointerEvent', 'MouseEvent', 'InputEvent', 'Event', 'HTMLTextAreaElement', 'HTMLInputElement', 'getComputedStyle', 'arg0', 'arg1', `return (async()=>{${script}})()`);
  return {
    composer,
    get clicked() { return clicked; },
    run: (message) => run(document, window, location, Element, PointerEvent_, MouseEvent_, InputEvent_, Event_, TextArea, Input, () => ({ visibility: 'visible', display: 'block' }), message, ''),
  };
}

const multiline = 'Title: Incremental editor maintenance\n\n- preserve paragraphs\n- keep tables';
const page = makePage({ serialized: multiline });
const result = await page.run(multiline);
assert.equal(result.message_typed, true);
assert.equal(result.send_clicked, true);
assert.equal(page.clicked, 1, 'matching serialized composer text sends exactly once');
assert.equal(page.composer.childNodes.length, 4, 'multiline text enters as separate ProseMirror blocks');

const mismatchPage = makePage({ serialized: 'wrong content' });
await assert.rejects(() => mismatchPage.run(multiline), /composer_text_mismatch: expected 73 normalized chars.*saw 13/);
assert.equal(mismatchPage.clicked, 0, 'serialized mismatch fails before clicking Send');

const fallbackPage = makePage({ execResult: false });
const fallback = await fallbackPage.run(multiline);
assert.equal(fallbackPage.clicked, 1, 'DOM fallback remains available when execCommand is unavailable');
assert.equal(fallback.send_clicked, true);

const textareaPage = makePage({ textarea: true });
const textareaResult = await textareaPage.run(multiline);
assert.equal(textareaResult.send_clicked, true, 'native input value wins over stale default child text');
assert.equal(textareaPage.composer.value, multiline);

console.log('chatgpt_send composer entry: all checks passed');
