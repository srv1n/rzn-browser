import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const manifest = (name: string) => JSON.parse(readFileSync(new URL(`../../workflows/book/book-${name}.json`, import.meta.url), 'utf8'));
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const md5 = 'dd6c5f9e304f5f6002ec9f0313b87d0b';
const origin = 'https://source.example';
const cell = (textContent: string, links: any[] = []) => ({ textContent, querySelector: () => links[0] || null, querySelectorAll: () => links });
const row = (title: string, year: string) => ({ children: [cell(title), cell('Author'), cell('Publisher'), cell(year), cell('English'), cell('100'), cell('1MB'), cell('epub'), cell('', [{ getAttribute: () => `/ads.php?md5=${md5}` }])] });
function harness(name: string, params: any = {}, rows: any[] | null = null) {
  const storage = new Map<string, string>();
  const sessionStorage = { getItem: (key: string) => storage.get(key) || null, setItem: (key: string, value: string) => storage.set(key, value), removeItem: (key: string) => storage.delete(key) };
  const location = { origin, href: `${origin}/search?q=book`, pathname: '/search', assign: (url: string) => { location.href = url; location.pathname = new URL(url).pathname; } };
  const document: any = {
    title: 'Search',
    querySelector: (selector: string) => selector === '#tablelibgen' && rows !== null ? { querySelectorAll: () => rows } : null,
    querySelectorAll: () => [],
  };
  const run = (id: string) => {
    const step = manifest(name).steps.find((step: any) => step.id === id);
    // The runner also expands placeholders inside scripts. Shorthand {sort}
    // collides with the sort parameter, even though it is valid JavaScript.
    const script = Object.entries(params).reduce((script, [key, value]) => script.replaceAll(`{${key}}`, String(value)), step.action.inputs.script);
    return new AsyncFunction('window', 'document', 'location', 'sessionStorage', script)(
      { __rzn_params: params }, document, location, sessionStorage,
    );
  };
  return { run, location, document, sessionStorage };
}

describe('book workflow scripts', () => {
  it('sets native Anna sorting after workflow parameter expansion', async () => {
    const h = harness('search', { sort: 'year_desc' });
    const control = { value: '' };
    h.document.querySelector = () => ({ name: 'q', form: { querySelector: () => control } });
    expect(await h.run('sort')).toEqual({ sort: 'year_desc' });
    expect(control.value).toBe('newest');
  });
  it('sorts every loaded row before limiting and places unknown years last in both directions', async () => {
    const rows = [row('Unknown', ''), row('Old', '1991'), row('New', '2024')];
    const newest = await harness('search', { sort: 'year_desc', limit: 1 }, rows).run('s8');
    expect(newest.results.map((r: any) => r.title)).toEqual(['New']);
    const oldest = await harness('search', { sort: 'year_asc' }, rows).run('s8');
    expect(oldest.results.map((r: any) => r.title)).toEqual(['Old', 'New', 'Unknown']);
  });

  it('parses Anna title cards without collecting header ticker links', async () => {
    const h = harness('search');
    const author = { textContent: 'David Mamet' }, publisher = { textContent: 'Viking Adult, 1991' };
    const meta = { classList: { contains: () => true }, childNodes: [{ textContent: 'English [en] · EPUB · 0.2MB · 1991 · Book' }] };
    const a = { textContent: 'On Directing film', getAttribute: () => `/md5/${md5}`, parentElement: { parentElement: { children: [meta] }, querySelectorAll: () => [author, publisher] } };
    h.document.querySelectorAll = (selector: string) => {
      expect(selector).toBe('main a.js-vim-focus[href^="/md5/"]');
      return [a];
    };
    const output = await h.run('s8');
    expect(output.results[0]).toMatchObject({ title: 'On Directing film', year: '1991', ext: 'epub', language: 'English', md5 });
    // Some records omit the language; positional parsing called the size a format.
    meta.childNodes[0].textContent = 'EPUB · 0.2MB · 1991 · Book';
    const withoutLanguage = await h.run('s8');
    expect(withoutLanguage.results[0]).toMatchObject({ ext: 'epub', size: '0.2MB', language: '' });
  });

  it('accepts the observed empty Anna result container without timing out', async () => {
    const h = harness('search');
    const main = { textContent: 'Download 0 No files found. Try fewer or different search terms and filters.' };
    h.document.querySelector = (selector: string) => selector === 'main' ? main : null;
    for (const name of ['search', 'get']) {
      const wait = manifest(name).steps.find((step: any) => step.id === 's6');
      expect(wait.action.target.selector).toContain('main .js-aarecord-list-outer');
    }
    expect(await h.run('s8')).toMatchObject({ layout_ok: true, count: 0, results: [] });
  });

  it('reports the observed manual DDoS-Guard check instead of an empty result or timeout', async () => {
    for (const name of ['search', 'get']) {
      const h = harness(name);
      h.document.querySelector = (selector: string) => selector === '#ddg-captcha' ? {} : null;
      await expect(h.run('check_verification')).rejects.toThrow('site_challenge');
      await expect(h.run('s8')).rejects.toThrow('site_challenge');
      if (name === 'get') {
        for (const step of ['slow', 'download_probe_0', 'download_probe_1', 'download_probe_2', 'resolve']) {
          await expect(h.run(step)).rejects.toThrow('site_challenge');
        }
      }
    }
  });

  it('distinguishes an empty LibGen counter from an unsupported page', async () => {
    for (const name of ['search', 'get']) {
      const h = harness(name);
      let counter = 'Files 0';
      h.document.querySelector = (selector: string) => selector === '.nav-tabs .nav-link.active' ? {textContent: counter} : null;
      expect(await h.run('s8')).toMatchObject(name === 'search' ? {layout_ok: true, count: 0} : {found: false, status: 'no_format_match'});
      counter = 'Editions 0';
      expect(await h.run('s8')).toMatchObject(name === 'search' ? {layout_ok: false} : {found: false, status: 'no_table'});
    }
  });

  it('retains an exact selection when result rows change order', async () => {
    const other = 'b'.repeat(32);
    const second = row('Selected', '2020');
    second.children[8] = cell('', [{getAttribute: () => `/ads.php?md5=${other}`}]);
    const rows = [row('First', '2021'), second];
    for (let i = 0; i < 2; i++) {
      const h = harness('get', {format: 'epub', selected_md5: other}, rows);
      await h.run('s8');
      expect(JSON.parse(h.sessionStorage.getItem('__rzn_book_selection')!).chosen.md5).toBe(other);
      rows.reverse();
    }
  });

  it('never substitutes another file when an exact selector disappears', async () => {
    const h = harness('get', { format: 'epub', selected_md5: 'a'.repeat(32) }, [row('Book', '2020')]);
    expect(await h.run('s8')).toMatchObject({ found: false, status: 'no_format_match' });
    expect(h.location.pathname).toBe('/search');
    expect(await h.run('resolve')).toMatchObject({ found: false, status: 'no_format_match' });
  });

  it('navigates to the file page and resolves a direct GET link without fetching HTML', async () => {
    const h = harness('get', { format: 'epub' }, [row('Book', '2020')]);
    await h.run('s8');
    expect(h.location.href).toBe(`${origin}/ads.php?md5=${md5}`);
    const link = { textContent: 'GET', href: 'https://files.example/book.epub', getAttribute: () => '/file/book.epub' };
    h.document.querySelectorAll = () => [link];
    const output = await h.run('resolve');
    expect(output).toMatchObject({ found: true, download_url: link.href, attachment_urls: [{ url: link.href, filename: 'Book.epub' }] });
  });

  it('uses the requested slow partner and never a fast-download link', async () => {
    const h = harness('get', { mirror_index: 1 });
    h.sessionStorage.setItem('__rzn_book_selection', JSON.stringify({ status: 'resolving', chosen: { md5 } }));
    const links = [0, 1].map(index => ({ href: `${origin}/slow_download/${md5}/${index}/0`, textContent: `Slow Partner Server #${index + 1}` }));
    h.document.querySelectorAll = (selector: string) => selector.includes('slow_download') ? links : [];
    expect(await h.run('slow')).toMatchObject({ status: 'waiting_for_download' });
    expect(h.location.pathname).toBe(`/slow_download/${md5}/1/0`);
  });

  it('prefers a slow partner without a waitlist', async () => {
    const h = harness('get');
    h.sessionStorage.setItem('__rzn_book_selection', JSON.stringify({ status: 'resolving', chosen: { md5 } }));
    const links = [0, 1].map(index => ({ href: `${origin}/slow_download/${md5}/${index}/0`, textContent: `Slow Partner Server #${index + 1}`, parentElement: { textContent: index ? 'no waitlist, but can be very slow' : 'slightly faster but with waitlist' } }));
    h.document.querySelectorAll = (selector: string) => selector.includes('slow_download') ? links : [];
    await h.run('slow');
    expect(h.location.pathname).toBe(`/slow_download/${md5}/1/0`);
  });

  it('resolves only Download now on the selected slow page', async () => {
    const h = harness('get');
    h.location.pathname = `/slow_download/${md5}/0/0`;
    h.sessionStorage.setItem('__rzn_book_selection', JSON.stringify({ status: 'resolving', chosen: { md5, title: 'Film', ext: 'epub' } }));
    h.document.querySelectorAll = () => [{ textContent: '📚 Download now', href: 'https://files.example/film.epub' }];
    expect(await h.run('resolve')).toMatchObject({ found: true, download_url: 'https://files.example/film.epub' });
  });

  it('reports a persistent verification page instead of returning an HTML attachment', async () => {
    const h = harness('get');
    h.sessionStorage.setItem('__rzn_book_selection', JSON.stringify({ status: 'resolving', chosen: { md5 } }));
    h.document.title = 'DDoS-Guard';
    await expect(h.run('resolve')).rejects.toThrow('site_challenge');
  });

  it('waits across runner probes without returning a slow landing page as the file', async () => {
    const h = harness('get');
    h.location.pathname = `/slow_download/${md5}/0/0`;
    h.sessionStorage.setItem('__rzn_book_selection', JSON.stringify({ status: 'resolving', chosen: { md5, title: 'Film', ext: 'epub' } }));
    h.document.documentElement = {};
    expect(await h.run('download_probe_0')).toMatchObject({found: false, status: 'waiting_for_download'});
    h.document.querySelectorAll = () => [{ textContent: '📚 Download now', href: 'https://files.example/ready.epub' }];
    expect(await h.run('resolve')).toMatchObject({ found: true, download_url: 'https://files.example/ready.epub' });
  });
});
