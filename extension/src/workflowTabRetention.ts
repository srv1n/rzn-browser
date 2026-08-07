export interface RetainedWorkflowTabState {
  frozen: boolean;
  discarded: boolean;
  updatedAtMs: number;
}

type TabLifecycleInput = { frozen?: unknown; discarded?: unknown };

// Chrome omits unrelated fields from TabChangeInfo. Omission is not a false
// transition, so retain the previous value until Chrome explicitly changes it.
export function updateRetainedWorkflowTabState(
  previous: RetainedWorkflowTabState | undefined,
  update: TabLifecycleInput,
  updatedAtMs = Date.now(),
): RetainedWorkflowTabState {
  return {
    frozen: typeof update.frozen === 'boolean' ? update.frozen : previous?.frozen === true,
    discarded: typeof update.discarded === 'boolean' ? update.discarded : previous?.discarded === true,
    updatedAtMs,
  };
}

export function retainedWorkflowTabLifecyclePayload(
  state: RetainedWorkflowTabState | undefined,
): Record<string, unknown> | undefined {
  if (!state) return undefined;
  return { retained: true, frozen: state.frozen, discarded: state.discarded, updated_at_ms: state.updatedAtMs };
}
