import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  BROWSER_INSTANCE_ID_STORAGE_KEY,
  browserInstancePingMetadata,
  ensureBrowserInstanceId,
  extensionRuntimePingMetadata,
  generateBrowserInstanceId,
  resetBrowserInstanceIdCacheForTests,
} from './browserInstanceId';

const FIRST_UUID = '11111111-1111-4111-8111-111111111111';
const SECOND_UUID = '22222222-2222-4222-8222-222222222222';

function createStorage(initial: Record<string, unknown> = {}) {
  const data = { ...initial };
  return {
    data,
    get: vi.fn(async (key: string) => ({ [key]: data[key] })),
    set: vi.fn(async (items: Record<string, unknown>) => {
      Object.assign(data, items);
    }),
  };
}

describe('browser instance id', () => {
  afterEach(() => {
    resetBrowserInstanceIdCacheForTests();
    vi.restoreAllMocks();
  });

  it('generates and stores a UUID-like ID on first startup', async () => {
    const storage = createStorage();
    const randomUUID = vi.fn(() => FIRST_UUID);

    await expect(ensureBrowserInstanceId({ storage, crypto: { randomUUID } })).resolves.toBe(FIRST_UUID);

    expect(randomUUID).toHaveBeenCalledOnce();
    expect(storage.set).toHaveBeenCalledWith({
      [BROWSER_INSTANCE_ID_STORAGE_KEY]: FIRST_UUID,
    });
    expect(storage.data[BROWSER_INSTANCE_ID_STORAGE_KEY]).toBe(FIRST_UUID);
  });

  it('reuses the stored ID across service worker restarts', async () => {
    const storage = createStorage({ [BROWSER_INSTANCE_ID_STORAGE_KEY]: FIRST_UUID });
    const randomUUID = vi.fn(() => SECOND_UUID);

    await expect(ensureBrowserInstanceId({ storage, crypto: { randomUUID } })).resolves.toBe(FIRST_UUID);

    expect(randomUUID).not.toHaveBeenCalled();
    expect(storage.set).not.toHaveBeenCalled();
  });

  it('generates a new ID after local extension storage is cleared', async () => {
    const storage = createStorage({ [BROWSER_INSTANCE_ID_STORAGE_KEY]: FIRST_UUID });
    await expect(
      ensureBrowserInstanceId({ storage, crypto: { randomUUID: () => FIRST_UUID } })
    ).resolves.toBe(FIRST_UUID);

    resetBrowserInstanceIdCacheForTests();
    delete storage.data[BROWSER_INSTANCE_ID_STORAGE_KEY];

    await expect(
      ensureBrowserInstanceId({ storage, crypto: { randomUUID: () => SECOND_UUID } })
    ).resolves.toBe(SECOND_UUID);
    expect(storage.data[BROWSER_INSTANCE_ID_STORAGE_KEY]).toBe(SECOND_UUID);
  });

  it('includes the ID in ping metadata once initialized', async () => {
    const storage = createStorage();

    await expect(
      browserInstancePingMetadata({
        storage,
        crypto: { randomUUID: () => FIRST_UUID },
        runtime: {
          id: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
          getManifest: () => ({ manifest_version: 3, name: 'RZN Browser Automation', version: '0.1.1' }),
        },
        navigator: {
          userAgent: 'diagnostic user agent',
          platform: 'macOS',
          vendor: 'Chromium',
          language: 'en-US',
        },
      })
    ).resolves.toEqual({
      browser_instance_id: FIRST_UUID,
      extension_id: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
      extension_origin: 'chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/',
      extension_target: 'unknown',
      extension_manifest_version: 3,
      extension_target_hint: 'chromium-mv3',
      browser_diagnostics: {
        user_agent: 'diagnostic user agent',
        platform: 'macOS',
        vendor: 'Chromium',
        language: 'en-US',
      },
    });
  });

  it('reports the Firefox manifest target hint without user-agent routing', () => {
    expect(
      extensionRuntimePingMetadata({
        runtime: {
          id: 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
          getManifest: () => ({ manifest_version: 2, name: 'RZN Browser Automation', version: '0.1.1' }),
        },
        navigator: {
          userAgent: 'diagnostic Firefox UA',
        },
      })
    ).toMatchObject({
      extension_id: 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
      extension_origin: 'chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/',
      extension_target: 'unknown',
      extension_manifest_version: 2,
      extension_target_hint: 'firefox-mv2',
      browser_diagnostics: {
        user_agent: 'diagnostic Firefox UA',
      },
    });
  });

  it('falls back to crypto.getRandomValues when randomUUID is unavailable', () => {
    let next = 0;
    const id = generateBrowserInstanceId({
      getRandomValues: (bytes) => {
        if (bytes instanceof Uint8Array) {
          bytes.forEach((_, index) => {
            bytes[index] = next++;
          });
        }
        return bytes;
      },
    });

    expect(id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  });
});
