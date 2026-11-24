export interface CloseWorkflowTabResult {
  closed: boolean;
  alreadyMissing: boolean;
}

export function isAlreadyMissingTabError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return (
    message.includes('No tab with id') ||
    message.includes('No tab') ||
    message.includes('The tab was closed')
  );
}

export function exactTabMissingError(tabId: number, error: unknown): Error {
  if (!isAlreadyMissingTabError(error)) {
    return error instanceof Error ? error : new Error(String(error));
  }
  const missing = new Error(`Exact tab target ${tabId} is missing.`);
  (missing as Error & { code?: string }).code = 'TAB_MISSING';
  return missing;
}

export function isExactTabMissingError(error: unknown): error is Error & { code: 'TAB_MISSING' } {
  return typeof error === 'object' && error !== null && (error as any).code === 'TAB_MISSING';
}

// A step that may start before the tab exists (capture_responses) uses the session's
// tab only while Chrome still has it. A gone exact tab ref fails closed as TAB_MISSING
// (the CLI site queue then reruns on a fresh tab); a gone session tab returns undefined
// so the step waits for the tab navigate_to_url opens.
export async function liveTabOrDefer(
  tabId: number | undefined,
  exactTabRef: boolean,
  get: (tabId: number) => Promise<unknown>,
): Promise<number | undefined> {
  if (tabId === undefined) return undefined;
  try {
    await get(tabId);
    return tabId;
  } catch (error) {
    if (!isAlreadyMissingTabError(error)) throw error;
    if (exactTabRef) throw exactTabMissingError(tabId, error);
    return undefined;
  }
}

// `remove` deliberately receives the caller's tab ID rather than looking up an
// active tab. A terminal cleanup can therefore be idempotent without ever
// changing its target when the original tab has already disappeared.
export async function closeExactWorkflowTab(
  tabId: number,
  remove: (tabId: number) => Promise<void>,
): Promise<CloseWorkflowTabResult> {
  try {
    await remove(tabId);
    return { closed: true, alreadyMissing: false };
  } catch (error) {
    if (isAlreadyMissingTabError(error)) {
      return { closed: false, alreadyMissing: true };
    }
    throw error;
  }
}
