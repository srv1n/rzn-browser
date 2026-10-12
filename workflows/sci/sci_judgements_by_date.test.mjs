// Runnable check for the sci/judgements-by-date reader step (s8).
// The fake DOM follows the result markup the site's ecourt-services-api.js renders:
// rows in #cnrResults, page links in #paginationHtml .pagination a[data-page-id].
// Run: node workflows/sci/sci_judgements_by_date.test.mjs
import assert from 'node:assert';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const wf = JSON.parse(fs.readFileSync(path.join(here, 'sci-judgements-by-date.json'), 'utf8'));
const script = wf.steps.find((s) => s.id === 's8').action.inputs.script;

// ---- minimal DOM: tag, #id, .class, [attr], [attr="v"], descendant and child combinators ----
class El {
  constructor(tag, attrs = {}, kids = []) {
    this.tag = tag; this.attrs = attrs; this.parent = null; this.onclick = null; this.children = [];
    this.setKids(kids);
  }
  setKids(kids) {
    this.children = kids.map((k) => (typeof k === 'string' ? new Text(k) : k));
    this.children.forEach((c) => { c.parent = this; });
  }
  get tagName() { return this.tag.toUpperCase(); }
  get colSpan() { return Number(this.attrs.colspan || 1); }
  get elements() { return this.children.filter((c) => c instanceof El); }
  get textContent() { return this.children.map((c) => c.textContent).join(''); }
  get innerText() { return this.children.map((c) => c.innerText ?? c.textContent).join(' '); }
  get innerHTML() { return this.children.map((c) => c.outerHTML).join(''); }
  get outerHTML() { return `<${this.tag}${Object.entries(this.attrs).map(([k, v]) => ` ${k}="${v}"`).join('')}>${this.innerHTML}</${this.tag}>`; }
  getAttribute(n) { return n in this.attrs ? this.attrs[n] : null; }
  click() { if (this.onclick) this.onclick(); }
  all() { return this.elements.flatMap((e) => [e, ...e.all()]); }
  querySelectorAll(sel) { return this.all().filter((e) => matches(e, sel, this)); }
  querySelector(sel) { return this.querySelectorAll(sel)[0] || null; }
}
class Text { constructor(t) { this.text = t; } get textContent() { return this.text; } get outerHTML() { return this.text; } }

function simple(e, part) {
  const m = part.match(/^([a-z0-9]*)((?:[#.][\w-]+|\[[^\]]+\])*)$/i);
  if (m[1] && e.tag !== m[1].toLowerCase()) return false;
  for (const t of m[2].match(/[#.][\w-]+|\[[^\]]+\]/g) || []) {
    if (t[0] === '#' && e.attrs.id !== t.slice(1)) return false;
    if (t[0] === '.' && !(e.attrs.class || '').split(' ').includes(t.slice(1))) return false;
    if (t[0] === '[') {
      const [, k, v] = t.match(/^\[([\w-]+)(?:="([^"]*)")?\]$/);
      if (!(k in e.attrs) || (v !== undefined && e.attrs[k] !== v)) return false;
    }
  }
  return true;
}
function matches(e, sel, scope) {
  return sel.split(',').some((one) => {
    const parts = one.trim().replace(/\s*>\s*/g, ' > ').split(/\s+/);
    const walk = (node, i) => {
      if (!simple(node, parts[i])) return false;
      if (i === 0) return true;
      const child = parts[i - 1] === '>';
      const j = child ? i - 2 : i - 1;
      for (let p = node.parent; p && p !== scope.parent; p = child ? null : p.parent) {
        if (p instanceof El && walk(p, j)) return true;
      }
      return false;
    };
    return walk(e, parts.length - 1);
  });
}

// ---- fake results page (columns and Judgment-cell links as captured live on 2026-10-12) ----
const h = (tag, attrs, ...kids) => new El(tag, attrs, kids);
const HEADS = ['Serial Number', 'Diary Number', 'Case Number', 'Petitioner / Respondent',
  'Petitioner/Respondent Advocate', 'Bench', 'Judgment By', 'Judgment'];
// links: [[label, href], ...] in page order
const row = (sno, parties, caseNo, diary, links) => h('tr', {},
  ...[sno, diary, caseNo, parties, 'ADV A __ ADV B', "HON'BLE MR. JUSTICE X", "HON'BLE MR. JUSTICE X"].map((t) => h('td', {}, t)),
  h('td', { 'data-th': 'Judgment' }, ...links.map(([label, href]) => h('a', { href }, label))));
const table = (...rows) => h('table', {},
  h('thead', {}, h('tr', {}, ...HEADS.map((t) => h('th', {}, t)))),
  h('tbody', {}, ...rows));
const pager = (current, total) => h('div', { class: 'pagination', 'data-nonce': 'n1' },
  ...Array.from({ length: total }, (_, i) => i + 1).filter((p) => p !== current).map((p) => h('a', { 'data-page-id': String(p) }, String(p))));

function page({ results, pages }) {
  const cnr = h('div', { id: 'cnrResults' }, results(1));
  const pag = h('div', { id: 'paginationHtml' }, ...(pages > 1 ? [pager(1, pages)] : []));
  const root = h('div', {}, cnr, pag);
  const wire = () => pag.querySelectorAll('a').forEach((a) => {
    a.onclick = () => setTimeout(() => {
      const p = Number(a.attrs['data-page-id']);
      cnr.setKids([results(p)]); pag.setKids([pager(p, pages)]); wire();
    }, 50);
  });
  wire();
  return { document: { querySelector: (s) => root.querySelector(s) }, window: {} };
}

async function run(env) {
  const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor;
  return new AsyncFunction('document', 'window', script)(env.document, env.window);
}

// Two result pages, 60 rows in all: every row survives (rows_json is not cut at 50),
// the English PDF wins over regional and INSC links, and names are not cleaned.
const en = (d, n) => `https://api.sci.gov.in/supremecourt/2018/${n}/${n}_2018_Judgement_${d}.pdf`;
const hin = (d, n) => `https://api.sci.gov.in/supremecourt_vernacular/2018/${n}/${n}_2018_4_1501_1_Judgement_${d}_HIN.pdf`;
const insc = (n) => `https://api.sci.gov.in//pdfdate/index1.php?dt=2019-01-02&dno=${n}2018&filename=x.pdf`;
const filler = (i) => row(String(i), `P${i} VS R${i}`, `C.A. No.-${i} - 2019`, `${i}/2018`, [['10-01-2019(English) ', en('10-Jan-2019', i)]]);
let out = await run(page({
  pages: 2,
  results: (p) => p === 1
    ? table(
        row('1', 'STATE OF X  VS  M/S ABC LTD.', 'C.A. No.-001313-001313 - 2019', '1001/2018',
          [[',02-01-2019(हिन्दी) ', hin('02-Jan-2019', 1001)], ['02-01-2019(English) ', en('02-Jan-2019', 1001)],
           ['2019 INSC 1(English) ', insc(1001)], ['', 'https://api.sci.gov.in/']]),
        row('2', 'RAM KUMAR VS UNION OF INDIA', 'W.P.(C) No.-7 - 2018', '1002/2018', [['29-01-2019(অসমীয়া) ', hin('29-Jan-2019', 1002)]]),
        ...Array.from({ length: 48 }, (_, i) => filler(i + 3)))
    : table(...Array.from({ length: 10 }, (_, i) => filler(i + 51)))
}));
assert.equal(out.status, 'ok');
assert.equal(out.complete, true);
assert.equal(out.page_count, 2);
assert.equal(out.pages_html.length, 2);
const rows = JSON.parse(out.rows_json);
assert.equal(out.row_count, 60);
assert.equal(rows.length, 60);
assert.deepEqual(
  { pdf_url: rows[0].pdf_url, petitioner: rows[0].petitioner, respondent: rows[0].respondent, case_number: rows[0].case_number,
    diary_number: rows[0].diary_number, decision_date: rows[0].decision_date, page: rows[0].page },
  { pdf_url: en('02-Jan-2019', 1001), petitioner: 'STATE OF X', respondent: 'M/S ABC LTD.', case_number: 'C.A. No.-001313-001313 - 2019',
    diary_number: '1001/2018', decision_date: '2019-01-02', page: 1 });
assert.equal(rows[1].pdf_url, hin('29-Jan-2019', 1002), 'a regional-only row keeps its regional PDF');
assert.equal(rows[1].decision_date, '2019-01-29');
assert.equal(rows[59].page, 2);
assert.match(rows[0].listed_text, /STATE OF X VS M\/S ABC LTD\./);

// Wrong CAPTCHA, no records, and an unknown message map to distinct statuses.
const notfound = (msg) => page({ pages: 1, results: () => h('div', { class: 'notfound' }, msg) });
assert.equal((await run(notfound('Invalid Captcha'))).status, 'captcha_wrong');
assert.equal((await run(notfound('No Record Found'))).status, 'empty');
const blocked = await run(notfound('Too many requests'));
assert.equal(blocked.status, 'error');
assert.equal(blocked.message, 'Too many requests');

console.log('sci_judgements_by_date: ok');
