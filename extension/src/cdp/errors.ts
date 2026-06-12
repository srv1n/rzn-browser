export function cdpErrorText(error: unknown): string {
  if (typeof error === 'string') return error;
  if (!error || typeof error !== 'object') return String(error);

  const anyError = error as {
    message?: unknown;
    code?: unknown;
    data?: unknown;
    description?: unknown;
    toString?: () => string;
  };
  const parts: string[] = [];

  if (typeof anyError.message === 'string' && anyError.message.trim()) {
    parts.push(anyError.message.trim());
  }
  if (typeof anyError.description === 'string' && anyError.description.trim()) {
    parts.push(anyError.description.trim());
  }
  if (typeof anyError.code === 'number') {
    parts.push(`code=${anyError.code}`);
  }
  if (anyError.data !== undefined) {
    try {
      const dataText =
        typeof anyError.data === 'string' ? anyError.data : JSON.stringify(anyError.data);
      if (dataText) parts.push(`data=${dataText}`);
    } catch {}
  }

  if (parts.length) return parts.join(' ');

  try {
    return JSON.stringify(error);
  } catch {
    return anyError.toString?.() || String(error);
  }
}

export function isExpectedCdpLifecycleError(error: unknown): boolean {
  const text = cdpErrorText(error).toLowerCase();
  return (
    text.includes('detached while handling command') ||
    text.includes('debugger is not attached') ||
    text.includes('no tab with given id') ||
    text.includes('the tab was closed') ||
    text.includes('target closed') ||
    text.includes('session closed') ||
    text.includes('inspected target navigated or closed')
  );
}

export function isExecutionContextDestroyedCdpError(error: unknown): boolean {
  const text = cdpErrorText(error).toLowerCase();
  return (
    text.includes('promise was collected') ||
    text.includes('inspected target navigated or closed') ||
    text.includes('execution context was destroyed') ||
    text.includes('cannot find context with specified id') ||
    text.includes('detached while handling command')
  );
}
