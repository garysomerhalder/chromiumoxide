// Firefox and Safari give extensions no debugger: the relay speaks a small
// page protocol instead (see domain/pages.ts), on the same socket.

import { type Command, failure, type Params, reply } from '../domain/cdp';
import { hello } from '../domain/hello';
import { numberParam, pageMethod, stringParam } from '../domain/pages';
import { IDLE, type ConnectionStatus } from '../domain/status';
import { isAttachable } from '../domain/target';
import type { PagePort } from '../ports/page';
import type { SocketConnection, SocketPort } from '../ports/socket';
import type { Tab, TabsPort } from '../ports/tabs';

export interface PageRelayPorts {
  tabs: TabsPort;
  pages: PagePort;
  socket: SocketPort;
  userAgent: string;
}

/** Firefox's own pages (about:, moz-extension:) cannot be scripted. */
function scriptable(url: string) {
  return isAttachable(url) && !/^about:(?!blank)/.test(url);
}

const info = (t: Tab) => ({ url: t.url, title: t.title });

export class PageRelay {
  private connection: SocketConnection | null = null;
  private relayUrl = '';
  private listeners: ((s: ConnectionStatus) => void)[] = [];

  constructor(private readonly ports: PageRelayPorts) {}

  connect(url: string): void {
    if (this.connection?.isOpen() && this.relayUrl === url) return;
    this.connection?.close();
    this.relayUrl = url;
    const connection = this.ports.socket.open(url, {
      message: (text) => void this.handle(JSON.parse(text) as Command),
      close: () => {
        if (this.connection === connection) {
          this.connection = null;
          this.notify();
        }
      },
    });
    this.connection = connection;
    connection.send(JSON.stringify(hello('pages', this.ports.userAgent)));
    this.notify();
  }

  /** Whether the socket to `url` is open (or opening). */
  connectedTo(url: string): boolean {
    return this.relayUrl === url && !!this.connection?.isOpen();
  }

  async disconnect(): Promise<void> {
    const c = this.connection;
    this.connection = null;
    c?.close();
    this.notify();
  }

  status(): ConnectionStatus {
    return this.connection?.isOpen() ? { state: 'connected', relayUrl: this.relayUrl, tabs: [] } : IDLE;
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

  private async handle(cmd: Command) {
    let out;
    try {
      out = reply(cmd, await this.run(cmd.method, cmd.params ?? {}));
    } catch (e) {
      out = failure(cmd, e);
    }
    if (this.connection?.isOpen()) this.connection.send(JSON.stringify(out));
  }

  private async tab(method: string, p: Params): Promise<Tab> {
    const tabId = numberParam(method, p, 'tabId');
    const tab = await this.ports.tabs.get(tabId);
    if (!tab) throw new Error(`no tab ${tabId}`);
    return tab;
  }

  private async run(method: string, p: Params): Promise<unknown> {
    const { tabs, pages } = this.ports;
    switch (pageMethod(method)) {
      case 'Tabs.list':
        return {
          tabs: (await tabs.list()).filter((t) => scriptable(t.url)).map((t) => ({ tabId: t.id, ...info(t) })),
        };
      case 'Tabs.create': {
        const tab = await tabs.create('about:blank', true);
        const url = typeof p.url === 'string' ? p.url : '';
        if (url) await pages.navigate(tab.id, url);
        return { tabId: tab.id };
      }
      case 'Tab.info':
        return info(await this.tab(method, p));
      case 'Tab.goto': {
        const tab = await this.tab(method, p);
        return info(await pages.navigate(tab.id, stringParam(method, p, 'url')));
      }
      case 'Tab.evaluate': {
        const tab = await this.tab(method, p);
        return { value: await pages.evaluate(tab.id, stringParam(method, p, 'expression')) };
      }
      case 'Tab.click': {
        const tab = await this.tab(method, p);
        await pages.click(tab.id, stringParam(method, p, 'selector'));
        return {};
      }
      case 'Tab.fill': {
        const tab = await this.tab(method, p);
        await pages.fill(tab.id, stringParam(method, p, 'selector'), stringParam(method, p, 'value'));
        return {};
      }
      case 'Tab.screenshot': {
        const tab = await this.tab(method, p);
        return { data: await pages.screenshot(tab.id) };
      }
      case 'Tab.close': {
        const tab = await this.tab(method, p);
        await tabs.remove(tab.id);
        return {};
      }
    }
  }
}
