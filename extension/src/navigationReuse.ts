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
