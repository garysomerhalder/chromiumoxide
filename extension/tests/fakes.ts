// In-memory port adapters for the application tests.
import type { DebuggerPort, DebugTarget } from '../src/ports/debugger';
import type { SocketConnection, SocketHandlers, SocketPort } from '../src/ports/socket';
import type { Tab, TabsPort } from '../src/ports/tabs';
import type { WindowBounds, WindowsPort } from '../src/ports/windows';

export class FakeTabs implements TabsPort {
  tabs = new Map<number, Tab>();
  private next = 100;
  add(url: string, title = '', openerTabId?: number): Tab {
    const tab: Tab = { id: this.next++, url, title, windowId: 1, openerTabId };
    this.tabs.set(tab.id, tab);
    return tab;
  }
  async list() { return [...this.tabs.values()]; }
  async get(id: number) { return this.tabs.get(id); }
  async create(url: string) { return this.add(url); }
  async remove(id: number) { this.tabs.delete(id); }
  async activate(id: number) { return this.tabs.get(id)!; }
}

export class FakeDebugger implements DebuggerPort {
  attached = new Set<number>();
  sent: { target: DebugTarget; method: string; params?: Record<string, unknown> }[] = [];
  replies = new Map<string, unknown>();
  private events: ((t: DebugTarget, method: string, params: Record<string, unknown>) => void)[] = [];
  private detaches: ((tabId: number) => void)[] = [];
  async attach(tabId: number) { this.attached.add(tabId); }
  async detach(tabId: number) { this.attached.delete(tabId); }
  async targetIdOf(tabId: number) { return `T${tabId}`; }
  async send(target: DebugTarget, method: string, params?: Record<string, unknown>) {
    this.sent.push({ target, method, params });
    return this.replies.get(method) ?? {};
  }
  onEvent(cb: (t: DebugTarget, method: string, params: Record<string, unknown>) => void) { this.events.push(cb); }
  onDetach(cb: (tabId: number) => void) { this.detaches.push(cb); }
  emit(target: DebugTarget, method: string, params: Record<string, unknown>) {
    this.events.forEach((cb) => cb(target, method, params));
  }
  drop(tabId: number) {
    this.attached.delete(tabId);
    this.detaches.forEach((cb) => cb(tabId));
  }
}

export class FakeWindows implements WindowsPort {
  bounds: WindowBounds = { left: 0, top: 0, width: 1200, height: 800, windowState: 'normal' };
  async get() { return this.bounds; }
  async update(_id: number, b: Partial<WindowBounds>) { this.bounds = { ...this.bounds, ...b }; }
  async focus() {}
}

/** The library's end of the relay socket. */
export class FakeSocket implements SocketPort, SocketConnection {
  url = '';
  open_ = false;
  received: Record<string, unknown>[] = [];
  private handlers: SocketHandlers | null = null;
  open(url: string, handlers: SocketHandlers): SocketConnection {
    this.url = url;
    this.open_ = true;
    this.handlers = handlers;
    return this;
  }
  send(text: string) { this.received.push(JSON.parse(text)); }
  close() {
    if (!this.open_) return;
    this.open_ = false;
    this.handlers?.close();
  }
  isOpen() { return this.open_; }
  /** The library sends a command; resolves with its reply. */
  async command(id: number, method: string, params: Record<string, unknown> = {}, sessionId?: string) {
    this.handlers!.message(JSON.stringify({ id, method, params, sessionId }));
    for (let i = 0; i < 50; i++) {
      const reply = this.received.find((m) => m.id === id);
      if (reply) return reply;
      await new Promise((r) => setTimeout(r, 1));
    }
    throw new Error(`no reply to ${method}`);
  }
  events(method: string) { return this.received.filter((m) => m.method === method); }
}

import type { PagePort } from '../src/ports/page';

/** A page per tab: scripts, navigation and pictures, in memory. */
export class FakePages implements PagePort {
  calls: string[] = [];
  values = new Map<string, unknown>();
  constructor(private tabs: FakeTabs) {}
  async navigate(tabId: number, url: string) {
    this.calls.push(`navigate ${tabId} ${url}`);
    const tab = this.tabs.tabs.get(tabId)!;
    tab.url = url;
    return tab;
  }
  async evaluate(tabId: number, expression: string) {
    this.calls.push(`evaluate ${tabId} ${expression}`);
    if (expression === 'boom') throw new Error('ReferenceError: boom is not defined');
    return this.values.get(expression);
  }
  async click(tabId: number, selector: string) {
    if (selector === '#missing') throw new Error('no element matches #missing');
    this.calls.push(`click ${tabId} ${selector}`);
  }
  async fill(tabId: number, selector: string, value: string) {
    this.calls.push(`fill ${tabId} ${selector} ${value}`);
  }
  async screenshot(tabId: number) {
    this.calls.push(`screenshot ${tabId}`);
    return 'iVBORw0KGgo=';
  }
}

import type { BrowserCookie, CookieSetting } from '../src/domain/cookies';
import type { CookiesPort } from '../src/ports/cookies';
import type { PermissionsPort } from '../src/ports/permissions';

export class FakeCookies implements CookiesPort {
  jar: BrowserCookie[] = [];
  async getAll() { return this.jar; }
  async set(c: CookieSetting) {
    this.jar = this.jar.filter((x) => x.name !== c.name);
    this.jar.push({
      name: c.name, value: c.value, domain: c.domain ?? new URL(c.url).hostname, path: c.path ?? '/',
      secure: !!c.secure, httpOnly: !!c.httpOnly, sameSite: c.sameSite ?? 'unspecified',
      session: c.expirationDate === undefined, expirationDate: c.expirationDate, hostOnly: !c.domain,
    });
  }
  async remove(_url: string, name: string) { this.jar = this.jar.filter((x) => x.name !== name); }
}

export class FakePermissions implements PermissionsPort {
  granted: string[] = [];
  async grant(settings: string[], origin?: string) {
    this.granted.push(...settings.map((s) => `${s}@${origin ?? '*'}`));
  }
  async reset() { this.granted = []; }
}

import type { DownloadItem, DownloadsPort } from '../src/ports/downloads';

export class FakeDownloads implements DownloadsPort {
  private created: ((d: DownloadItem) => void)[] = [];
  private changed: ((d: DownloadItem) => void)[] = [];
  onCreated(cb: (d: DownloadItem) => void) { this.created.push(cb); }
  onChanged(cb: (d: DownloadItem) => void) { this.changed.push(cb); }
  start(d: DownloadItem) { this.created.forEach((cb) => cb(d)); }
  update(d: DownloadItem) { this.changed.forEach((cb) => cb(d)); }
}
