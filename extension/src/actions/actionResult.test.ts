import { describe, expect, it } from 'vitest';
import {
  actionFailure,
  actionResultFailureMessage,
  actionSuccess,
  isActionResultFailure,
  normalizeActionResult,
} from './actionResult';

describe('canonical action results', () => {
  it('builds a canonical success result', () => {
    const result = actionSuccess({
      action: 'click_element',
      result: { clicked: true },
      tabId: 7,
      duration_ms: 12,
      timestamp: 1234,
    });

    expect(result).toEqual({
      success: true,
      status: 'ok',
      action: 'click_element',
      result: { clicked: true },
      tabId: 7,
      timestamp: 1234,
      duration_ms: 12,
      warnings: [],
      artifacts: [],
    });
  });

  it('builds a canonical failure result', () => {
    const result = actionFailure({
      action: 'upload_file',
      error: new Error('no file input'),
      error_code: 'UPLOAD_FILE_ERROR',
      timestamp: 4321,
    });

    expect(result).toEqual({
      success: false,
      status: 'error',
      action: 'upload_file',
      result: null,
      error: 'no file input',
      error_msg: 'no file input',
      error_code: 'UPLOAD_FILE_ERROR',
      timestamp: 4321,
      warnings: [],
      artifacts: [],
    });
  });

  it('passes canonical results through normalization unchanged', () => {
    const typed = actionSuccess({
      action: 'type_text',
      result: { inserted: true, textLength: 4 },
      timestamp: 1234,
    });

    expect(normalizeActionResult('type_text', typed)).toBe(typed);
  });

  it('wraps raw success values in the canonical result field', () => {
    const value = { clicked: true, selector: '#go' };

    expect(normalizeActionResult('click_element', value, { timestamp: 1234 })).toEqual({
      success: true,
      status: 'ok',
      action: 'click_element',
      result: value,
      timestamp: 1234,
      warnings: [],
      artifacts: [],
    });
  });

  it('normalizes failures with canonical error fields', () => {
    const result = normalizeActionResult('click_element', {
      success: false,
      error: 'element not found',
    });

    expect(result).toMatchObject({
      success: false,
      status: 'error',
      action: 'click_element',
      result: null,
      error: 'element not found',
      error_msg: 'element not found',
      warnings: [],
      artifacts: [],
    });
  });

  it('detects failure-shaped results for execute wrappers', () => {
    expect(isActionResultFailure({ success: false, error: 'boom' })).toBe(true);
    expect(isActionResultFailure({ status: 'error', error_msg: 'boom' })).toBe(true);
    expect(isActionResultFailure({ success: true, result: { ok: true } })).toBe(false);
    expect(actionResultFailureMessage({ success: false, error_msg: 'boom' })).toBe('boom');
  });
});
