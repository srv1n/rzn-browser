export function shouldSkipWorkflowNavigation(currentUrl: string, rawNeedle: unknown): boolean {
  const needle = typeof rawNeedle === 'string' ? rawNeedle.trim() : '';
  if (!needle) return false;

  // Match path segments, not a URL substring: `/c/chat-1` must not retain a
  // tab on `/c/chat-10`, while Project routes may prefix the same conversation.
  const pathSegments = (value: string): string[] => {
    try {
      return new URL(value, 'https://rzn.invalid').pathname.split('/').filter(Boolean);
    } catch {
      return [];
    }
  };
  const expected = pathSegments(needle);
  const actual = pathSegments(currentUrl);
  return expected.length > 0 && actual.length >= expected.length && expected.every(
    (segment, index) => actual[actual.length - expected.length + index] === segment,
  );
}

export type NavigationReuseDisposition = 'navigate' | 'skip' | 'wake_and_skip';

// A first navigation owns tab creation: creating a provisional tab first makes
// Chrome visit an unrelated URL before the real destination and needlessly
// exercises ChatGPT's anti-abuse path.
export function firstWorkflowStepNeedsInitializedTab(firstStepType: unknown): boolean {
  return (
    firstStepType !== 'open_new_tab' &&
    firstStepType !== 'close_current_tab' &&
    firstStepType !== 'navigate_to_url'
  );
}

// A discarded Chrome tab retains its URL, but it has no live renderer/content
// script. Treating it as immediately reusable makes the next action operate on
// a dead page. The caller must wake the same tab before skipping navigation.
export function navigationReuseDisposition(
  currentUrl: string,
  rawNeedle: unknown,
  discarded: boolean | undefined,
): NavigationReuseDisposition {
  if (!shouldSkipWorkflowNavigation(currentUrl, rawNeedle)) return 'navigate';
  return discarded === true ? 'wake_and_skip' : 'skip';
}

// Same site = same last two host labels (an eTLD+1 approximation). A miss on
// `co.uk`-style hosts only costs a cold navigation.
export function sameSite(fromUrl: string, toUrl: string): boolean {
  const site = (value: string): string => {
    try {
      const url = new URL(value);
      if (url.protocol !== 'http:' && url.protocol !== 'https:') return '';
      return url.hostname.split('.').slice(-2).join('.');
    } catch {
      return '';
    }
  };
  const from = site(fromUrl);
  return from !== '' && from === site(toUrl);
}

// Navigate inside the page when the tab already shows the same site: the
// request then carries the site's referrer like a click, instead of looking
// like a fresh visit typed into the address bar. A fragment-only change has no
// page load to wait for, so it stays a browser navigation.
export function canNavigateInPage(currentUrl: string, targetUrl: string, tabStatus?: string): boolean {
  if (tabStatus !== 'complete' || !sameSite(currentUrl, targetUrl)) return false;
  const strip = (value: string) => value.replace(/#.*$/, '');
  return !(targetUrl.includes('#') && strip(currentUrl) === strip(targetUrl));
}

export interface PageFacts {
  url: string;
  title: string;
  // Challenge-only selectors present on the page (see CHALLENGE_SELECTORS).
  selectorHits: string[];
  bodyText: string;
  bodyLength: number;
}

// Elements that only appear on bot-check pages. Turnstile and hCaptcha widgets
// are left out on purpose: sites embed them in ordinary login forms.
export const CHALLENGE_SELECTORS: Record<string, string> = {
  "iframe[src*='captcha-delivery.com']": 'datadome',
  '#challenge-form': 'cloudflare',
  '#challenge-running': 'cloudflare',
  '#cf-challenge-running': 'cloudflare',
  '#challenge-stage': 'cloudflare',
  '#challenge-error-text': 'cloudflare',
  '#px-captcha': 'perimeterx',
  "form[action*='validateCaptcha']": 'amazon',
};

const CHALLENGE_TITLE = /^(just a moment|attention required|verification required|verify you are human|robot or human|are you a robot|pardon our interruption)/i;
const CHALLENGE_TEXT = /verify you are human|verifying you are human|slide right to|press (and|&) hold|unusual (traffic|activity) from your (computer|device|network)|are you a robot|robot or human\?|enter the characters you see below|blocked by network security|incapsula incident id|pardon our interruption/i;

// Name of the bot check the page shows, or null. Text rules only apply to
// short pages, so an article that mentions "are you a robot" is not a wall.
export function siteChallengeKind(page: PageFacts): string | null {
  for (const hit of page.selectorHits) {
    if (CHALLENGE_SELECTORS[hit]) return CHALLENGE_SELECTORS[hit];
  }
  try {
    const url = new URL(page.url);
    if (/(^|\.)google\.[a-z.]+$/.test(url.hostname) && url.pathname.startsWith('/sorry/')) return 'google';
    if (/\/errors\/validatecaptcha/i.test(url.pathname)) return 'amazon';
  } catch {
    // Not a URL: fall through to title and text.
  }
  const short = page.bodyLength < 4000;
  if (CHALLENGE_TITLE.test(page.title.trim())) return 'challenge';
  if (short && /^access denied$/i.test(page.title.trim())) return 'blocked';
  if (short && CHALLENGE_TEXT.test(page.bodyText)) return 'challenge';
  return null;
}
