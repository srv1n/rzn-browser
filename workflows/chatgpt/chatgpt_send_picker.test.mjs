// Runnable check for the chatgpt/send model-picker steps (s6/s9/s12).
// Simulates ChatGPT's legacy and compact composer pickers.
// Run: node workflows/chatgpt/chatgpt_send_picker.test.mjs
import assert from 'node:assert';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const wf = JSON.parse(fs.readFileSync(path.join(here, 'chatgpt_send.json'), 'utf8'));
const scriptOf = (id) => wf.steps.find((s) => s.id === id).action.inputs.script;

// ---- minimal DOM ------------------------------------------------------------
class El {
  constructor(tag, attrs = {}, text = '') {
    this.tag = tag;
    this.attrs = attrs;
    this.text = text;
    this.children = [];
    this.parent = null;
    this.onclick = null;
  }
  get id() { return this.attrs.id || ''; }
  get tagName() { return this.tag.toUpperCase(); }
  contains(other) { let n = other; while (n) { if (n === this) return true; n = n.parent; } return false; }
  set id(v) { this.attrs.id = v; }
  // real textContent concatenates child text with no separator
  get textContent() { return this.children.length ? this.children.map((c) => c.textContent).join('') : this.text; }
  getAttribute(n) { return n in this.attrs ? String(this.attrs[n]) : null; }
  setAttribute(n, v) { this.attrs[n] = v; }
  append(...kids) { for (const k of kids) { k.parent = this; this.children.push(k); } return this; }
  clear() { this.children = []; }
  scrollIntoView() {}
  focus() {}
  getBoundingClientRect() { return { width: 100, height: 20 }; }
  closest(sel) { let n = this; while (n) { if (matches(n, sel)) return n; n = n.parent; } return null; }
  dispatchEvent(ev) {
    if (ev.type === 'click' && this.onclick) this.onclick(this);
    if (ev.type === 'keydown' && this.onkeydown) this.onkeydown(ev);
    return true;
  }
  querySelectorAll(sel) { return descendants(this).filter((n) => matches(n, sel)); }
  querySelector(sel) { return this.querySelectorAll(sel)[0] || null; }
}
const descendants = (n) => n.children.flatMap((c) => [c, ...descendants(c)]);

// selector support: comma groups of tag#id.class[attr][attr=v][attr*=v]
function matches(node, sel) {
  return sel.split(',').some((part) => {
    const p = part.trim();
    const m = p.match(/^([a-zA-Z]*)((?:[#.][\w-]+)*)((?:\[[^\]]+\])*)$/);
    if (!m) return false;
    const [, tag, simple, attrs] = m;
    if (tag && node.tag !== tag) return false;
    for (const s of simple.match(/[#.][\w-]+/g) || []) {
      if (s[0] === '#' && node.attrs.id !== s.slice(1)) return false;
      if (s[0] === '.' && !String(node.attrs.class || '').split(/\s+/).includes(s.slice(1))) return false;
    }
    for (const a of attrs.match(/\[[^\]]+\]/g) || []) {
      const body = a.slice(1, -1);
      const mm = body.match(/^([\w-]+)(\*?=)?"?([^"]*)"?$/);
      const [, name, op, val] = mm;
      const actual = node.getAttribute(name);
      if (actual === null) return false;
      if (op === '=' && actual !== val) return false;
      if (op === '*=' && !actual.includes(val)) return false;
    }
    return true;
  });
}

// ---- the simulated picker ---------------------------------------------------
// shape: how the picker renders its rows.
//   'menuitem'    ground truth, captured 2026-08-07 from the live page:
//                 <div role=menuitem aria-haspopup=menu> whose label and value sit
//                 in separate child divs, so textContent is "ModelGPT-5.6 Sol" with
//                 NO separating space. A /^Model\b/ match fails on that string.
//   'button-aria' plain <button aria-label="Model"> whose text is only the value
//   'wrapped'     role=menuitem rows nested inside a plain container div
//   'compact'     <div role=menuitem aria-label="Select model">Medium</div>
function buildPage({ models, efforts, model, effort, commit = true, shape = 'menuitem' }) {
  const state = { model, effort };
  const doc = new El('body');
  const form = new El('form');
  const pill = new El('button', { class: '__composer-pill', 'aria-haspopup': 'menu' }, state.effort);
  form.append(pill, new El('input', { id: 'upload-files' }));
  doc.append(form);

  const menu = new El('div', { role: 'menu' });
  const row = (label, value) => {
    const r = shape === 'button-aria'
      ? new El('button', { 'aria-label': label, 'aria-haspopup': 'menu' }, value)
      // label and value are sibling nodes -> textContent concatenates with no space
      : new El('div', { role: 'menuitem', 'aria-haspopup': 'menu' }).append(
        new El('div', { class: 'truncate' }, label),
        new El('div', { class: 'trailing' }, value),
      );
    r.onclick = () => {
      menu.querySelectorAll('[role=menuitemradio]').forEach((n) => { n.parent.children = n.parent.children.filter((c) => c !== n); });
      const opts = label === 'Model' ? models : efforts;
      menu.append(...opts.map((o) => {
        const el = new El('div', { role: 'menuitemradio', 'aria-checked': String(o === state[label.toLowerCase()]) }, o);
        el.onclick = () => { if (commit) state[label.toLowerCase()] = o; closeAll(); };
        return el;
      }));
    };
    return r;
  };
  const compact = () => {
    const r = new El('div', { role: 'menuitem', 'aria-label': 'Select model' }, state.effort);
    const power = new El('div', { role: 'menuitem', 'aria-label': 'Power' });
    const slider = new El('span', { role: 'slider', 'aria-valuemin': '0', 'aria-valuemax': String(efforts.length - 1), 'aria-valuenow': String(efforts.indexOf(state.effort)) });
    power.append(slider);
    power.onkeydown = (ev) => {
      const current = Number(slider.getAttribute('aria-valuenow'));
      const next = Math.max(0, Math.min(efforts.length - 1, current + (ev.key === 'ArrowRight' ? 1 : ev.key === 'ArrowLeft' ? -1 : 0)));
      slider.setAttribute('aria-valuenow', String(next));
      if (commit) { state.effort = efforts[next]; r.text = state.effort; }
    };
    r.onclick = () => {};
    menu.append(power, ...models.map((o) => new El('div', { role: 'menuitemradio', 'aria-checked': String(o === 'GPT-5.6 Sol') }, o)));
    return r;
  };
  const openAdvanced = () => {
    menu.clear();
    // the live picker keeps an inert copy of the collapsed view mounted alongside
    // the advanced view; matching inside it would click a dead row
    const dead = [row('Model', state.model), row('Effort', state.effort)];
    dead.forEach((d) => { d.onclick = null; }); // inert rows do nothing when clicked
    menu.append(new El('div', { inert: '' }).append(...dead));
    const rows = [row('Model', state.model), row('Effort', state.effort)];
    if (shape === 'wrapped') menu.append(new El('div', {}).append(...rows));
    else menu.append(...rows);
  };
  const advanced = new El('div', { role: 'menuitem' }, 'Advanced');
  advanced.onclick = openAdvanced;
  const closeAll = () => { menu.clear(); menu.attrs['data-state'] = 'closed'; doc.children = doc.children.filter((c) => c !== menu); };
  pill.onclick = () => {
    menu.clear();
    menu.attrs['data-state'] = 'open';
    if (shape === 'compact') menu.append(compact());
    else menu.append(advanced);
    if (!doc.children.includes(menu)) doc.append(menu);
  };
  doc.body = doc;
  doc.onkeydown = closeAll; // Escape closes the picker, like Radix does
  return { doc, state, closeAll };
}

async function run(stepId, page, args = []) {
  const body = scriptOf(stepId);
  const fn = new Function('document', 'window', 'location', 'URLSearchParams', 'getComputedStyle', 'Element', 'PointerEvent', 'MouseEvent', 'KeyboardEvent', 'arg0', 'arg1', 'arg2',
    `return (async()=>{${body}})()`);
  class Ev { constructor(type, init = {}) { this.type = type; Object.assign(this, init); } }
  return fn(page.doc, page.win, page.win.location, URLSearchParams, () => ({ visibility: 'visible', display: 'block' }), El, Ev, Ev, Ev, ...args);
}

// ---- checks -----------------------------------------------------------------
const MODELS = ['GPT-5.6 Sol', 'GPT-5.5', 'o3'];
const EFFORTS = ['Instant', 'Medium', 'High', 'Extra High', 'Pro'];
const newPage = (over = {}) => {
  const p = buildPage({ models: MODELS, efforts: EFFORTS, model: 'GPT-5.5', effort: 'Instant', ...over });
  p.win = { location: { href: 'https://chatgpt.com/', search: '' }, URLSearchParams };
  p.doc.title = 't';
  return p;
};
// the stamped option is committed by a CDP click in the real workflow; emulate it here
const commitStamped = (page, id) => page.doc.querySelector(`#${id}`).onclick();

// The live page differs from the mock at exactly this layer, so cover every row
// shape the Advanced panel might plausibly use, not just the one guessed first.
for (const shape of ['menuitem', 'button-aria', 'wrapped']) {
  const page = newPage({ shape });
  await run('s6', page, ['GPT-5.6 Sol', 'High', 'true']);
  commitStamped(page, 'rzn-target-model');
  assert.equal(page.state.model, 'GPT-5.6 Sol', `${shape}: model row found and stamped`);
  await run('s9', page);
  commitStamped(page, 'rzn-target-effort');
  assert.equal(page.state.effort, 'High', `${shape}: effort row found and stamped`);
  const s12 = await run('s12', page, ['none']);
  assert.equal(s12.model_selection.applied, true, `${shape}: verification reads both rows back`);
}

{ // compact Thinking effort picker: Select model row exposes effort directly
  const page = newPage({ shape: 'compact', effort: 'Medium' });
  const s6 = await run('s6', page, ['GPT-5.6 Sol', 'Pro', 'true']);
  assert.equal(s6.picker_mode, 'compact');
  assert.equal(s6.target_text, 'Medium', 'compact row reproduces aria-label Select model with Medium text');
  assert.deepEqual(s6.available_models, ['GPT-5.6 Sol'], 'compact picker does not search effort options for a model');
  commitStamped(page, 'rzn-target-model');
  const s9 = await run('s9', page);
  assert.equal(s9.picker_mode, 'compact');
  assert.deepEqual(s9.available_efforts, EFFORTS);
  commitStamped(page, 'rzn-target-effort');
  assert.equal(page.state.effort, 'Pro');
  const s12 = await run('s12', page, ['none']);
  assert.equal(s12.model_selection.applied, true, 'compact picker verifies fixed model and exact Pro effort');
  assert.equal(s12.model_selection.model_selected, 'GPT-5.6 Sol');
  assert.equal(s12.model_selection.effort_selected, 'Pro');
}

{ // compact picker cannot satisfy a different exact model
  const page = newPage({ shape: 'compact', effort: 'Medium' });
  await assert.rejects(
    () => run('s6', page, ['GPT-5.5', 'Pro', 'true']),
    /compact Thinking effort picker is fixed to GPT-5\.6 Sol/,
  );
}

{ // happy path: model + effort selected, then verified
  const page = newPage();
  const s6 = await run('s6', page, ['GPT-5.6 Sol', 'High', 'true']);
  assert.deepEqual(s6.available_models, MODELS, 's6 lists the models it saw');
  commitStamped(page, 'rzn-target-model');
  assert.equal(page.state.model, 'GPT-5.6 Sol', 's6 stamps the requested model');

  const s9 = await run('s9', page);
  assert.deepEqual(s9.available_efforts, EFFORTS, 's9 lists the effort tiers it saw');
  commitStamped(page, 'rzn-target-effort');
  assert.equal(page.state.effort, 'High', 's9 stamps the requested effort');

  const s12 = await run('s12', page, ['none']);
  assert.equal(s12.model_selection.applied, true, 's12 verifies the applied selection');
  assert.equal(s12.model_observed, 'GPT-5.6 Sol / High');
  assert.equal(s12.upload_input_selector, '#rzn-chatgpt-upload-input');
}

{ // defaults are GPT-5.6 Sol / Medium
  const page = newPage();
  const s6 = await run('s6', page, ['', '', '']);
  assert.equal(s6.desiredModel, 'GPT-5.6 Sol');
  assert.equal(s6.desiredEffort, 'Medium');
}

{ // every supported effort tier selects and verifies through the same picker path
  for (const effort of EFFORTS) {
    const page = newPage();
    await run('s6', page, ['GPT-5.6 Sol', effort, 'true']);
    commitStamped(page, 'rzn-target-model');
    await run('s9', page);
    commitStamped(page, 'rzn-target-effort');
    const s12 = await run('s12', page, ['none']);
    assert.equal(s12.model_selection.applied, true, `${effort}: selection verifies`);
    assert.equal(s12.model_observed, `GPT-5.6 Sol / ${effort}`, `${effort}: observed effort matches`);
  }
}

{ // a plan without Pro fails with the list of what IS available, not a blank throw
  const page = newPage({ efforts: ['Instant', 'Medium', 'High'] });
  await run('s6', page, ['GPT-5.6 Sol', 'Pro', 'true']);
  commitStamped(page, 'rzn-target-model');
  await assert.rejects(() => run('s9', page), /effort_not_found: wanted Pro; available=\["Instant","Medium","High"\]/);
}

{ // unknown model reports the real menu contents
  const page = newPage();
  await assert.rejects(() => run('s6', page, ['GPT-9', 'Pro', 'true']), /model_not_found: wanted GPT-9; available=/);
}

{ // silently-dropped commit is caught by verification
  const page = newPage({ commit: false });
  await run('s6', page, ['GPT-5.6 Sol', 'High', 'true']);
  commitStamped(page, 'rzn-target-model');
  await run('s9', page);
  commitStamped(page, 'rzn-target-effort');
  await assert.rejects(() => run('s12', page, ['none']), /model_selection_verify_failed/);
}

{ // menu labels are free-form: a row that drops the "GPT-" prefix is the same model
  const page = newPage({ models: ['5.6 Sol', '5.5', 'o3'], model: '5.5' });
  await run('s6', page, ['GPT-5.6 Sol', 'High', 'true']);
  commitStamped(page, 'rzn-target-model');
  await run('s9', page);
  commitStamped(page, 'rzn-target-effort');
  const s12 = await run('s12', page, ['none']);
  assert.equal(s12.model_selection.applied, true, 'a GPT-less label must still verify');
  assert.equal(s12.model_selection.model_selected, '5.6 Sol');
}

{ // a genuine mismatch still fails, and says what the row actually read
  const page = newPage({ commit: false });
  await run('s6', page, ['GPT-5.6 Sol', 'High', 'true']);
  commitStamped(page, 'rzn-target-model');
  await run('s9', page);
  commitStamped(page, 'rzn-target-effort');
  await assert.rejects(() => run('s12', page, ['none']), /model_row="ModelGPT-5\.5"/);
}

{ // require_exact_model=false reports the mismatch instead of throwing
  const page = newPage({ commit: false });
  await run('s6', page, ['GPT-5.6 Sol', 'High', 'false']);
  commitStamped(page, 'rzn-target-model');
  await run('s9', page);
  commitStamped(page, 'rzn-target-effort');
  const s12 = await run('s12', page, ['none']);
  assert.equal(s12.model_selection.applied, false);
  assert.equal(s12.model_selection.model_selected, 'GPT-5.5');
}

{ // an unrecognised panel must hand back the real markup, not just a bare throw
  const page = newPage();
  page.doc.querySelector('[role=menu]'); // no menu until the pill is clicked
  const advanced = { onclick: null };
  void advanced;
  // strip the Advanced entry's expansion so no rows ever appear
  const doc = page.doc;
  const origClick = doc.querySelector('button.__composer-pill').onclick;
  doc.querySelector('button.__composer-pill').onclick = (n) => {
    origClick(n);
    const menu = doc.querySelector('[role=menu]');
    menu.clear();
    menu.append(new El('div', { role: 'menuitem', 'data-testid': 'mystery-row' }, 'Something Else'));
  };
  await assert.rejects(
    () => run('s6', page, ['GPT-5.6 Sol', 'Pro', 'true']),
    (e) => /picker_menu_not_found|advanced_rows_not_found/.test(e.message)
      && /"testid":"mystery-row"/.test(e.message)
      && /"text":"Something Else"/.test(e.message),
    'unknown panel markup is dumped into the error for diagnosis',
  );
}

console.log('chatgpt_send picker steps: all checks passed');
