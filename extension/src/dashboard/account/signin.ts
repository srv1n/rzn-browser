// Sign in to RZN (backend docs/plans/browser-fleet/design.md section 3, BFL-T-0012).
// The server's own sign-in page opens in a tab as the public OAuth client `rzn-browser` with
// PKCE. Its last step returns to the extension's reserved address
// (https://<extension id>.chromiumapp.org/), which never loads: the tab is closed as soon as it
// reaches it, and the code it carries goes to the helper, which trades it for this browser's
// device token (`account.sign_in`).
import type { RpcClient } from '../shared';

export const DEFAULT_SERVER_URL = 'https://app.rzn.ai';
export const CLIENT_ID = 'rzn-browser';

type Tabs = Pick<typeof chrome.tabs, 'create' | 'remove' | 'onUpdated' | 'onRemoved'>;

export const redirectUri = (extensionId = chrome.runtime.id): string => `https://${extensionId}.chromiumapp.org/`;

export function serverUrl(settings: { server_url?: string } | undefined): string {
  return (settings?.server_url || '').trim().replace(/\/+$/, '') || DEFAULT_SERVER_URL;
}

const b64url = (bytes: Uint8Array): string =>
  btoa(String.fromCharCode(...bytes)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');

const random = (n: number): string => b64url(crypto.getRandomValues(new Uint8Array(n)));

export async function pkce(): Promise<{ verifier: string; challenge: string }> {
  const verifier = random(32);
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier));
  return { verifier, challenge: b64url(new Uint8Array(digest)) };
}

export function authorizeUrl(server: string, challenge: string, state: string, redirect: string): string {
  const url = new URL('/oauth/authorize', server);
  for (const [key, value] of Object.entries({
    response_type: 'code', client_id: CLIENT_ID, redirect_uri: redirect,
    code_challenge: challenge, code_challenge_method: 'S256', state,
  })) url.searchParams.set(key, value);
  return url.toString();
}

/** The code in `url` when it is the return address with our `state`; `null` while not there yet. */
export function codeFrom(url: string | undefined, redirect: string, state: string): string | null {
  if (!url || !url.startsWith(redirect)) return null;
  const params = new URL(url).searchParams;
  if (params.get('state') !== state) throw new Error('Sign-in answered a different request; try again.');
  const code = params.get('code');
  if (!code) throw new Error(params.get('error_description') || params.get('error') || 'Sign-in returned no code.');
  return code;
}

/** Open the sign-in tab and wait for its code. */
export function waitForCode(tabs: Tabs, url: string, redirect: string, state: string): Promise<string> {
  return new Promise((resolve, reject) => {
    let tabId: number | undefined;
    const done = (finish: () => void) => {
      tabs.onUpdated.removeListener(onUpdated); tabs.onRemoved.removeListener(onRemoved); finish();
    };
    const onUpdated = (id: number, info: { url?: string }, tab?: { url?: string }) => {
      if (id !== tabId) return;
      try {
        const code = codeFrom(info.url || tab?.url, redirect, state);
        if (code === null) return;
        done(() => resolve(code));
      } catch (error) { done(() => reject(error)); }
      void tabs.remove(id);
    };
    const onRemoved = (id: number) => { if (id === tabId) done(() => reject(new Error('The sign-in tab was closed before signing in.'))); };
    tabs.onUpdated.addListener(onUpdated); tabs.onRemoved.addListener(onRemoved);
    tabs.create({ url }).then(tab => { tabId = tab.id; }, error => done(() => reject(error)));
  });
}

export async function signIn(server: string, call: RpcClient, tabs: Tabs = chrome.tabs, extensionId = chrome.runtime.id): Promise<any> {
  const { verifier, challenge } = await pkce();
  const state = random(16);
  const redirect = redirectUri(extensionId);
  const code = await waitForCode(tabs, authorizeUrl(server, challenge, state, redirect), redirect, state);
  return call('account.sign_in', { server_url: server, code, code_verifier: verifier, redirect_uri: redirect });
}
