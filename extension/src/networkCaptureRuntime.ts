// The extension's one NetworkCapture: it shares this extension's debugger attachment with the
// frame router (see FrameRouter.holdAttachment) and never issues a request.
import { frameRouter } from './cdp/frameRouter';
import { NetworkCapture } from './networkCapture';

export const networkCapture = new NetworkCapture({
  attach: (tabId) => chrome.debugger.attach({ tabId }, '1.3'),
  detach: (tabId) => chrome.debugger.detach({ tabId }),
  send: (tabId, method, params) => chrome.debugger.sendCommand({ tabId }, method, params),
  hold: (tabId) => frameRouter.holdAttachment(tabId),
  release: (tabId) => frameRouter.releaseAttachment(tabId),
});

if (typeof chrome !== 'undefined' && chrome.debugger?.onEvent?.addListener) {
  chrome.debugger.onEvent.addListener((source, method, params) => networkCapture.onEvent(source as any, method, params));
  chrome.debugger.onDetach.addListener((source, reason) => networkCapture.onDetach(source?.tabId, String(reason || '')));
}
