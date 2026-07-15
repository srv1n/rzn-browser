export class TestElement {
  innerHTML = ''; textContent = ''; className = ''; hidden = false; disabled = false;
  value = ''; checked = false; dataset: Record<string, string> = {}; scrollTop = 0; clientHeight = 1; scrollHeight = 1;
  private listeners = new Map<string, (event: any) => void | Promise<void>>();
  constructor(public children = new Map<string, TestElement>()) {}
  querySelector<T = TestElement>(selector: string): T | null { return (this.children.get(selector) || null) as T | null; }
  querySelectorAll<T = TestElement>(_selector: string): T[] { return []; }
  closest<T = TestElement>(_selector: string): T | null { return (this.children.get('__closest') || null) as T | null; }
  addEventListener(name: string, listener: (event: any) => void | Promise<void>): void { this.listeners.set(name, listener); }
  async fire(name: string): Promise<void> { await this.listeners.get(name)?.({ target: this, currentTarget: this, preventDefault() {} }); }
}

export class TestRoot extends TestElement {
  all = new Map<string, TestElement[]>();
  override querySelectorAll<T = TestElement>(selector: string): T[] { return (this.all.get(selector) || []) as T[]; }
}

export const route = (tab: string, id?: string, query = '') => ({ tab, id, query: new URLSearchParams(query) });
