import { RZN_EXTENSION_TARGET } from './buildInfo';

export const BROWSER_INSTANCE_ID_STORAGE_KEY = 'rzn_browser_instance_id';

const UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

export type BrowserInstanceStorage = Pick<chrome.storage.LocalStorageArea, 'get' | 'set'>;

type BrowserInstanceCrypto = {
  randomUUID?: () => string;
  getRandomValues?: <T extends ArrayBufferView | null>(array: T) => T;
};

type BrowserInstanceRuntime = {
  id?: string;
  getManifest?: () => chrome.runtime.Manifest;
};

type BrowserInstanceNavigator = {
  userAgent?: string;
  platform?: string;
  vendor?: string;
  language?: string;
};

type BrowserInstanceIdOptions = {
  storage?: BrowserInstanceStorage;
  crypto?: BrowserInstanceCrypto;
  runtime?: BrowserInstanceRuntime;
  navigator?: BrowserInstanceNavigator;
};

let cachedBrowserInstanceId: string | null = null;
let pendingBrowserInstanceId: Promise<string> | null = null;

export function isBrowserInstanceId(value: unknown): value is string {
  return typeof value === 'string' && UUID_PATTERN.test(value);
}

function browserInstanceStorage(): BrowserInstanceStorage {
  const storage = (globalThis as any).chrome?.storage?.local;
  if (!storage?.get || !storage?.set) {
    throw new Error('chrome.storage.local is unavailable');
  }
  return storage;
}

export function generateBrowserInstanceId(
  cryptoSource: BrowserInstanceCrypto = globalThis.crypto
): string {
  const randomUuid = cryptoSource?.randomUUID?.();
  if (isBrowserInstanceId(randomUuid)) {
    return randomUuid;
  }

  if (typeof cryptoSource?.getRandomValues !== 'function') {
    throw new Error('crypto.randomUUID and crypto.getRandomValues are unavailable');
  }

  const bytes = new Uint8Array(16);
  cryptoSource.getRandomValues(bytes);
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;

  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0'));
  return [
    hex.slice(0, 4).join(''),
    hex.slice(4, 6).join(''),
    hex.slice(6, 8).join(''),
    hex.slice(8, 10).join(''),
    hex.slice(10, 16).join(''),
  ].join('-');
}

export async function ensureBrowserInstanceId(
  options: BrowserInstanceIdOptions = {}
): Promise<string> {
  if (cachedBrowserInstanceId) {
    return cachedBrowserInstanceId;
  }
  if (pendingBrowserInstanceId) {
    return pendingBrowserInstanceId;
  }

  const storage = options.storage ?? browserInstanceStorage();
  pendingBrowserInstanceId = (async () => {
    const stored = await storage.get(BROWSER_INSTANCE_ID_STORAGE_KEY);
    const existing = stored?.[BROWSER_INSTANCE_ID_STORAGE_KEY];
    if (isBrowserInstanceId(existing)) {
      cachedBrowserInstanceId = existing;
      return existing;
    }

    const generated = generateBrowserInstanceId(options.crypto);
    await storage.set({ [BROWSER_INSTANCE_ID_STORAGE_KEY]: generated });
    cachedBrowserInstanceId = generated;
    return generated;
  })().finally(() => {
    pendingBrowserInstanceId = null;
  });

  return pendingBrowserInstanceId;
}

export function getCachedBrowserInstanceId(): string | null {
  return cachedBrowserInstanceId;
}

function extensionRuntime(options: BrowserInstanceIdOptions): BrowserInstanceRuntime | undefined {
  return options.runtime ?? (globalThis as any).chrome?.runtime;
}

function browserNavigator(options: BrowserInstanceIdOptions): BrowserInstanceNavigator | undefined {
  return options.navigator ?? globalThis.navigator;
}

function extensionTargetHint(manifestVersion: number | null): string {
  if (manifestVersion === 2) return 'firefox-mv2';
  if (manifestVersion === 3) return 'chromium-mv3';
  return 'unknown';
}

export function extensionRuntimePingMetadata(options: BrowserInstanceIdOptions = {}) {
  const runtime = extensionRuntime(options);
  const manifest = runtime?.getManifest?.();
  const manifestVersion =
    typeof manifest?.manifest_version === 'number' ? manifest.manifest_version : null;
  const extensionId = typeof runtime?.id === 'string' && runtime.id ? runtime.id : null;
  const navigatorInfo = browserNavigator(options);

  return {
    extension_id: extensionId,
    extension_origin: extensionId ? `chrome-extension://${extensionId}/` : null,
    extension_target: RZN_EXTENSION_TARGET,
    extension_manifest_version: manifestVersion,
    extension_target_hint: extensionTargetHint(manifestVersion),
    browser_diagnostics: {
      user_agent: navigatorInfo?.userAgent ?? null,
      platform: navigatorInfo?.platform ?? null,
      vendor: navigatorInfo?.vendor ?? null,
      language: navigatorInfo?.language ?? null,
    },
  };
}

export async function browserInstancePingMetadata(
  options: BrowserInstanceIdOptions = {}
): Promise<ReturnType<typeof extensionRuntimePingMetadata> & { browser_instance_id: string }> {
  return {
    ...extensionRuntimePingMetadata(options),
    browser_instance_id: await ensureBrowserInstanceId(options),
  };
}

export function resetBrowserInstanceIdCacheForTests(): void {
  cachedBrowserInstanceId = null;
  pendingBrowserInstanceId = null;
}
