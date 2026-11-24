// Hand-built X samples through the capture path: the workflows' own mapping step run against a
// hand-built fixture of what capture_responses keeps (UserTweets / SearchTimeline batches and the
// post page's TweetResultByRestId / TweetDetail, shaped like X's GraphQL responses) plus the few
// cards still in the DOM. Not a recording:
// replace with real runs (`source: "recorded"`).
//
//   node scripts/workflow_samples/x_items_sample.mjs            # check samples are current
//   node scripts/workflow_samples/x_items_sample.mjs --write    # rewrite them
import { finish, sample } from './sample_lib.mjs';

const user = (id, sn, name) => ({ __typename: 'User', rest_id: id, core: { screen_name: sn, name }, legacy: {} });
const jack = user('12', 'jack', 'jack');
const tweet = (id, fields) => ({ __typename: 'Tweet', rest_id: id,
  core: { user_results: { result: fields.user || jack } }, views: { count: fields.views || '1000' },
  edit_control: { edit_tweet_ids: fields.edits || [id] },
  legacy: { id_str: id, lang: 'en', user_id_str: (fields.user || jack).rest_id, ...fields.legacy }, ...fields.extra });
const created = (iso) => new Date(iso).toUTCString().replace('GMT', '+0000');

// A two-post self-thread; the second post quotes another account and carries an image.
const thread1 = tweet('1790000000000000001', { legacy: { created_at: created('2026-11-20T08:15:00Z'), conversation_id_str: '1790000000000000001',
  full_text: 'Shipping notes for this week, a thread. https://t.co/abc',
  entities: { urls: [{ url: 'https://t.co/abc', expanded_url: 'https://example.com/notes' }] },
  favorite_count: 4100, retweet_count: 320, reply_count: 85, quote_count: 12, bookmark_count: 40 }, views: '120000' });
const thread2 = tweet('1790000000000000002', { legacy: { created_at: created('2026-11-20T08:16:30Z'), conversation_id_str: '1790000000000000001',
  in_reply_to_status_id_str: '1790000000000000001', in_reply_to_user_id_str: '12',
  full_text: 'Second: the chart below &amp; the post it answers. https://t.co/img',
  extended_entities: { media: [{ type: 'photo', media_url_https: 'https://pbs.twimg.com/media/chart.jpg', ext_alt_text: 'chart of weekly deploys', url: 'https://t.co/img' }] },
  favorite_count: 900, retweet_count: 30, reply_count: 10, quote_count: 1, bookmark_count: 5 }, views: '40000',
  extra: { quoted_status_result: { result: { __typename: 'TweetWithVisibilityResults', tweet: { __typename: 'Tweet', rest_id: '1780000000000000009',
    core: { user_results: { result: user('99', 'other', 'Other Person') } },
    legacy: { id_str: '1780000000000000009', user_id_str: '99', full_text: 'The quoted post text.' } } } } } });
// A repost of @other: listed by X, skipped by the mapping.
const repost = tweet('1789500000000000000', { legacy: { created_at: created('2026-11-19T12:00:00Z'), full_text: 'RT @other: something',
  retweeted_status_result: { result: tweet('1786000000000000003', { user: user('99', 'other', 'Other Person'), legacy: { full_text: 'something' } }) } } });
// A lone long post, edited once; its card text was cut, the note text is whole.
const longPost = tweet('1789000000000000005', { legacy: { created_at: created('2026-11-18T21:00:00Z'), conversation_id_str: '1789000000000000005',
  full_text: 'Long post preview…', favorite_count: 12, retweet_count: 1, reply_count: 2, quote_count: 0, bookmark_count: 3 },
  edits: ['1788999999999999990', '1789000000000000005'],
  extra: { note_tweet: { note_tweet_results: { result: { text: 'A long post that runs past 280 characters, read in full from the note text rather than the cut card.' } } } } });
// A reply to another account: its own item, with the post it answers as a field.
const reply = tweet('1788000000000000007', { legacy: { created_at: created('2026-11-17T10:00:00Z'), conversation_id_str: '1787000000000000001',
  in_reply_to_status_id_str: '1787000000000000001', in_reply_to_user_id_str: '99',
  full_text: '@other Agreed, and measured it too.', display_text_range: [7, 35], favorite_count: 3, retweet_count: 0, reply_count: 0, quote_count: 0, bookmark_count: 0 }, views: '310' });
// An ad X slots into the timeline: dropped.
const promoted = tweet('1791000000000000000', { user: jack, legacy: { created_at: created('2026-11-21T00:00:00Z'), full_text: 'Promoted.' } });

const item = (t, extra = {}) => ({ itemType: 'TimelineTweet', __typename: 'TimelineTweet', tweet_results: { result: t }, tweetDisplayType: 'Tweet', ...extra });
const entry = (t, extra) => ({ entryId: 'tweet-' + t.rest_id, sortIndex: t.rest_id, content: { entryType: 'TimelineTimelineItem', __typename: 'TimelineTimelineItem', itemContent: item(t, extra) } });
const conversation = (id, ts) => ({ entryId: 'profile-conversation-' + id, sortIndex: id, content: { entryType: 'TimelineTimelineModule', __typename: 'TimelineTimelineModule',
  items: ts.map((t) => ({ entryId: 'profile-conversation-' + id + '-tweet-' + t.rest_id, item: { itemContent: item(t) } })), displayType: 'VerticalConversation' } });
const cursor = (value, type) => ({ entryId: 'cursor-' + type.toLowerCase() + '-' + value, sortIndex: value, content: { entryType: 'TimelineTimelineCursor', __typename: 'TimelineTimelineCursor', value, cursorType: type } });
const response = (url, data) => ({ seq: 0, url, method: 'GET', status: 200, mime_type: 'application/json', resource_type: 'Fetch', bytes: JSON.stringify(data).length, json: data });
const batches = (op, wrap) => {
  const url = (n) => `https://x.com/i/api/graphql/abcQueryId/${op}?variables=%7B%22count%22%3A20%2C%22page%22%3A${n}%7D`;
  const first = [entry(promoted, { promotedMetadata: { advertiser_results: {} } }), conversation('1790000000000000002', [thread1, thread2]), entry(repost), cursor('c1', 'Bottom')];
  const second = [entry(longPost), entry(reply), cursor('c2', 'Bottom')];
  const responses = [first, second].map((entries, i) => response(url(i + 1), wrap({ instructions: [{ type: 'TimelineClearCache' }, { type: 'TimelineAddEntries', entries }] })));
  responses.forEach((r, i) => { r.seq = i + 1; });
  return { url_pattern: `/graphql/[^/?]+/${op}\\?`, responses, duplicates: 1, dropped: { too_large: 0, over_budget: 0, failed: 0, body_unavailable: 0 }, detached: null };
};
const userTweets = batches('UserTweets', (timeline) => ({ data: { user: { result: { __typename: 'User', timeline: { timeline } } } } }));
const searchTimeline = batches('SearchTimeline', (timeline) => ({ data: { search_by_raw_query: { search_timeline: { timeline } } } }));

// The DOM after scrolling: X has dropped the thread's cards; it holds the last two captured posts
// and one card (1785...) no captured batch carried, which comes back from the card, truncated.
const cards = [
  ['jack', '1789000000000000005', 'jack', 'Long post preview…', '2026-11-18T21:00:00.000Z'],
  ['jack', '1788000000000000007', 'jack', '@other Agreed, and measured it too.', '2026-11-17T10:00:00.000Z'],
  ['jack', '1785000000000000004', 'jack', 'An older post only the page still showed.', '2026-11-10T09:30:00.000Z'],
];
const article = ([sn, id, name, text, time]) => {
  const el = (attrs) => ({ getAttribute: (n) => attrs[n] ?? null, textContent: attrs.text || '', innerText: attrs.text || '', closest: () => el({ href: '/' + sn + '/status/' + id }) });
  return { querySelector: (sel) => {
    if (sel.includes("/status/'] time")) return el({});
    if (sel.includes('/status/')) return el({ href: '/' + sn + '/status/' + id });
    if (sel.includes('User-Name')) return el({ text: name });
    if (sel.includes('tweetText')) return el({ text });
    if (sel === 'time') return el({ datetime: time });
    if (sel.includes("'like'")) return el({ 'aria-label': '7 Likes. Like' });
    return null;
  } };
};
globalThis.document = { querySelectorAll: () => cards.map(article) };
// The capture path makes no request: any fetch here fails the build.
globalThis.fetch = () => { throw new Error('a read workflow must not fetch'); };

const note = 'Built by scripts/workflow_samples/x_items_sample.mjs from the workflow\'s own mapping step, a hand-built '
  + 'fixture of captured timeline responses and invented DOM cards; replace with a recorded run.';
await sample('workflows/x/x_profile_posts.json', ['jack', '20'], 'x_items_sample.mjs', { params: { captured_responses: userTweets }, note });
await sample('workflows/x/x_search_posts.json', ['jack', '40'], 'x_items_sample.mjs', { params: { captured_responses: searchTimeline }, note });

// x/open on the thread's second post: the page's own TweetResultByRestId (that post) and TweetDetail
// (its conversation: the thread, a reply from another account) responses.
const reply99 = tweet('1790000000000000050', { user: user('99', 'other', 'Other Person'), legacy: { created_at: created('2026-11-20T09:00:00Z'),
  conversation_id_str: '1790000000000000001', in_reply_to_status_id_str: '1790000000000000001', in_reply_to_user_id_str: '12', full_text: '@jack Nice chart.' } });
const detail = { data: { threaded_conversation_with_injections_v2: { instructions: [{ type: 'TimelineAddEntries',
  entries: [entry(thread1), entry(thread2), entry(reply99)] }] } } };
const byRestId = { data: { tweetResult: { result: thread2 } } };
const postPage = { url_pattern: '/graphql/[^/?]+/(TweetDetail|TweetResultByRestId)\\?', duplicates: 0, detached: null,
  dropped: { too_large: 0, over_budget: 0, failed: 0, body_unavailable: 0 },
  responses: [response('https://x.com/i/api/graphql/abcQueryId/TweetResultByRestId?variables=%7B%7D', byRestId), response('https://x.com/i/api/graphql/abcQueryId/TweetDetail?variables=%7B%7D', detail)] };
postPage.responses.forEach((r, i) => { r.seq = i + 1; });
globalThis.location = { pathname: '/jack/status/1790000000000000002' };
await sample('workflows/x/x_open.json', ['https://x.com/jack/status/1790000000000000002'], 'x_items_sample.mjs', { params: { captured_responses: postPage },
  note: note.replace('captured timeline responses and invented DOM cards', 'captured post-page responses') });
finish();
