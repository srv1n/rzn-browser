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
