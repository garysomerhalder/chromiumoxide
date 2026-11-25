import type { Tab, TabsPort } from '../ports/tabs';
import type { WindowBounds, WindowsPort } from '../ports/windows';

type RawTab = {
  id?: number;
  url?: string;
  pendingUrl?: string;
  title?: string;
  windowId?: number;
  openerTabId?: number;
};

function tab(t: RawTab): Tab {
  return {
    id: t.id ?? -1,
    url: t.url || t.pendingUrl || '',
    title: t.title ?? '',
    windowId: t.windowId ?? -1,
    openerTabId: t.openerTabId,
  };
}

/** `browser.tabs` (every browser). */
export class BrowserTabs implements TabsPort {
  async list() {
    return (await browser.tabs.query({})).filter((t) => t.id !== undefined).map(tab);
  }
  async get(id: number) {
    return browser.tabs.get(id).then(tab, () => undefined);
  }
  async create(url: string, active: boolean) {
    return tab(await browser.tabs.create({ url, active }));
  }
  async remove(id: number) {
    await browser.tabs.remove(id);
  }
  async activate(id: number) {
    return tab((await browser.tabs.update(id, { active: true }))!);
  }
}

/** `browser.windows`. */
export class BrowserWindows implements WindowsPort {
  async get(id: number): Promise<WindowBounds> {
    const w = await browser.windows.get(id);
    return { left: w.left ?? 0, top: w.top ?? 0, width: w.width ?? 0, height: w.height ?? 0, windowState: w.state ?? 'normal' };
  }
  async update(id: number, b: Partial<WindowBounds>) {
    const info: Parameters<typeof browser.windows.update>[1] = {};
    if (b.windowState) info.state = b.windowState as typeof info.state;
    if (b.left !== undefined) info.left = b.left;
    if (b.top !== undefined) info.top = b.top;
    if (b.width !== undefined) info.width = b.width;
    if (b.height !== undefined) info.height = b.height;
    await browser.windows.update(id, info);
  }
  async focus(id: number) {
    await browser.windows.update(id, { focused: true });
  }
}
