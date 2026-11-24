// Hand-built Reddit samples: the workflows' own mapping step run against invented old.reddit.com
// pages (thread, feed and search rows). Not a recording: replace with real runs (`source: "recorded"`).
//
//   node scripts/workflow_samples/reddit_items_sample.mjs            # check samples are current
//   node scripts/workflow_samples/reddit_items_sample.mjs --write    # rewrite them
import { finish, sample } from './sample_lib.mjs';

// The page-reading path makes no request: any fetch here fails the build.
globalThis.fetch = () => { throw new Error('a read workflow must not fetch'); };

// A fake of the old.reddit.com page: elements answer the selectors the mapping steps ask.
const node = ({ attrs = {}, cls = [], text = '', q = {}, all = {} } = {}) => ({ textContent: text, innerText: text,
  getAttribute: (n) => attrs[n] ?? null, classList: { contains: (c) => cls.includes(c) },
  querySelector: (sel) => q[sel] ?? null, querySelectorAll: (sel) => all[sel] ?? [] });
const postAttrs = (id, f) => ({ 'data-fullname': 't3_' + id, 'data-author': f.author, 'data-subreddit': 'rust', 'data-timestamp': String(f.at * 1000),
  'data-permalink': `/r/rust/comments/${id}/${f.slug}/`, 'data-url': f.url || `/r/rust/comments/${id}/${f.slug}/`, 'data-domain': f.domain || 'self.rust',
  'data-comments-count': String(f.comments), 'data-score': String(f.score), 'data-nsfw': 'false', 'data-promoted': f.promoted ? 'true' : 'false' });
const postRow = (id, f) => node({ attrs: postAttrs(id, f), cls: ['thing', 'link'], q: {
  'a.title': node({ text: f.title }), '.linkflairlabel': f.flair ? node({ attrs: { title: f.flair } }) : null,
  ':scope > .entry .tagline': node({ q: { 'time:not(.edited-timestamp)': node({ attrs: { datetime: new Date(f.at * 1000).toISOString().replace('.000Z', '+00:00') } }),
    'time.edited-timestamp': f.edited ? node({ attrs: { datetime: new Date(f.edited * 1000).toISOString() } }) : null } }),
  ':scope > .entry .expando .usertext-body .md': f.body ? node({ text: f.body }) : null } });
const askPost = { author: 'ferris', title: 'How do you pace async Rust services?', slug: 'how_do_you_pace_async_rust_services', flair: 'question',
  body: 'We run tokio with one runtime per service. What do you do for backpressure?', at: 1735689600, score: 212, comments: 4, edited: 1735693000 };
const crate = { author: 'maintainer', title: 'Announcing a crate', slug: 'announcing_a_crate', url: 'https://github.com/example/crate', domain: 'github.com', at: 1735772400, score: 98, comments: 14 };
const ad = { author: 'brand', title: 'Try our product', slug: 'ad', at: 1735772000, score: 1, comments: 0, promoted: true };
const comment = (id, author, body, at, score, replies = [], { op = false, deleted = false } = {}) => node({
  attrs: { 'data-fullname': 't1_' + id, 'data-author': author, 'data-permalink': `/r/rust/comments/abc123/x/${id}/` }, cls: ['thing', 'comment', ...(deleted ? ['deleted'] : [])],
  q: { ':scope > .entry': node({ q: {
         '.tagline': node({ q: { 'a.author': deleted ? null : node({ cls: op ? ['author', 'submitter'] : ['author'] }), '.score.unvoted': deleted ? null : node({ attrs: { title: String(score) } }),
           'time:not(.edited-timestamp)': node({ attrs: { datetime: new Date(at * 1000).toISOString().replace('.000Z', '+00:00') } }) } }),
         '.usertext-body .md': node({ text: body }) } }),
       ':scope > .child > .sitetable': replies.length ? node({ all: { ':scope > .thing.comment': replies } }) : null } });
const comments = node({ all: { ':scope > .thing.comment': [
  comment('c1', 'crab', 'Bounded channels everywhere, and a semaphore at the edge.', 1735693200, 40, [
    comment('c2', 'ferris', 'Thanks, the semaphore is what we were missing.', 1735696800, 12, [], { op: true }),
    comment('c3', '[deleted]', '[deleted]', 1735697000, 1, [comment('c4', 'tokio_fan', 'Same here.', 1735700400, 3)], { deleted: true }),
  ]),
] } });
const page = (q, all) => { globalThis.document = node({ q, all }); };

globalThis.location = { pathname: '/r/rust/comments/abc123/how_do_you_pace_async_rust_services/' };
page({ '#siteTable > .thing.link': postRow('abc123', askPost), '.side .linkinfo .score': node({ text: '212 points (97% upvoted)' }), '.commentarea > .sitetable': comments },
  { '.commentarea .thing.morechildren, .commentarea .deepthread': [] });
await sample('workflows/reddit/reddit-thread.json', ['https://www.reddit.com/r/rust/comments/abc123/'], 'reddit_items_sample.mjs');

globalThis.location = { pathname: '/r/rust/top/' };
page({ '.nav-buttons .next-button a': node({ attrs: { href: 'https://old.reddit.com/r/rust/top/?t=all&limit=2&count=2&after=t3_def456' } }) },
  { '#siteTable > .thing.link': [postRow('abc123', askPost), postRow('zzz999', ad), postRow('def456', crate)] });
await sample('workflows/reddit/reddit-listing.json', ['rust', '2'], 'reddit_items_sample.mjs');

const el = (text, attrs = {}) => ({ textContent: text, getAttribute: (n) => attrs[n] ?? null });
const row = (id, title, sub, author, score, comments, at) => ({ querySelector: (sel) => ({
  'a.search-title, a.title': el(title, { href: `https://old.reddit.com/r/${sub}/comments/${id}/slug/` }),
  'a.search-comments, a.comments': el(`${comments} comments`, { href: `https://old.reddit.com/r/${sub}/comments/${id}/slug/` }),
  'a.search-subreddit-link, a.subreddit': el(`r/${sub}`), 'a.author': el(author), '.search-score, .score': el(`${score} points`),
  time: el('', { datetime: at }) }[sel] || null) });
globalThis.location = { origin: 'https://old.reddit.com' };
globalThis.document = { querySelectorAll: () => [
  row('abc123', 'How do you pace async Rust services?', 'rust', 'ferris', '1,212', 56, '2025-01-01T00:00:00+00:00'),
  row('ghi789', 'Tokio vs async-std in 2025', 'learnrust', 'newcomer', 7, 3, '2025-01-03T09:30:00+00:00'),
] };
await sample('workflows/reddit/reddit-search.json', ['25'], 'reddit_items_sample.mjs');
finish();
