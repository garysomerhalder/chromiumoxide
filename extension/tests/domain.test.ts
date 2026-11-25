import { describe, expect, it } from 'vitest';
import { parseConnectUrl } from '../src/domain/connect';
import { isAttachable, parseTabSession, tabSession } from '../src/domain/target';
import { browserVersion } from '../src/domain/version';

describe('the connect page', () => {
  it('names the relay the extension connects to', () => {
    expect(parseConnectUrl('http://127.0.0.1:41234/chromiumoxide-connect')).toBe(
      'ws://127.0.0.1:41234/extension',
    );
    expect(parseConnectUrl('http://localhost:9/chromiumoxide-connect?x=1')).toBe(
      'ws://localhost:9/extension',
    );
    expect(parseConnectUrl('http://[::1]:80/chromiumoxide-connect')).toBe('ws://[::1]:80/extension');
  });

  it('ignores every other page, and relays that are not on this computer', () => {
    expect(parseConnectUrl('https://example.com/chromiumoxide-connect')).toBeNull();
    expect(parseConnectUrl('http://10.0.0.2:80/chromiumoxide-connect')).toBeNull();
    expect(parseConnectUrl('http://127.0.0.1:80/other')).toBeNull();
    expect(parseConnectUrl(undefined)).toBeNull();
  });
});

describe('tabs', () => {
  it('can be driven unless they are browser or store pages', () => {
    for (const url of ['https://a.test/', 'http://x/', 'about:blank', 'file:///tmp/a.html', 'data:text/html,x', '']) {
      expect(isAttachable(url), url).toBe(true);
    }
    for (const url of [
      'chrome://settings',
      'edge://settings',
      'chrome-extension://abc/popup.html',
      'devtools://devtools/x',
      'https://chromewebstore.google.com/detail/x',
      'https://chrome.google.com/webstore/x',
    ]) {
      expect(isAttachable(url), url).toBe(false);
    }
  });

  it('have one session id each', () => {
    expect(tabSession(7)).toBe('tab-7');
    expect(parseTabSession('tab-7')).toBe(7);
    expect(parseTabSession('A1B2')).toBeNull();
  });
});

describe('the browser version', () => {
  it('is read from the user agent, for Chrome and Edge', () => {
    const chrome =
      'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.7727.15 Safari/537.36';
    expect(browserVersion(chrome)).toEqual({
      protocolVersion: '1.3',
      product: 'Chrome/147.0.7727.15',
      userAgent: chrome,
      jsVersion: '',
      revision: '',
    });
    const edge = `${chrome} Edg/147.0.3000.1`;
    expect(browserVersion(edge).product).toBe('Chrome/147.0.7727.15');
  });
});
