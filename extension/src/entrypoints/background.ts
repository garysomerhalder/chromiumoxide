// Composition root: wires the relay to this browser's APIs.
//
// The library listens on a local port (`Chromium::extension_relay`). A tab
// opening its page, http://127.0.0.1:<port>/chromiumoxide-connect, tells the
// extension where to connect.
import { BrowserPages } from '../adapters/browserPages';
import { BrowserTabs, BrowserWindows } from '../adapters/browserTabs';
import { ChromeDebugger, hasDebugger } from '../adapters/chromeDebugger';
import { ChromeCookies, ChromeDownloads, ChromePermissions } from '../adapters/chromeSettings';
import { serveStatus } from '../adapters/runtimeStatus';
import { BrowserWebSocket } from '../adapters/webSocket';
import { CdpRelay } from '../application/cdpRelay';
import { PageRelay } from '../application/pageRelay';
import { parseConnectUrl } from '../domain/connect';

export default defineBackground(() => {
  const tabs = new BrowserTabs();
  const socket = new BrowserWebSocket();
  const userAgent = navigator.userAgent;
  // Chrome and Edge: the whole DevTools protocol. Firefox and Safari (no
  // debugger for extensions): the page protocol.
  const cdp = hasDebugger()
    ? new CdpRelay({
        tabs,
        debugger: new ChromeDebugger(),
        windows: new BrowserWindows(),
        socket,
        userAgent,
        cookies: new ChromeCookies(),
        permissions: new ChromePermissions(),
        downloads: new ChromeDownloads(),
      })
    : null;
  const relay = cdp ?? new PageRelay({ tabs, pages: new BrowserPages(), socket, userAgent });
  serveStatus(relay);

  /** Connects if `url` is a connect page; whether connected to its relay. */
  const consider = (url: string | undefined): boolean => {
    const ws = parseConnectUrl(url);
    if (!ws) return false;
    relay.connect(ws);
    return relay.connectedTo(ws);
  };
  browser.tabs.onUpdated.addListener((tabId, info, tab) => {
    consider(info.url ?? tab.url);
    void cdp?.tabChanged(tabId, tab.url, tab.title);
  });
  browser.tabs.onCreated.addListener((tab) => {
    consider(tab.url ?? tab.pendingUrl);
    if (tab.id !== undefined) void cdp?.tabChanged(tab.id, tab.url ?? tab.pendingUrl, tab.title);
  });
  // From the connect page's script (wakes a sleeping background).
  browser.runtime.onMessage.addListener((m: unknown) => {
    const msg = m as { type?: string; url?: string };
    if (msg?.type === 'connect') return Promise.resolve(consider(msg.url));
    return undefined;
  });
  // A connect page already open when the extension starts.
  void browser.tabs.query({}).then((tabs) => tabs.forEach((t) => consider(t.url ?? t.pendingUrl)));
});
