import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const workflow = JSON.parse(readFileSync(new URL('../../workflows/linkedin/linkedin_company.json', import.meta.url), 'utf8'));
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
function harness(document: any, frames: any[][] = []) {
  const store = new Map([['__rzn_co_slug', '"stripe"']]);
  const sessionStorage = { getItem: (k: string) => store.get(k) || null, setItem: (k: string, v: string) => store.set(k, v), removeItem: (k: string) => store.delete(k) };
  const location = { origin: 'https://www.linkedin.com', href: 'https://www.linkedin.com/company/stripe/posts/', assign: (url: string) => { location.href = url; } };
  let frame = 0;
  const window = { innerHeight: 1000, scrollBy: () => { frame++; } };
  if (frames.length) document.querySelectorAll = () => frames[Math.min(frame, frames.length - 1)];
  const run = (id: string, cap = 10) => new AsyncFunction('document', 'sessionStorage', 'location', 'window', 'setTimeout', 'Node', 'arg0', workflow.steps.find((s: any) => s.id === id).action.inputs.script)(document, sessionStorage, location, window, (f: () => void) => f(), { DOCUMENT_POSITION_FOLLOWING: 4 }, String(cap));
  return { run, location, store };
}
function card(id: string, text: string, reactions: string, comments: string, reposts: string, legacy = false) {
  const menu = { getAttribute: () => 'Open control menu for post by Stripe' };
  const link = { href: `https://www.linkedin.com/posts/stripe_activity-${id}-abc?tracking=1` };
  const labels = [reactions + ' reactions', comments + ' comments', reposts + ' reposts'].map(s => ({ getAttribute: () => s }));
  const box = { innerText: text, contains: () => false, compareDocumentPosition: () => 4 };
  const el: any = {
    innerText: `Stripe 2w ${text} ${reactions} reactions ${comments} comments ${reposts} reposts`,
    getAttribute: (k: string) => legacy && k === 'data-urn' ? `urn:li:activity:${id}` : null,
    querySelector: (s: string) => s.includes('expandable-text-box') ? box : s.startsWith('button[aria-label^=') ? menu : s.includes('Comment') ? {} : null,
    querySelectorAll: (s: string) => s === 'a[href]' ? [link] : s === 'button,span' ? [] : labels,
  };
  return el;
}

describe('LinkedIn company workflow', () => {
  it('opens People when its link is missing and accepts query-bearing tab links', async () => {
    const h = harness({ querySelectorAll: () => [] });
    await h.run('s14');
    expect(h.location.href).toBe('https://www.linkedin.com/company/stripe/people/');
    let clicked = false;
    const a = { href: 'https://www.linkedin.com/company/stripe/people/?view=all', click: () => { clicked = true; } };
    const withLink = harness({ querySelectorAll: () => [{ href: 'https://www.linkedin.com/company/other/people/', click: () => { throw new Error('wrong company'); } }, a] });
    await withLink.run('s14');
    expect(clicked).toBe(true);
  });

  it('retains virtualized posts and scopes every metric and body to its own card', async () => {
    const a = card('123', 'First post', '1.2K', '8', '2');
    const b = card('456', 'Second post', '31', '0', '4', true);
    const duplicate = card('123', 'First post', '1.2K', '8', '2');
    const original = duplicate.querySelectorAll;
    duplicate.querySelectorAll = (s: string) => s === 'a[href]' ? [] : original(s);
    const h = harness({}, [[a, duplicate], [b]]);
    const result = await h.run('s22', 2);
    expect(result.posts_fetched).toBe(2);
    expect(result.posts_last_30_days).toBe(2);
    expect(result.posts).toMatchObject([
      { text: 'First post', reactions: 1200, comments: 8, reposts: 2, urn: 'urn:li:activity:123', age_days: 14 },
      { text: 'Second post', reactions: 31, comments: 0, reposts: 4, urn: 'urn:li:activity:456' },
    ]);
    expect(result.posts[0].post_url).not.toContain('?');
    expect(JSON.parse(h.store.get('__rzn_co_posts')!).posts).toEqual(result.posts);
  });

  it('reads a bare modern reaction total without inventing comments or reposts', async () => {
    const el = card('789', 'Only reactions', '0', '0', '0');
    el.innerText = 'Stripe Only reactions 27 Like Comment Repost';
    const original = el.querySelectorAll;
    el.querySelectorAll = (s: string) => s === '[aria-label]' ? [] : s === 'button,span' ? [{ innerText: '27', getAttribute: () => '' }] : original(s);
    const result = await harness({}, [[el]]).run('s22', 1);
    expect(result.posts[0]).toMatchObject({ reactions: 27, comments: 0, reposts: 0 });
  });

  it('bounds scrolling on empty pages and respects max_posts', async () => {
    expect((await harness({}, [[]]).run('s22')).posts_fetched).toBe(0);
    const cards = [card('1', 'One', '1', '0', '0'), card('2', 'Two', '2', '0', '0')];
    expect((await harness({}, [cards]).run('s22', 1)).posts).toHaveLength(1);
  });

  it('compiles every company script', () => {
    for (const step of workflow.steps) {
      if (step.action.inputs.script) expect(() => new AsyncFunction('arg0', step.action.inputs.script)).not.toThrow();
    }
  });
});
