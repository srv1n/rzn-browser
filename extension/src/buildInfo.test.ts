import { describe, expect, it } from 'vitest';
import {
  normalizeExtensionTarget,
  RZN_EXTENSION_TARGET,
  RZN_PAGE_TEST_BRIDGE_ENABLED,
} from './buildInfo';

describe('build info', () => {
  it('normalizes supported extension build targets', () => {
    expect(normalizeExtensionTarget('chrome')).toBe('chrome');
    expect(normalizeExtensionTarget('edge')).toBe('edge');
    expect(normalizeExtensionTarget('chromium')).toBe('chromium');
    expect(normalizeExtensionTarget(' Chrome ')).toBe('chrome');
  });

  it('defaults unset or unsupported extension build targets to unknown', () => {
    expect(normalizeExtensionTarget(undefined)).toBe('unknown');
    expect(normalizeExtensionTarget('firefox')).toBe('unknown');
    expect(normalizeExtensionTarget('')).toBe('unknown');
    expect(RZN_EXTENSION_TARGET).toBe('unknown');
  });

  it('keeps the page test bridge disabled unless the build opts in', () => {
    expect(RZN_PAGE_TEST_BRIDGE_ENABLED).toBe(false);
  });
});
