declare const __RZN_BUILD_SIGNATURE__: string | undefined;

export const RZN_BUILD_SIGNATURE =
  typeof __RZN_BUILD_SIGNATURE__ === 'string' && __RZN_BUILD_SIGNATURE__.trim().length > 0
    ? __RZN_BUILD_SIGNATURE__
    : 'dev-unknown';
