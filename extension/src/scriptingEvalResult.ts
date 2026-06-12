export const RZN_EVAL_ERROR_KEY = '__rzn_eval_error';

function errorText(error: unknown): string {
  if (typeof error === 'string') return error;
  const anyError = error as any;
  return (
    anyError?.message ||
    anyError?.description ||
    anyError?.toString?.() ||
    String(error)
  );
}

export function evalErrorFromScriptingResult(result: unknown): Error | null {
  const injectionResult = result as any;
  const chromeError = injectionResult?.error;
  if (chromeError) {
    const err = new Error(errorText(chromeError));
    if (chromeError?.name) err.name = String(chromeError.name);
    if (chromeError?.stack) err.stack = String(chromeError.stack);
    return err;
  }

  const payload = injectionResult?.result;
  const evalError = payload && typeof payload === 'object' ? payload[RZN_EVAL_ERROR_KEY] : null;
  if (!evalError) return null;

  const err = new Error(errorText(evalError));
  if (evalError?.name) err.name = String(evalError.name);
  if (evalError?.stack) err.stack = String(evalError.stack);
  return err;
}
