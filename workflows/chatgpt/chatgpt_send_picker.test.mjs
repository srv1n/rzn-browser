// Runnable check for the chatgpt/send model-picker steps (s6/s9/s12).
// The fake DOM reproduces the composer picker captured live on 2026-08-31:
// one flat panel holding a model radio per model and a five-stop effort slider.
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
  set id(v) { this.attrs.id = v; }
  get tagName() { return this.tag.toUpperCase(); }
  contains(other) { let n = other; while (n) { if (n === this) return true; n = n.parent; } return false; }
  // real textContent concatenates child text with no separator
  get textContent() { return this.children.length ? this.children.map((c) => c.textContent).join('') : this.text; }
  getAttribute(n) { return n in this.attrs ? String(this.attrs[n]) : null; }
  setAttribute(n, v) { this.attrs[n] = v; }
  append(...kids) { for (const k of kids) { k.parent = this; this.children.push(k); } return this; }
  clear() { this.children = []; }
  scrollIntoView() {}
  focus() {}
  getBoundingClientRect() { return { left: 0, top: 0, width: 100, height: 20 }; }
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
// Ground truth, captured 2026-08-31 from the live composer:
//   button[aria-haspopup=menu]                              pill, text = current tier
//   [data-testid=composer-intelligence-picker-content]      the panel
//     div[role=menuitem][aria-label="Select model"]         text is the TIER, not a model
//     [data-testid=composer-model-picker-slider-simple-view]
//         "Pro, 5 of 5.Use Left and Right arrow keys to adjust power."
//       div[role=menuitem][aria-label=Power] tabindex=0     arrow keys land here
//         span[role=slider] aria-valuenow/min/max           0..4, NO aria-valuetext
//     [data-testid=composer-model-picker-slider-advanced-view]
//       div[role=menuitemradio] aria-checked  per model
//
// commitModel=false simulates the page dropping a model click on the floor.
// sliderDead=true simulates a slider that ignores arrow keys.
function buildPage({ models, efforts, model, effort, commitModel = true, sliderDead = false, modelsHidden = false }) {
  const state = { model, effort };
  const doc = new El('body');
  const form = new El('form');
  const pill = new El('button', { class: '__composer-pill', 'aria-haspopup': 'menu' }, state.effort);
  form.append(pill, new El('input', { id: 'upload-files' }));
  doc.append(form);

  const menu = new El('div', { role: 'menu' });
  const closeAll = () => {
    menu.clear();
    menu.attrs['data-state'] = 'closed';
    doc.children = doc.children.filter((c) => c !== menu);
  };

  const buildPanel = () => {
    const panel = new El('div', { 'data-testid': 'composer-intelligence-picker-content' });
    const header = new El('div', { role: 'menuitem', 'aria-label': 'Select model' }, state.effort);

    const simple = new El('div', { 'data-testid': 'composer-model-picker-slider-simple-view' });
    const index = () => efforts.indexOf(state.effort);
    const caption = new El('span', {}, '');
    const power = new El('div', { role: 'menuitem', 'aria-label': 'Power', tabindex: '0' });
    const slider = new El('span', {
      role: 'slider',
      'aria-valuemin': '0',
      'aria-valuemax': String(efforts.length - 1),
      'aria-valuenow': String(index()),
    });
    // the live page prints the tier only in this caption, never as aria-valuetext
    const paint = () => {
      const i = Number(slider.getAttribute('aria-valuenow'));
      caption.text = `${efforts[i]}, ${i + 1} of ${efforts.length}.Use Left and Right arrow keys to adjust power.`;
      state.effort = efforts[i];
      header.text = state.effort;
    };
    power.append(slider);
    simple.append(caption, power);
    power.onkeydown = (ev) => {
      if (sliderDead) return;
      const cur = Number(slider.getAttribute('aria-valuenow'));
      const step = ev.key === 'ArrowRight' ? 1 : ev.key === 'ArrowLeft' ? -1 : 0;
      slider.setAttribute('aria-valuenow', String(Math.max(0, Math.min(efforts.length - 1, cur + step))));
      paint();
    };
    // a real click on the row snaps the thumb to the click point; the workflow must never do this
    power.onclick = () => {
      if (sliderDead) return;
      slider.setAttribute('aria-valuenow', String(Math.floor((efforts.length - 1) / 2)));
      paint();
    };
    paint();

    const advanced = new El('div', { 'data-testid': 'composer-model-picker-slider-advanced-view' });
    const paintModels = () => {
      advanced.clear();
      advanced.append(...models.map((o) => {
        const el = new El('div', { role: 'menuitemradio', 'aria-checked': String(o === state.model) }, o);
        el.onclick = () => { if (commitModel) state.model = o; paintModels(); };
        return el;
      }));
    };
    paintModels();

    // some renders collapse the model list until the header is clicked
    if (modelsHidden) {
      advanced.clear();
      header.onclick = () => paintModels();
    } else {
      header.onclick = () => {};
    }

    panel.append(header, simple, advanced);
    return panel;
  };

  pill.onclick = () => {
    menu.clear();
    menu.attrs['data-state'] = 'open';
    menu.append(buildPanel());
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
const MODELS = ['GPT-5.6 Sol', 'GPT-5.5'];
const EFFORTS = ['Instant', 'Medium', 'High', 'Extra High', 'Pro'];
const newPage = (over = {}) => {
  const p = buildPage({ models: MODELS, efforts: EFFORTS, model: 'GPT-5.5', effort: 'Instant', ...over });
  p.win = { location: { href: 'https://chatgpt.com/', search: '' }, URLSearchParams };
  p.doc.title = 't';
  return p;
};

{ // no click_element step may touch the picker; both bugs this file exists to prevent
  const clicks = wf.steps.filter((s) => s.action.kind === 'click_element').map((s) => s.action.target.selector);
  assert.ok(!clicks.includes('#rzn-target-effort'),
    'a click_element step targets the slider row again: a trusted click snaps the slider and discards the tier');
  assert.ok(!clicks.includes('#rzn-target-model'),
    'a click_element step targets the model radio again: the trusted CDP click does not register on the live popper');
  assert.ok(!scriptOf('s9').includes("id='rzn-target-effort'"),
    's9 must commit the effort with arrow keys, not stamp a target for a click');
  assert.ok(!scriptOf('s6').includes("id='rzn-target-model'"),
    's6 must commit the model radio itself, not stamp a target for a click');
}

{ // happy path: both model and effort apply, then verify
  const page = newPage();
  const s6 = await run('s6', page, ['GPT-5.6 Sol', 'High', 'true']);
  assert.deepEqual(s6.available_models, MODELS, 's6 lists the models it saw');
  assert.equal(s6.picker_mode, 'slider');
  assert.equal(page.state.model, 'GPT-5.6 Sol', 's6 commits the requested model radio itself');

  const s9 = await run('s9', page);
  assert.deepEqual(s9.available_efforts, EFFORTS, 's9 harvests the tier ladder off the page');
  assert.equal(s9.effort_committed, true);
  assert.equal(page.state.effort, 'High', 's9 commits the effort with arrow keys alone');

  const s12 = await run('s12', page, ['none']);
  assert.equal(s12.model_selection.applied, true);
  assert.equal(s12.model_observed, 'GPT-5.6 Sol / High');
  assert.equal(s12.upload_input_selector, '#rzn-chatgpt-upload-input');
}

{ // the second model is selectable; the picker is not pinned to GPT-5.6 Sol
  const page = newPage({ model: 'GPT-5.6 Sol' });
  const s6 = await run('s6', page, ['GPT-5.5', 'Medium', 'true']);
  assert.equal(s6.target_text, 'GPT-5.5');
  assert.equal(page.state.model, 'GPT-5.5');
  await run('s9', page);
  const s12 = await run('s12', page, ['none']);
  assert.equal(s12.model_selection.model_selected, 'GPT-5.5');
  assert.equal(s12.model_selection.applied, true);
}

{ // every tier selects and verifies, in both directions of travel
  for (const effort of EFFORTS) {
    for (const start of ['Instant', 'Pro']) {
      const page = newPage({ effort: start });
      await run('s6', page, ['GPT-5.6 Sol', effort, 'true']);
      await run('s9', page);
      const s12 = await run('s12', page, ['none']);
      assert.equal(s12.model_observed, `GPT-5.6 Sol / ${effort}`, `${start} -> ${effort}`);
    }
  }
}

{ // a model list that only renders after the header is clicked still resolves
  const page = newPage({ modelsHidden: true });
  const s6 = await run('s6', page, ['GPT-5.6 Sol', 'High', 'true']);
  assert.deepEqual(s6.available_models, MODELS, 's6 clicks Select model to reveal the radios');
}

{ // defaults are GPT-5.6 Sol / Medium
  const page = newPage();
  const s6 = await run('s6', page, ['', '', '']);
  assert.equal(s6.desiredModel, 'GPT-5.6 Sol');
  assert.equal(s6.desiredEffort, 'Medium');
}

{ // a plan without Pro fails with the list of what IS available, not a blank throw
  const page = newPage({ efforts: ['Instant', 'Medium', 'High'] });
  await run('s6', page, ['GPT-5.6 Sol', 'Pro', 'true']);
  await assert.rejects(() => run('s9', page), /effort_not_found: wanted Pro; available=\["Instant","Medium","High"\]/);
}

{ // a slider that ignores arrow keys is reported as such, not silently accepted
  const page = newPage({ sliderDead: true, effort: 'High' });
  await run('s6', page, ['GPT-5.6 Sol', 'Pro', 'true']);
  await assert.rejects(() => run('s9', page), /effort_slider_unresponsive/);
}

{ // unknown model reports the real menu contents
  const page = newPage();
  await assert.rejects(() => run('s6', page, ['GPT-9', 'Pro', 'true']), /model_not_found: wanted GPT-9; available=/);
}

{ // a silently-dropped model commit fails inside s6, naming the panel contents
  const page = newPage({ commitModel: false });
  await assert.rejects(
    () => run('s6', page, ['GPT-5.6 Sol', 'High', 'true']),
    (e) => /model_commit_failed: clicked GPT-5\.6 Sol/.test(e.message) && /"text":"GPT-5\.5"/.test(e.message),
    'a click the page ignores must fail loudly at the step that made it',
  );
}

{ // verification still catches a model that drifts after s6 committed it
  const page = newPage();
  await run('s6', page, ['GPT-5.6 Sol', 'High', 'true']);
  await run('s9', page);
  page.state.model = 'GPT-5.5'; // the page silently reverts behind our back
  await assert.rejects(() => run('s12', page, ['none']),
    /model_selection_verify_failed.*model_row="GPT-5\.6 Sol \| GPT-5\.5"/s);
}

{ // menu labels are free-form: a row that drops the "GPT-" prefix is the same model
  const page = newPage({ models: ['5.6 Sol', '5.5'], model: '5.5' });
  await run('s6', page, ['GPT-5.6 Sol', 'High', 'true']);
  await run('s9', page);
  const s12 = await run('s12', page, ['none']);
  assert.equal(s12.model_selection.applied, true, 'a GPT-less label must still verify');
  assert.equal(s12.model_selection.model_selected, '5.6 Sol');
}

{ // require_exact_model=false reports a drifted model instead of throwing
  const page = newPage();
  await run('s6', page, ['GPT-5.6 Sol', 'High', 'false']);
  await run('s9', page);
  page.state.model = 'GPT-5.5';
  const s12 = await run('s12', page, ['none']);
  assert.equal(s12.model_selection.applied, false);
  assert.equal(s12.model_selection.model_selected, 'GPT-5.5');
}

{ // an unrecognised panel must hand back the real markup, not just a bare throw
  const page = newPage();
  const doc = page.doc;
  const pill = doc.querySelector('button.__composer-pill');
  pill.onclick = () => {
    const menu = new El('div', { role: 'menu', 'data-state': 'open' });
    menu.append(new El('div', { role: 'menuitem', 'data-testid': 'mystery-row' }, 'Something Else'));
    doc.append(menu);
  };
  await assert.rejects(
    () => run('s6', page, ['GPT-5.6 Sol', 'Pro', 'true']),
    (e) => /picker_menu_not_found/.test(e.message)
      && /"testid":"mystery-row"/.test(e.message)
      && /"text":"Something Else"/.test(e.message),
    'unknown panel markup is dumped into the error for diagnosis',
  );
}

console.log('chatgpt_send picker steps: all checks passed');
