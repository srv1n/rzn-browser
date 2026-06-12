import { describe, expect, it } from 'vitest';
import { buildScriptEvalBody } from './scriptingEvalBody';

describe('script eval body generation', () => {
  it('returns simple expressions', () => {
    expect(buildScriptEvalBody('document.title')).toBe('return (document.title);');
  });

  it('preserves explicit return statements', () => {
    expect(buildScriptEvalBody('return document.title;')).toBe('return document.title;');
  });

  it('returns parenthesized IIFE values even when the IIFE contains statements', () => {
    const script = `(async () => {
      const value = 41;
      for (const n of [1]) {
        return value + n;
      }
    })()`;

    expect(buildScriptEvalBody(script)).toBe(`return (${script});`);
  });

  it('accepts semicolon-prefixed IIFEs without dropping their value', () => {
    const script = `;(function () {
      let value = 2;
      return value;
    })()`;

    expect(buildScriptEvalBody(script)).toBe(`return (${script.slice(1)});`);
  });

  it('leaves statement blocks as statements', () => {
    const script = 'const value = 1; window.__value = value;';
    expect(buildScriptEvalBody(script)).toBe(script);
  });
});
