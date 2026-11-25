// Chrome and Edge: the library speaks Chrome's DevTools protocol to this
// relay as if it were a browser. Tab commands go to the tab's debugger;
// browser-wide commands (targets, windows, version), which extensions cannot
// send, are answered here. The browser's own profile is the library's
// default context (`browserContextId: ''`).

import { cookieUrl, type CookieParam, fromBrowserCookie, toBrowserCookie } from '../domain/cookies';
import { type Command, failure, type Outgoing, type Params, reply, Unavailable } from '../domain/cdp';
import { hello } from '../domain/hello';
import { type DownloadItem, progress, suggestedFilename } from '../domain/downloads';
import { contentSettingsFor } from '../domain/permissions';
import { IDLE, type ConnectionStatus, type TabSummary } from '../domain/status';
import { isAttachable, parseTabSession, tabSession, targetInfo } from '../domain/target';
import { browserVersion } from '../domain/version';
import type { CookiesPort } from '../ports/cookies';
import type { DebuggerPort, DebugTarget } from '../ports/debugger';
import type { DownloadsPort } from '../ports/downloads';
import type { PermissionsPort } from '../ports/permissions';
import type { SocketConnection, SocketPort } from '../ports/socket';
import type { TabsPort } from '../ports/tabs';
import type { WindowBounds, WindowsPort } from '../ports/windows';

export interface CdpRelayPorts {
  tabs: TabsPort;
  debugger: DebuggerPort;
  windows: WindowsPort;
  socket: SocketPort;
  userAgent: string;
  cookies: CookiesPort;
  permissions: PermissionsPort;
  downloads: DownloadsPort;
}

interface Driven {
  targetId: string;
  title: string;
  url: string;
}

export class CdpRelay {
  private connection: SocketConnection | null = null;
  private relayUrl = '';
  private autoAttach = false;
  private driven = new Map<number, Driven>();
  private attaching = new Map<number, Promise<string | null>>();
  /** Child sessions (iframes, workers) -> their tab. */
  private children = new Map<string, number>();
  private listeners: ((s: ConnectionStatus) => void)[] = [];
  /** `Browser.setDownloadBehavior` asked for download events. */
  private downloadEvents = false;
  /** Recent requests' frames by URL, to tell which page started a download. */
  private requestFrames = new Map<string, string>();

  constructor(private readonly ports: CdpRelayPorts) {
    ports.debugger.onEvent((target, method, params) => this.onDebuggerEvent(target, method, params));
    ports.debugger.onDetach((tabId) => this.onDetached(tabId));
    ports.downloads.onCreated((item) => this.onDownload(item));
    ports.downloads.onChanged((item) => {
      const p = this.downloadEvents ? progress(item) : null;
      if (p) this.send({ method: 'Browser.downloadProgress', params: p });
    });
  }

  private onDownload(item: DownloadItem) {
    if (!this.downloadEvents) return;
    let frameId = this.requestFrames.get(item.url);
    if (frameId === undefined) {
      // The page the download came from: its main frame (id = target id).
      for (const d of this.driven.values()) if (d.url === item.referrer) frameId = d.targetId;
    }
    this.send({
      method: 'Browser.downloadWillBegin',
      params: { frameId: frameId ?? '', guid: String(item.id), url: item.url, suggestedFilename: suggestedFilename(item) },
    });
  }

  /** Connects to the relay at `url` (from the connect page); once per relay. */
  connect(url: string): void {
    if (this.connection?.isOpen() && this.relayUrl === url) return;
    this.connection?.close();
    this.relayUrl = url;
    const connection = this.ports.socket.open(url, {
      message: (text) => void this.handle(JSON.parse(text) as Command),
      close: () => {
        if (this.connection === connection) {
          this.connection = null;
          void this.release();
        }
      },
    });
    this.connection = connection;
    connection.send(JSON.stringify(hello('cdp', this.ports.userAgent)));
    this.notify();
  }

  /** Whether the socket to `url` is open (or opening). */
  connectedTo(url: string): boolean {
    return this.relayUrl === url && !!this.connection?.isOpen();
  }

  async disconnect(): Promise<void> {
    const connection = this.connection;
    this.connection = null;
    connection?.close();
    await this.release();
  }

  /** A tab was opened or changed: drive it if the library asked for every tab. */
  async tabChanged(tabId: number, url: string | undefined, title?: string): Promise<void> {
    const known = this.driven.get(tabId);
    if (known) {
      if (url !== undefined) known.url = url;
      if (title !== undefined) known.title = title;
      this.notify();
      return;
    }
    if (this.autoAttach && isAttachable(url)) await this.attach(tabId);
  }

  status(): ConnectionStatus {
    if (!this.connection?.isOpen()) return IDLE;
    const tabs: TabSummary[] = [...this.driven].map(([id, d]) => ({ id, title: d.title, url: d.url }));
    return { state: 'connected', relayUrl: this.relayUrl, tabs };
  }

  onStatus(listener: (s: ConnectionStatus) => void): () => void {
    this.listeners.push(listener);
    return () => {
      this.listeners = this.listeners.filter((l) => l !== listener);
    };
  }

  private notify() {
    const s = this.status();
    this.listeners.forEach((l) => l(s));
  }

  private send(m: Outgoing) {
    if (this.connection?.isOpen()) this.connection.send(JSON.stringify(m));
  }

  /** Lets go of every tab (the tabs stay open). */
  private async release() {
    this.autoAttach = false;
    this.downloadEvents = false;
    const tabs = [...this.driven.keys()];
    this.driven.clear();
    this.children.clear();
    await Promise.all(tabs.map((id) => this.ports.debugger.detach(id).catch(() => {})));
    this.notify();
  }

  private attach(tabId: number): Promise<string | null> {
    const known = this.driven.get(tabId);
    if (known) return Promise.resolve(known.targetId);
    const pending = this.attaching.get(tabId);
    if (pending) return pending;
    const run = (async () => {
      try {
        await this.ports.debugger.attach(tabId);
      } catch (e) {
        if (!/already attached/i.test(String(e))) return null;
      }
      const targetId = (await this.ports.debugger.targetIdOf(tabId)) ?? tabSession(tabId);
      const tab = await this.ports.tabs.get(tabId);
      this.driven.set(tabId, { targetId, title: tab?.title ?? '', url: tab?.url ?? '' });
      const opener = tab?.openerTabId === undefined ? undefined : this.driven.get(tab.openerTabId)?.targetId;
      this.send({
        method: 'Target.attachedToTarget',
        params: {
          sessionId: tabSession(tabId),
          targetInfo: targetInfo(targetId, tab, opener),
          waitingForDebugger: false,
        },
      });
      this.notify();
      return targetId;
    })().finally(() => this.attaching.delete(tabId));
    this.attaching.set(tabId, run);
    return run;
  }

  private onDebuggerEvent(target: DebugTarget, method: string, params: Params) {
    if (!this.driven.has(target.tabId)) return;
    if (method === 'Target.attachedToTarget') this.children.set(params.sessionId as string, target.tabId);
    if (method === 'Target.detachedFromTarget') this.children.delete(params.sessionId as string);
    if (method === 'Network.requestWillBeSent') {
      const url = (params.request as { url?: string } | undefined)?.url;
      if (url && typeof params.frameId === 'string') {
        this.requestFrames.set(url, params.frameId);
        if (this.requestFrames.size > 500) this.requestFrames.delete(this.requestFrames.keys().next().value!);
      }
    }
    this.send({ method, params, sessionId: target.sessionId ?? tabSession(target.tabId) });
  }

  private onDetached(tabId: number) {
    const d = this.driven.get(tabId);
    if (!d) return;
    this.driven.delete(tabId);
    for (const [child, tab] of this.children) if (tab === tabId) this.children.delete(child);
    this.send({ method: 'Target.detachedFromTarget', params: { sessionId: tabSession(tabId), targetId: d.targetId } });
    this.notify();
  }

  private async handle(cmd: Command) {
    try {
      const result =
        cmd.sessionId === undefined
          ? await this.browserCommand(cmd.method, cmd.params ?? {})
          : await this.tabCommand(cmd.sessionId, cmd.method, cmd.params);
      this.send(reply(cmd, result));
    } catch (e) {
      this.send(failure(cmd, e));
    }
  }

  private tabCommand(sessionId: string, method: string, params?: Params) {
    const own = parseTabSession(sessionId);
    const tabId = own ?? this.children.get(sessionId);
    if (tabId === undefined) throw new Error(`Session with given id not found: ${sessionId}`);
    const target: DebugTarget = own !== null ? { tabId } : { tabId, sessionId };
    return this.ports.debugger.send(target, method, params);
  }

  private tabOf(targetId: unknown): number {
    for (const [tabId, d] of this.driven) if (d.targetId === targetId) return tabId;
    throw new Error(`No target with given id found: ${String(targetId)}`);
  }

  /** The browser-wide commands extensions cannot send. */
  private async browserCommand(method: string, p: Params): Promise<unknown> {
    const { tabs, windows } = this.ports;
    switch (method) {
      case 'Browser.getVersion':
        return browserVersion(this.ports.userAgent);
      case 'Target.setAutoAttach': {
        this.autoAttach = true;
        const all = await tabs.list();
        await Promise.all(all.filter((t) => isAttachable(t.url)).map((t) => this.attach(t.id)));
        return {};
      }
      case 'Target.setDiscoverTargets':
      case 'Browser.cancelDownload':
        return {};
      case 'Browser.setDownloadBehavior':
        // Files stay where the browser saves them; events say where.
        this.downloadEvents = p.eventsEnabled === true;
        return {};
      case 'Storage.getCookies':
        return { cookies: (await this.ports.cookies.getAll()).map(fromBrowserCookie) };
      case 'Storage.setCookies':
        for (const c of (p.cookies ?? []) as CookieParam[]) await this.ports.cookies.set(toBrowserCookie(c));
        return {};
      case 'Storage.clearCookies':
        for (const c of await this.ports.cookies.getAll()) await this.ports.cookies.remove(cookieUrl(c), c.name);
        return {};
      case 'Browser.grantPermissions':
        await this.ports.permissions.grant(
          contentSettingsFor((p.permissions ?? []) as string[]),
          typeof p.origin === 'string' ? p.origin : undefined,
        );
        return {};
      case 'Browser.resetPermissions':
        await this.ports.permissions.reset();
        return {};
      case 'Target.getTargets': {
        const infos = await Promise.all(
          [...this.driven].map(async ([id, d]) => targetInfo(d.targetId, await tabs.get(id))),
        );
        return { targetInfos: infos };
      }
      case 'Target.getTargetInfo': {
        const id = this.tabOf(p.targetId);
        return { targetInfo: targetInfo(this.driven.get(id)!.targetId, await tabs.get(id)) };
      }
      case 'Target.createTarget': {
        if (p.browserContextId) {
          throw new Unavailable("the extension drives the browser's own profile only (contexts()[0])");
        }
        const tab = await tabs.create((p.url as string) || 'about:blank', !p.background);
        const targetId = await this.attach(tab.id);
        if (!targetId) throw new Error('cannot attach to the new tab');
        return { targetId };
      }
      case 'Target.attachToTarget':
        return { sessionId: tabSession(this.tabOf(p.targetId)) };
      case 'Target.closeTarget':
        await tabs.remove(this.tabOf(p.targetId));
        return { success: true };
      case 'Target.activateTarget': {
        const tab = await tabs.activate(this.tabOf(p.targetId));
        await windows.focus(tab.windowId);
        return {};
      }
      case 'Browser.getWindowForTarget': {
        const id = p.targetId ? this.tabOf(p.targetId) : [...this.driven.keys()][0];
        const tab = id === undefined ? undefined : await tabs.get(id);
        if (!tab) throw new Error('no window');
        return { windowId: tab.windowId, bounds: await windows.get(tab.windowId) };
      }
      case 'Browser.setWindowBounds':
        await windows.update(p.windowId as number, (p.bounds ?? {}) as Partial<WindowBounds>);
        return {};
      case 'Browser.close':
        // Disconnects; never closes the browser someone is using.
        await this.release();
        setTimeout(() => void this.disconnect(), 50);
        return {};
      case 'Target.createBrowserContext':
      case 'Target.disposeBrowserContext':
        throw new Unavailable(
          "the extension drives the browser's own profile; use browser.contexts()[0] instead of a new context",
        );
      default:
        throw new Unavailable(`not available through the extension: ${method}`);
    }
  }
}
