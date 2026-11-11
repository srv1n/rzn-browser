import { expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';

for (const [file, name] of [['contentScript.ts', 'serializeForTransport'], ['pageBridge.ts', 'serializeForBridge']]) {
  it(`${name} preserves nesting and shared references`, () => {
    const source = readFileSync(new URL(`./${file}`, import.meta.url), 'utf8');
    const match = source.match(new RegExp(`^function ${name}\\([\\s\\S]*?^}`, 'm'))!;
    const serialize = new Function('Element', `${stripTypeScriptTypes(match[0])}; return ${name};`)(class {});
    let nested: any = { deepest: 'intact' };
    for (let i = 0; i < 10; i++) nested = { child: nested };
    const shared = { value: 'shared' };
    expect(serialize({ nested, a: shared, b: shared, arrays: [shared, shared] })).toEqual({ nested, a: shared, b: shared, arrays: [shared, shared] });
    const cycle: any[] = [];
    cycle.push(cycle);
    expect(serialize(cycle)).toEqual(['[Circular]']);
  });
}
