// Composition root: wires the relay to this browser's APIs.
//
// The library listens on a local port (`Chromium::extension_relay`). A tab
// opening its page, http://127.0.0.1:<port>/chromiumoxide-connect, tells the
// extension where to connect.
import { BrowserTabs, BrowserWindows } from '../adapters/browserTabs';
import { ChromeDebugger, hasDebugger } from '../adapters/chromeDebugger';
import { serveStatus } from '../adapters/runtimeStatus';
import { BrowserWebSocket } from '../adapters/webSocket';
import { CdpRelay } from '../application/cdpRelay';
import { parseConnectUrl } from '../domain/connect';

export default defineBackground(() => {
  if (!hasDebugger()) return; // Firefox and Safari: next step.
  const relay = new CdpRelay({
    tabs: new BrowserTabs(),
    debugger: new ChromeDebugger(),
    windows: new BrowserWindows(),
    socket: new BrowserWebSocket(),
    userAgent: navigator.userAgent,
  });
  serveStatus(relay);

  const consider = (url: string | undefined) => {
    const ws = parseConnectUrl(url);
    if (ws) relay.connect(ws);
  };
  browser.tabs.onUpdated.addListener((tabId, info, tab) => {
    consider(info.url ?? tab.url);
    void relay.tabChanged(tabId, tab.url, tab.title);
  });
  browser.tabs.onCreated.addListener((tab) => {
    consider(tab.url ?? tab.pendingUrl);
    if (tab.id !== undefined) void relay.tabChanged(tab.id, tab.url ?? tab.pendingUrl, tab.title);
  });
  // From the connect page's script (wakes a sleeping background).
  browser.runtime.onMessage.addListener((m: unknown) => {
    const msg = m as { type?: string; url?: string };
    if (msg?.type === 'connect') consider(msg.url);
  });
  // A connect page already open when the extension starts.
  void browser.tabs.query({}).then((tabs) => tabs.forEach((t) => consider(t.url)));
});
