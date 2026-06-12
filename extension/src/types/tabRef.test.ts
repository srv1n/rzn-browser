import { describe, expect, it } from 'vitest';
import { parseTabRef, serializeTabRef } from './tabRef';

describe('tabRef', () => {
  it('round trips and distinguishes browser instances', () => {
    const chrome = serializeTabRef({ browser_instance_id: 'chrome-instance', tab_id: 123 });
    const edge = serializeTabRef({ browser_instance_id: 'edge-instance', tab_id: 123 });

    expect(chrome).toBe('rzn://browser/chrome-instance/tab/123');
    expect(parseTabRef(chrome)).toEqual({ browser_instance_id: 'chrome-instance', tab_id: 123 });
    expect(chrome).not.toBe(edge);
  });

  it('rejects malformed refs', () => {
    for (const value of [
      '',
      'https://browser/chrome-instance/tab/1',
      'rzn://browser//tab/1',
      'rzn://browser/chrome-instance/tab/',
      'rzn://browser/chrome-instance/tab/abc',
      'rzn://browser/chrome-instance/tab/-1',
      'rzn://browser/chrome-instance/tab/1/extra',
      'rzn://browser/chrome/instance/tab/1',
    ]) {
      expect(() => parseTabRef(value)).toThrow();
    }
  });

  it('rejects invalid serializer inputs', () => {
    expect(() => serializeTabRef({ browser_instance_id: '', tab_id: 1 })).toThrow();
    expect(() => serializeTabRef({ browser_instance_id: 'chrome/instance', tab_id: 1 })).toThrow();
    expect(() => serializeTabRef({ browser_instance_id: 'chrome-instance', tab_id: -1 })).toThrow();
  });
});
