// Hand-built Reddit samples: the workflows' own mapping step run against invented .json listings
// and old.reddit search rows. Not a recording: replace with real runs (`source: "recorded"`).
//
//   node scripts/workflow_samples/reddit_items_sample.mjs            # check samples are current
//   node scripts/workflow_samples/reddit_items_sample.mjs --write    # rewrite them
import { finish, sample } from './sample_lib.mjs';

const post = {
  id: 'abc123', title: 'How do you pace async Rust services?', subreddit: 'rust', author: 'ferris',
  selftext: 'We run tokio with one runtime per service. What do you do for backpressure?',
  permalink: '/r/rust/comments/abc123/how_do_you_pace_async_rust_services/', url: 'https://www.reddit.com/r/rust/comments/abc123/how_do_you_pace_async_rust_services/',
  domain: 'self.rust', link_flair_text: 'question', is_self: true, over_18: false, edited: false,
  score: 212, upvote_ratio: 0.97, num_comments: 3, created_utc: 1735689600,
};
const comment = (id, author, body, created, score, replies = [], extra = {}) => ({ kind: 't1', data: { id, author, body, created_utc: created, score,
  permalink: `/r/rust/comments/abc123/x/${id}/`, is_submitter: author === 'ferris', distinguished: null, stickied: false, ...extra,
  replies: replies.length ? { data: { children: replies } } : '' } });
const thread = [
  { data: { children: [{ kind: 't3', data: post }] } },
  { data: { children: [
    comment('c1', 'crab', 'Bounded channels everywhere, and a semaphore at the edge.', 1735693200, 40, [
      comment('c2', 'ferris', 'Thanks, the semaphore is what we were missing.', 1735696800, 12),
      comment('c3', '[deleted]', '[deleted]', 1735697000, 1, [comment('c4', 'tokio_fan', 'Same here.', 1735700400, 3)]),
    ]),
  ] } },
];
const listing = { data: { after: null, children: [
  { kind: 't3', data: post },
  { kind: 't3', data: { ...post, id: 'def456', title: 'Announcing a crate', author: 'maintainer', selftext: '', is_self: false,
    domain: 'github.com', url: 'https://github.com/example/crate', link_flair_text: null, edited: 1735776000, score: 98, upvote_ratio: 0.9, num_comments: 14,
    permalink: '/r/rust/comments/def456/announcing_a_crate/', created_utc: 1735772400 } },
] } };

globalThis.fetch = async (url) => ({ ok: true, json: async () => (String(url).startsWith('/comments/') ? thread : listing) });
await sample('workflows/reddit/reddit-thread.json', ['abc123', '500', 'best'], 'reddit_items_sample.mjs');
await sample('workflows/reddit/reddit-listing.json', ['rust', 'top', 'all', '25'], 'reddit_items_sample.mjs');

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
