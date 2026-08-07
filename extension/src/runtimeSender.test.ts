import { describe, expect, it } from 'vitest';
import { isOwnExtensionPageSender, type RuntimeSenderIdentity } from './runtimeSender';

const extensionId = 'rzn-extension-id';
const extensionOrigin = `chrome-extension://${extensionId}/`;

function sender(overrides: Partial<RuntimeSenderIdentity> = {}): RuntimeSenderIdentity {
  return {
    id: extensionId,
    url: `${extensionOrigin}popup.html`,
    ...overrides,
  };
}

describe('isOwnExtensionPageSender', () => {
  it('accepts packaged extension pages with or without an attached tab', () => {
    expect(isOwnExtensionPageSender(sender(), extensionId, extensionOrigin)).toBe(true);
    expect(isOwnExtensionPageSender(
      sender({ url: `${extensionOrigin}dashboard.html#runs`, tab: { id: 42 } }),
      extensionId,
      extensionOrigin,
    )).toBe(true);
  });

  it('rejects content scripts, foreign extensions, and incomplete identities', () => {
    expect(isOwnExtensionPageSender(
      sender({ url: 'https://example.com/', tab: { id: 42 } }),
      extensionId,
      extensionOrigin,
    )).toBe(false);
    expect(isOwnExtensionPageSender(
      sender({ id: 'foreign-extension-id' }),
      extensionId,
      extensionOrigin,
    )).toBe(false);
    expect(isOwnExtensionPageSender(sender({ url: undefined }), extensionId, extensionOrigin)).toBe(false);
  });
});
