export interface RuntimeSenderIdentity {
  id?: string;
  url?: string;
  tab?: { id?: number };
}

export function isOwnExtensionPageSender(
  sender: RuntimeSenderIdentity,
  extensionId: string,
  extensionOrigin: string,
): boolean {
  return sender.id === extensionId
    && typeof sender.url === 'string'
    && sender.url.startsWith(extensionOrigin);
}
