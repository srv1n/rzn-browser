export type ActionStatus = 'ok' | 'error';

export type ActionWarning = {
  code: string;
  message: string;
  details?: Record<string, any>;
};

export type ActionArtifact = {
  kind: 'file' | 'download' | 'screenshot' | 'json' | 'text';
  name?: string;
  path?: string;
  url?: string;
  mime_type?: string;
  size_bytes?: number;
  metadata?: Record<string, any>;
};

export type ActionResultMeta = {
  tabId?: number;
  timestamp?: number;
  duration_ms?: number;
  rung_used?: number | string;
  escalated?: boolean;
};

export type ActionSuccessInput<T = any> = ActionResultMeta & {
  action: string;
  result?: T;
  legacy?: Record<string, any>;
  warnings?: ActionWarning[];
  artifacts?: ActionArtifact[];
  debug?: Record<string, any>;
};

export type ActionErrorInput = ActionResultMeta & {
  action: string;
  error: unknown;
  error_code?: string;
  legacy?: Record<string, any>;
  warnings?: ActionWarning[];
  artifacts?: ActionArtifact[];
  debug?: Record<string, any>;
};

export interface TypedActionResult<T = any> {
  success: boolean;
  status: ActionStatus;
  action: string;
  result: T | null;
  warnings: ActionWarning[];
  artifacts: ActionArtifact[];
  debug?: Record<string, any>;
  error?: string;
  error_code?: string;
  error_msg?: string;
  tabId?: number;
  timestamp: number;
  duration_ms?: number;
  rung_used?: number | string;
  escalated?: boolean;
  [legacyField: string]: any;
}

const canonicalActionResultFields = new Set([
  'success',
  'status',
  'action',
  'result',
  'warnings',
  'artifacts',
  'debug',
  'error',
  'error_code',
  'error_msg',
  'tabId',
  'timestamp',
  'duration_ms',
  'rung_used',
  'escalated',
]);

function isRecord(value: unknown): value is Record<string, any> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function legacyFields(legacy?: Record<string, any>): Record<string, any> {
  if (!legacy) return {};

  return Object.fromEntries(
    Object.entries(legacy).filter(([key]) => !canonicalActionResultFields.has(key)),
  );
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (error && typeof error === 'object' && 'message' in error) {
    return String((error as any).message);
  }
  return String(error);
}

function commonFields(input: ActionResultMeta) {
  return {
    ...(typeof input.tabId === 'number' ? { tabId: input.tabId } : {}),
    timestamp: input.timestamp ?? Date.now(),
    ...(typeof input.duration_ms === 'number' ? { duration_ms: input.duration_ms } : {}),
    ...(typeof input.rung_used !== 'undefined' ? { rung_used: input.rung_used } : {}),
    ...(typeof input.escalated === 'boolean' ? { escalated: input.escalated } : {}),
  };
}

export function actionSuccess<T = any>(input: ActionSuccessInput<T>): TypedActionResult<T> {
  const result = (typeof input.result === 'undefined' ? null : input.result) as T;
  return {
    success: true,
    status: 'ok',
    action: input.action,
    result,
    warnings: input.warnings ?? [],
    artifacts: input.artifacts ?? [],
    ...(input.debug ? { debug: input.debug } : {}),
    ...commonFields(input),
    ...legacyFields(input.legacy),
  };
}

export function actionFailure(input: ActionErrorInput): TypedActionResult<null> {
  const message = errorMessage(input.error);
  return {
    success: false,
    status: 'error',
    action: input.action,
    result: null,
    error: message,
    error_msg: message,
    ...(input.error_code ? { error_code: input.error_code } : {}),
    warnings: input.warnings ?? [],
    artifacts: input.artifacts ?? [],
    ...(input.debug ? { debug: input.debug } : {}),
    ...commonFields(input),
    ...legacyFields(input.legacy),
  };
}

function isCanonicalStatus(success: boolean, status: unknown): status is ActionStatus {
  return (success && status === 'ok') || (!success && status === 'error');
}

function isTypedActionResult(value: unknown): value is TypedActionResult {
  return (
    isRecord(value) &&
    typeof value.success === 'boolean' &&
    isCanonicalStatus(value.success, value.status) &&
    typeof value.action === 'string' &&
    'result' in value &&
    Array.isArray(value.warnings) &&
    Array.isArray(value.artifacts) &&
    typeof value.timestamp === 'number'
  );
}

export function isActionResultFailure(value: unknown): value is Record<string, any> {
  if (!isRecord(value)) return false;
  if (value.success === false) return true;
  if (value.status === 'error') return true;
  return value.success !== true && ('error' in value || 'error_msg' in value || 'error_code' in value);
}

export function actionResultFailureMessage(value: unknown, fallback = 'Action failed'): string {
  if (!isRecord(value)) return fallback;
  if ('error' in value) return errorMessage(value.error);
  if ('error_msg' in value) return errorMessage(value.error_msg);
  return fallback;
}

export function normalizeActionResult<T = any>(
  action: string,
  value: unknown,
  meta: Omit<ActionSuccessInput<T>, 'action' | 'result' | 'legacy'> = {},
): TypedActionResult<T> {
  if (isTypedActionResult(value)) {
    return value as TypedActionResult<T>;
  }

  if (isActionResultFailure(value)) {
    return actionFailure({
      action,
      error: actionResultFailureMessage(value),
      ...meta,
      legacy: value,
    });
  }

  return actionSuccess<T>({
    action,
    result: value as T,
    ...meta,
    legacy: isRecord(value) ? value : undefined,
  });
}
