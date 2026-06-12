declare const __RZN_BUILD_SIGNATURE__: string | undefined;
declare const __RZN_EXTENSION_TARGET__: string | undefined;
declare const __RZN_PAGE_TEST_BRIDGE_ENABLED__: boolean | undefined;

export type RznExtensionTarget = 'chrome' | 'edge' | 'chromium' | 'unknown';

export function normalizeExtensionTarget(value: unknown): RznExtensionTarget {
  if (typeof value !== 'string') return 'unknown';
  const normalized = value.trim().toLowerCase();
  if (normalized === 'chrome' || normalized === 'edge' || normalized === 'chromium') {
    return normalized;
  }
  return 'unknown';
}

export const RZN_BUILD_SIGNATURE =
  typeof __RZN_BUILD_SIGNATURE__ === 'string' && __RZN_BUILD_SIGNATURE__.trim().length > 0
    ? __RZN_BUILD_SIGNATURE__
    : 'dev-unknown';

export const RZN_EXTENSION_TARGET = normalizeExtensionTarget(
  typeof __RZN_EXTENSION_TARGET__ === 'string' ? __RZN_EXTENSION_TARGET__ : undefined
);

export const RZN_PAGE_TEST_BRIDGE_ENABLED =
  typeof __RZN_PAGE_TEST_BRIDGE_ENABLED__ === 'boolean'
    ? __RZN_PAGE_TEST_BRIDGE_ENABLED__
    : false;
