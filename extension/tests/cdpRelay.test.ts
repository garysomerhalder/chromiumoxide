import { beforeEach, describe, expect, it } from 'vitest';
import { CdpRelay } from '../src/application/cdpRelay';
import { FakeDebugger, FakeSocket, FakeTabs, FakeWindows } from './fakes';

const UA = 'Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.7727.15 Safari/537.36';

let tabs: FakeTabs;
let dbg: FakeDebugger;
let socket: FakeSocket;
let relay: CdpRelay;

beforeEach(() => {
  tabs = new FakeTabs();
  dbg = new FakeDebugger();
  socket = new FakeSocket();
  relay = new CdpRelay({ tabs, debugger: dbg, windows: new FakeWindows(), socket, userAgent: UA });
});

describe('the relay', () => {
  it('connects to the relay named by the connect page, and says which protocol it speaks', () => {
    relay.connect('ws://127.0.0.1:5/extension');
    expect(socket.url).toBe('ws://127.0.0.1:5/extension');
    expect(socket.received[0]).toEqual({ method: 'Extension.hello', params: { protocol: 'cdp', userAgent: UA } });
    expect(relay.status().state).toBe('connected');
  });

  it('announces every tab it can drive when the library auto-attaches', async () => {
    const page = tabs.add('https://a.test/', 'A');
    tabs.add('chrome://settings');
    relay.connect('ws://127.0.0.1:5/extension');
    const reply = await socket.command(1, 'Target.setAutoAttach', { autoAttach: true, flatten: true });
    expect(reply).toEqual({ id: 1, result: {} });
    expect([...dbg.attached]).toEqual([page.id]);
    const [attached] = socket.events('Target.attachedToTarget');
    expect(attached!.params).toMatchObject({
      sessionId: `tab-${page.id}`,
      waitingForDebugger: false,
      targetInfo: { targetId: `T${page.id}`, type: 'page', url: 'https://a.test/', title: 'A', browserContextId: '' },
    });
    expect(relay.status().tabs).toEqual([{ id: page.id, title: 'A', url: 'https://a.test/' }]);
  });

  it('sends tab commands to that tab, and child-session commands to the child', async () => {
    const page = tabs.add('https://a.test/');
    relay.connect('ws://127.0.0.1:5/extension');
    await socket.command(1, 'Target.setAutoAttach');
    dbg.replies.set('Runtime.evaluate', { result: { value: 42 } });
    const reply = await socket.command(2, 'Runtime.evaluate', { expression: '6*7' }, `tab-${page.id}`);
    expect(reply).toEqual({ id: 2, sessionId: `tab-${page.id}`, result: { result: { value: 42 } } });
    expect(dbg.sent[0]).toEqual({ target: { tabId: page.id }, method: 'Runtime.evaluate', params: { expression: '6*7' } });

    dbg.emit({ tabId: page.id }, 'Target.attachedToTarget', { sessionId: 'CHILD', targetInfo: { type: 'iframe' } });
    expect(socket.events('Target.attachedToTarget').at(-1)!.sessionId).toBe(`tab-${page.id}`);
    await socket.command(3, 'Page.enable', {}, 'CHILD');
    expect(dbg.sent.at(-1)!.target).toEqual({ tabId: page.id, sessionId: 'CHILD' });
    dbg.emit({ tabId: page.id, sessionId: 'CHILD' }, 'Page.loadEventFired', { timestamp: 1 });
    expect(socket.events('Page.loadEventFired')[0]).toEqual({
      method: 'Page.loadEventFired',
      params: { timestamp: 1 },
      sessionId: 'CHILD',
    });
  });

  it('opens new tabs in the browser\'s own profile only', async () => {
    relay.connect('ws://127.0.0.1:5/extension');
    await socket.command(1, 'Target.setAutoAttach');
    const reply = await socket.command(2, 'Target.createTarget', { url: 'about:blank' });
    const tab = [...tabs.tabs.values()].at(-1)!;
    expect(reply).toEqual({ id: 2, result: { targetId: `T${tab.id}` } });
    expect(dbg.attached.has(tab.id)).toBe(true);

    const isolated = await socket.command(3, 'Target.createTarget', { url: 'about:blank', browserContextId: 'X' });
    expect((isolated.error as { message: string }).message).toMatch(/own profile/);
    const ctx = await socket.command(4, 'Target.createBrowserContext');
    expect((ctx.error as { message: string }).message).toMatch(/contexts\(\)\[0\]/);
  });

  it('closes tabs by target id', async () => {
    const page = tabs.add('https://a.test/');
    relay.connect('ws://127.0.0.1:5/extension');
    await socket.command(1, 'Target.setAutoAttach');
    expect(await socket.command(2, 'Target.closeTarget', { targetId: `T${page.id}` })).toEqual({
      id: 2,
      result: { success: true },
    });
    expect(tabs.tabs.has(page.id)).toBe(false);
  });

  it('answers the browser version from the user agent', async () => {
    relay.connect('ws://127.0.0.1:5/extension');
    const reply = await socket.command(1, 'Browser.getVersion');
    expect((reply.result as { product: string }).product).toBe('Chrome/147.0.7727.15');
  });

  it('only disconnects on Browser.close: the browser and its tabs stay', async () => {
    const page = tabs.add('https://a.test/');
    relay.connect('ws://127.0.0.1:5/extension');
    await socket.command(1, 'Target.setAutoAttach');
    expect(await socket.command(2, 'Browser.close')).toEqual({ id: 2, result: {} });
    expect(dbg.attached.size).toBe(0);
    expect(tabs.tabs.has(page.id)).toBe(true);
  });

  it('reports tabs that go away, and refuses what it cannot do', async () => {
    const page = tabs.add('https://a.test/');
    relay.connect('ws://127.0.0.1:5/extension');
    await socket.command(1, 'Target.setAutoAttach');
    dbg.drop(page.id);
    expect(socket.events('Target.detachedFromTarget')[0]!.params).toEqual({
      sessionId: `tab-${page.id}`,
      targetId: `T${page.id}`,
    });
    const unknown = await socket.command(2, 'Tracing.start');
    expect((unknown.error as { message: string }).message).toBe('not available through the extension: Tracing.start');
  });

  it('drives tabs opened later, and lets go of everything when the library leaves', async () => {
    relay.connect('ws://127.0.0.1:5/extension');
    await socket.command(1, 'Target.setAutoAttach');
    const later = tabs.add('https://b.test/');
    await relay.tabChanged(later.id, later.url);
    expect(dbg.attached.has(later.id)).toBe(true);

    socket.close();
    await new Promise((r) => setTimeout(r, 5));
    expect(dbg.attached.size).toBe(0);
    expect(relay.status()).toEqual({ state: 'idle', tabs: [] });
  });

  it('disconnects on request', async () => {
    relay.connect('ws://127.0.0.1:5/extension');
    const states: string[] = [];
    relay.onStatus((s) => states.push(s.state));
    await relay.disconnect();
    expect(socket.isOpen()).toBe(false);
    expect(states.at(-1)).toBe('idle');
  });
});
