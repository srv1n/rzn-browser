import { describe, expect, it } from 'vitest';
import { retainedWorkflowTabLifecyclePayload, updateRetainedWorkflowTabState } from './workflowTabRetention';

describe('retained workflow tab lifecycle', () => {
  it('preserves omitted lifecycle fields across independent tab updates', () => {
    const discarded = updateRetainedWorkflowTabState(undefined, { discarded: true }, 10);
    expect(updateRetainedWorkflowTabState(discarded, { frozen: true }, 20))
      .toEqual({ frozen: true, discarded: true, updatedAtMs: 20 });
  });

  it('reports a compact supervisor-safe payload', () => {
    const state = updateRetainedWorkflowTabState(undefined, { frozen: false, discarded: true }, 42);
    expect(retainedWorkflowTabLifecyclePayload(state))
      .toEqual({ retained: true, frozen: false, discarded: true, updated_at_ms: 42 });
  });
});
