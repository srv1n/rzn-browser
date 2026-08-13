import { describe, expect, it } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';

describe('execute_javascript eval result bridge', () => {
  it('preserves nested custom failures and ordinary payloads', () => {
    const source = fs.readFileSync(path.join(process.cwd(), 'src/contentScript.ts'), 'utf8');
    expect(source).toMatch(/async function tryEvalViaPageBridge[\s\S]*?return evalResponse\(\{/);
    const body = source.match(/function evalResponse\(result: any\): any \{([\s\S]*?)\n\}\n\nfunction isTypedActionResultEnvelope/)?.[1];
    if (!body) throw new Error('evalResponse helper not found');
    const evalResponse = new Function(
      'isActionResultFailure',
      'actionResultFailureMessage',
      `return function evalResponse(result) {${body}}`,
    )(
      (value: any) => value?.success !== true && typeof value?.error_code === 'string',
      (value: any) => value.error_msg,
    );

    expect(evalResponse({ success: true, result: { success: false, error_code: 'BOUNDARY_SUPERSEDED', error_msg: 'later turn' } })).toMatchObject({
      success: false,
      error_code: 'BOUNDARY_SUPERSEDED',
      error_msg: 'later turn',
    });
    const payload = { selected_message_id: 'assistant-1' };
    expect(evalResponse({ success: true, result: payload })).toEqual({ success: true, result: payload });
  });
});
