import { describe, expect, it } from 'vitest';
import { RZN_EVAL_ERROR_KEY, evalErrorFromScriptingResult } from './scriptingEvalResult';

describe('scripting eval results', () => {
  it('surfaces Chrome InjectionResult errors', () => {
    const err = evalErrorFromScriptingResult({
      error: {
        name: 'ReferenceError',
        message: 'arg5 is not defined',
      },
      result: undefined,
    });

    expect(err?.name).toBe('ReferenceError');
    expect(err?.message).toBe('arg5 is not defined');
  });

  it('surfaces injected wrapper errors returned as a sentinel payload', () => {
    const err = evalErrorFromScriptingResult({
      result: {
        [RZN_EVAL_ERROR_KEY]: {
          name: 'TypeError',
          message: 'boom',
        },
      },
    });

    expect(err?.name).toBe('TypeError');
    expect(err?.message).toBe('boom');
  });

  it('leaves ordinary eval results alone', () => {
    expect(evalErrorFromScriptingResult({ result: { ok: true } })).toBeNull();
  });
});
