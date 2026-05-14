import { describe, expect, it } from 'vitest';
import {
  actionFailure,
  actionResultFailureMessage,
  actionSuccess,
  isActionResultFailure,
  normalizeActionResult,
} from './actionResult';

describe('action result normalizer', () => {
  it('builds additive success results with legacy fields preserved', () => {
    const result = actionSuccess({
      action: 'click_element',
      result: { clicked: true },
      tabId: 7,
      duration_ms: 12,
      legacy: { selector: '#go' },
    });

    expect(result).toMatchObject({
      success: true,
      status: 'ok',
      action: 'click_element',
      result: { clicked: true },
      selector: '#go',
      tabId: 7,
      duration_ms: 12,
      warnings: [],
      artifacts: [],
    });
    expect(typeof result.timestamp).toBe('number');
  });

  it('does not let legacy fields override canonical success fields', () => {
    const result = actionSuccess({
      action: 'click_element',
      result: { clicked: true },
      timestamp: 1234,
      legacy: {
        success: false,
        status: 'error',
        result: { clicked: false },
        error: 'legacy error',
        error_msg: 'legacy error message',
        timestamp: 1,
        selector: '#go',
      },
    });

    expect(result).toMatchObject({
      success: true,
      status: 'ok',
      action: 'click_element',
      result: { clicked: true },
      selector: '#go',
      timestamp: 1234,
    });
    expect(result.error).toBeUndefined();
    expect(result.error_msg).toBeUndefined();
  });

  it('does not let legacy fields override canonical failure fields', () => {
    const result = actionFailure({
      action: 'upload_file',
      error: 'canonical error',
      timestamp: 4321,
      legacy: {
        success: true,
        status: 'ok',
        result: { uploaded: true },
        error: 'legacy error',
        timestamp: 1,
        input: 'file',
      },
    });

    expect(result).toMatchObject({
      success: false,
      status: 'error',
      action: 'upload_file',
      result: null,
      error: 'canonical error',
      error_msg: 'canonical error',
      input: 'file',
      timestamp: 4321,
    });
  });

  it('passes already typed results through unchanged', () => {
    const typed = actionSuccess({
      action: 'type_text',
      result: { inserted: true, textLength: 4 },
      legacy: { textLength: 4 },
    });

    expect(normalizeActionResult('type_text', typed)).toBe(typed);
  });

  it('normalizes errors with stable message fields', () => {
    const result = actionFailure({
      action: 'upload_file',
      error: new Error('no file input'),
      error_code: 'UPLOAD_FILE_ERROR',
    });

    expect(result).toMatchObject({
      success: false,
      status: 'error',
      action: 'upload_file',
      result: null,
      error: 'no file input',
      error_msg: 'no file input',
      error_code: 'UPLOAD_FILE_ERROR',
    });
  });

  it('normalizes legacy failure payloads as failures', () => {
    const result = normalizeActionResult('click_element_enhanced', {
      success: false,
      error: 'element not found',
      selector: '#missing',
    });

    expect(result).toMatchObject({
      success: false,
      status: 'error',
      action: 'click_element_enhanced',
      result: null,
      error: 'element not found',
      error_msg: 'element not found',
      selector: '#missing',
      warnings: [],
      artifacts: [],
    });
  });

  it('normalizes typed-shaped but non-canonical failures instead of passing them through', () => {
    const legacy = {
      success: false,
      status: 'ok',
      action: 'legacy_action',
      result: { clicked: true },
      error: 'legacy failure',
      selector: '#bad',
    };

    const result = normalizeActionResult('click_element_enhanced', legacy);

    expect(result).not.toBe(legacy);
    expect(result).toMatchObject({
      success: false,
      status: 'error',
      action: 'click_element_enhanced',
      result: null,
      error: 'legacy failure',
      error_msg: 'legacy failure',
      selector: '#bad',
    });
  });

  it('normalizes status-error legacy payloads as failures', () => {
    const result = normalizeActionResult('hover_element_enhanced', {
      status: 'error',
      error_msg: 'hover target disappeared',
      selector: '#gone',
    });

    expect(result).toMatchObject({
      success: false,
      status: 'error',
      action: 'hover_element_enhanced',
      result: null,
      error: 'hover target disappeared',
      error_msg: 'hover target disappeared',
      selector: '#gone',
    });
  });

  it('normalizes legacy success payloads while preserving additive fields', () => {
    const legacy = {
      success: true,
      clicked: true,
      selector: '#go',
    };

    const result = normalizeActionResult('click_element_enhanced', legacy);

    expect(result).toMatchObject({
      success: true,
      status: 'ok',
      action: 'click_element_enhanced',
      result: legacy,
      clicked: true,
      selector: '#go',
    });
  });

  it('detects failure-shaped action results for outer execute wrappers', () => {
    expect(isActionResultFailure({ success: false, error: 'boom' })).toBe(true);
    expect(isActionResultFailure({ status: 'error', error_msg: 'boom' })).toBe(true);
    expect(isActionResultFailure({ success: true, result: { ok: true } })).toBe(false);
    expect(actionResultFailureMessage({ success: false, error_msg: 'boom' })).toBe('boom');
  });
});
