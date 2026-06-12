export interface TabRef {
  browser_instance_id: string;
  tab_id: number;
}

const TAB_REF_PREFIX = 'rzn://browser/';
const TAB_REF_SEPARATOR = '/tab/';

export function serializeTabRef(ref: TabRef): string {
  validateBrowserInstanceId(ref.browser_instance_id);
  if (!Number.isInteger(ref.tab_id) || ref.tab_id < 0) {
    throw new Error('tab_ref tab id must be a non-negative integer');
  }
  return `${TAB_REF_PREFIX}${ref.browser_instance_id}${TAB_REF_SEPARATOR}${ref.tab_id}`;
}

export function parseTabRef(value: string): TabRef {
  if (!value.startsWith(TAB_REF_PREFIX)) {
    throw new Error('unsupported tab_ref scheme');
  }
  const rest = value.slice(TAB_REF_PREFIX.length);
  const separatorIndex = rest.indexOf(TAB_REF_SEPARATOR);
  if (separatorIndex < 0) {
    throw new Error('tab_ref must contain /tab/');
  }
  const browserInstanceId = rest.slice(0, separatorIndex);
  const tabPart = rest.slice(separatorIndex + TAB_REF_SEPARATOR.length);
  validateBrowserInstanceId(browserInstanceId);
  if (tabPart.length === 0) {
    throw new Error('tab_ref tab id is required');
  }
  if (tabPart.includes('/')) {
    throw new Error('tab_ref must not contain extra path segments');
  }
  if (tabPart.startsWith('-')) {
    throw new Error('tab_ref tab id must be non-negative');
  }
  if (!/^\d+$/.test(tabPart)) {
    throw new Error('tab_ref tab id must be numeric');
  }
  return {
    browser_instance_id: browserInstanceId,
    tab_id: Number(tabPart),
  };
}

function validateBrowserInstanceId(value: string): void {
  if (value.trim().length === 0) {
    throw new Error('tab_ref browser instance id is required');
  }
  if (value.includes('/')) {
    throw new Error('tab_ref browser instance id must not contain /');
  }
}
