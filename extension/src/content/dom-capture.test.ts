import assert from 'node:assert/strict';
import { afterEach, beforeEach, describe, test } from 'vitest';
import { breadthFirst, buildSnapshot, captureCurrentDOM, domHash, INTERACTIVE_SELECTOR, visible } from './dom-capture';

// Small deterministic DOM fixture. Counts browser reads without timing assertions.
describe('DOM capture work and compatibility', () => {
  let reads: { rect: number; style: number; query: number };
  let interactive: FixtureElement[];
  let body: FixtureElement;
  let savedGlobals: Map<string, PropertyDescriptor | undefined>;

  class FixtureElement {
    children: FixtureElement[] = [];
    parentElement: FixtureElement | null = null;
    className = '';
    textContent = 'example';
    attrs: Record<string, string>;
    style = { visibility: 'visible', display: 'block' };
    rect = { left: 10, top: 10, right: 110, bottom: 30, width: 100, height: 20 };
    constructor(public tagName: string, public id: string) { this.attrs = { id }; }
    getAttribute(name: string): string | null { return this.attrs[name] ?? null; }
    getBoundingClientRect() { reads.rect++; return this.rect; }
    append(...children: FixtureElement[]) {
      for (const child of children) { child.parentElement = this; this.children.push(child); }
    }
  }

  const asElement = (element: FixtureElement) => element as unknown as Element;

  function populate(count: number): void {
    interactive = Array.from({ length: count }, (_, i) => new FixtureElement('BUTTON', `button-${i}`));
    body.append(...interactive);
  }

  beforeEach(() => {
    reads = { rect: 0, style: 0, query: 0 };
    interactive = [];
    body = new FixtureElement('BODY', 'body');
    savedGlobals = new Map(['HTMLElement', 'window', 'document'].map(key =>
      [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
    Object.assign(globalThis, {
      HTMLElement: FixtureElement,
      window: {
        innerHeight: 900, innerWidth: 1200,
        location: { href: 'https://example.test/' },
        getComputedStyle: (element: FixtureElement) => { reads.style++; return element.style; },
      },
      document: {
        body, title: 'Fixture',
        querySelectorAll: (selector: string) => {
          reads.query++;
          return selector === INTERACTIVE_SELECTOR ? interactive : [];
        },
      },
    });
  });

  afterEach(() => {
    for (const [key, descriptor] of savedGlobals) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else Reflect.deleteProperty(globalThis, key);
    }
  });

  test('default capture uses one selector scan and one rectangle read per retained element', () => {
    populate(200);
    const captured = captureCurrentDOM();
    assert.equal(captured.elements.length, 120);
    assert.deepEqual(reads, { query: 1, rect: 120, style: 120 });
    assert.equal(captured.hash, domHash());
  });

  test('buildSnapshot reuses geometry in the interactive and fallback paths', () => {
    populate(1);
    body.append(new FixtureElement('DIV', 'content'));
    const elements = buildSnapshot(10);
    assert.deepEqual(elements.map(element => element.selector), ['#button-0', '#body', '#content']);
    assert.equal(reads.rect, 3);
  });

  test('hashes remain independent of the requested output limit', () => {
    populate(200);
    const expected = domHash();
    for (const limit of [-1, 0, 1, 10, 49, 50, 51, 120, 300]) {
      assert.equal(captureCurrentDOM(limit).hash, expected, `limit ${limit}`);
    }
  });

  test('a small capture still detects changes outside its returned elements', () => {
    populate(80);
    const before = captureCurrentDOM(10);
    interactive[40].textContent = 'changed outside the output limit';
    const after = captureCurrentDOM(10);
    assert.deepEqual(after.elements, before.elements);
    assert.notEqual(after.hash, before.hash);
  });

  test('stable IDs, element order, prompt references, and viewport metadata are retained', () => {
    populate(3);
    const first = captureCurrentDOM(50);
    const second = captureCurrentDOM(50);
    assert.deepEqual(second.elements, first.elements);
    assert.equal(second.prompt, first.prompt);
    assert.deepEqual(first.elements.map(element => element.selector),
      ['#button-0', '#button-1', '#button-2', '#body']);
    assert.ok(first.prompt.includes('ref="@e1"'));
    assert.equal(first.metadata.url, 'https://example.test/');
    assert.deepEqual(first.metadata.viewport, { width: 1200, height: 900 });
  });

  test('offscreen and zero-size nodes avoid computed-style reads', () => {
    body.rect.top = 1000;
    body.rect.bottom = 1020;
    assert.equal(visible(asElement(body)), false);
    body.rect.top = 10;
    body.rect.bottom = 30;
    body.rect.width = 0;
    assert.equal(visible(asElement(body)), false);
    assert.equal(reads.style, 0);
  });

  test('visibility rules continue to reject hidden, non-HTML, and non-finite geometry', () => {
    body.style.visibility = 'hidden';
    assert.equal(visible(asElement(body)), false);
    body.style.visibility = 'visible';
    body.style.display = 'none';
    assert.equal(visible(asElement(body)), false);
    body.style.display = 'block';
    body.rect.width = NaN;
    assert.equal(visible(asElement(body)), false);
    assert.equal(visible({} as Element), false);
  });

  for (const useInteractivePath of [true, false]) {
    test(`omits password values in the ${useInteractivePath ? 'interactive' : 'fallback'} path`, () => {
      const password = new FixtureElement('INPUT', 'password');
      password.attrs = { id: 'password', type: 'PassWord', value: 'must-not-leak', name: 'login-password' };
      password.textContent = '';
      const text = new FixtureElement('INPUT', 'username');
      text.attrs = { id: 'username', type: 'text', value: 'keep-this-value' };
      body.append(password, text);
      if (useInteractivePath) interactive = [password, text];
      const capture = captureCurrentDOM(50);
      const field = capture.elements.find(element => element.selector === '#password')!;
      assert.equal(field.attributes.type, 'PassWord');
      assert.equal(field.attributes.name, 'login-password');
      assert.equal(Object.hasOwn(field.attributes, 'value'), false);
      assert.equal(capture.elements.find(element => element.selector === '#username')!.attributes.value, 'keep-this-value');
      assert.equal(JSON.stringify(capture).includes('must-not-leak'), false);
      assert.equal(password.attrs.value, 'must-not-leak', 'capture must not mutate the input');
    });
  }

  test('breadth-first traversal keeps level ordering for wide trees', () => {
    const nodes = Array.from({ length: 4000 }, (_, i) => new FixtureElement('DIV', `node-${i}`));
    body.append(...nodes);
    nodes[0].append(new FixtureElement('SPAN', 'grandchild'));
    const visited = [...breadthFirst(asElement(body))];
    assert.equal(visited.length, 4002);
    assert.equal(visited[0], body);
    assert.equal(visited[4000], nodes[3999]);
    assert.equal(visited[4001], nodes[0].children[0]);
  });

  test('reads children after yielding and does not revisit moved nodes', () => {
    const a = new FixtureElement('DIV', 'a');
    const b = new FixtureElement('DIV', 'b');
    const c = new FixtureElement('DIV', 'c');
    body.append(a, b);
    const iterator = breadthFirst(asElement(body));
    assert.equal(iterator.next().value, body);
    body.append(c);
    assert.equal(iterator.next().value, a);
    // Simulate a node moved into a later subtree while the consumer is paused.
    b.children.push(a);
    assert.deepEqual([...iterator], [b, c]);
  });

  test('supports an empty document before body creation', () => {
    assert.deepEqual([...breadthFirst(null)], []);
    Object.assign(document, { body: null });
    const capture = captureCurrentDOM();
    assert.deepEqual(capture.elements, []);
    assert.equal(capture.hash, domHash());
  });
});
