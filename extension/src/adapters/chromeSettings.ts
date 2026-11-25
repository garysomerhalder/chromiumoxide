/// <reference types="chrome" />
import type { BrowserCookie, CookieSetting } from '../domain/cookies';
import type { CookiesPort } from '../ports/cookies';
import type { PermissionsPort } from '../ports/permissions';

/** `chrome.cookies`: the profile's cookies. */
export class ChromeCookies implements CookiesPort {
  async getAll(): Promise<BrowserCookie[]> {
    return (await chrome.cookies.getAll({})) as BrowserCookie[];
  }
  async set(cookie: CookieSetting) {
    const done = await chrome.cookies.set(cookie as chrome.cookies.SetDetails);
    if (!done) throw new Error(`the browser refused cookie ${cookie.name}`);
  }
  async remove(url: string, name: string) {
    await chrome.cookies.remove({ url, name });
  }
}

type Setting = { set(d: object): Promise<void>; clear(d: object): Promise<void> };

/** `chrome.contentSettings`: site permissions. */
export class ChromePermissions implements PermissionsPort {
  private touched = new Set<string>();
  private setting(name: string): Setting {
    const s = (chrome.contentSettings as unknown as Record<string, Setting | undefined>)[name];
    if (!s) throw new Error(`this browser has no ${name} setting`);
    return s;
  }
  async grant(settings: string[], origin?: string) {
    const primaryPattern = origin ? `${origin.replace(/\/$/, '')}/*` : '<all_urls>';
    for (const name of settings) {
      await this.setting(name).set({ primaryPattern, setting: 'allow' });
      this.touched.add(name);
    }
  }
  async reset() {
    for (const name of this.touched) await this.setting(name).clear({});
    this.touched.clear();
  }
}

import type { DownloadItem } from '../domain/downloads';
import type { DownloadsPort } from '../ports/downloads';

function item(d: chrome.downloads.DownloadItem): DownloadItem {
  return {
    id: d.id,
    url: d.finalUrl || d.url,
    referrer: d.referrer ?? '',
    filename: d.filename ?? '',
    state: d.state as DownloadItem['state'],
    totalBytes: d.totalBytes,
    bytesReceived: d.bytesReceived,
  };
}

/** `chrome.downloads`. */
export class ChromeDownloads implements DownloadsPort {
  onCreated(listener: (item: DownloadItem) => void) {
    chrome.downloads.onCreated.addListener((d) => listener(item(d)));
  }
  onChanged(listener: (item: DownloadItem) => void) {
    chrome.downloads.onChanged.addListener(async (delta) => {
      if (!delta.state) return;
      const [d] = await chrome.downloads.search({ id: delta.id });
      if (d) listener(item(d));
    });
  }
}
