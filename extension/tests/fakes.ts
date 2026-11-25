// In-memory port adapters for the application tests.
import type { DebuggerPort, DebugTarget } from '../src/ports/debugger';
import type { SocketConnection, SocketHandlers, SocketPort } from '../src/ports/socket';
import type { Tab, TabsPort } from '../src/ports/tabs';
import type { WindowBounds, WindowsPort } from '../src/ports/windows';

export class FakeTabs implements TabsPort {
  tabs = new Map<number, Tab>();
  private next = 100;
  add(url: string, title = ''): Tab {
    const tab = { id: this.next++, url, title, windowId: 1 };
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
