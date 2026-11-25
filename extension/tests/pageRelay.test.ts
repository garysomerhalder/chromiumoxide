import { beforeEach, describe, expect, it } from 'vitest';
import { PageRelay } from '../src/application/pageRelay';
import { FakePages, FakeSocket, FakeTabs } from './fakes';

let tabs: FakeTabs;
let pages: FakePages;
let socket: FakeSocket;
let relay: PageRelay;

beforeEach(() => {
  tabs = new FakeTabs();
  pages = new FakePages(tabs);
  socket = new FakeSocket();
  relay = new PageRelay({ tabs, pages, socket, userAgent: 'Mozilla/5.0 Firefox/159.0' });
});

describe('the page relay (Firefox, Safari)', () => {
  it('says hello with its protocol when it connects', () => {
    relay.connect('ws://127.0.0.1:5/extension');
    expect(socket.received[0]).toEqual({
      method: 'Extension.hello',
      params: { protocol: 'pages', userAgent: 'Mozilla/5.0 Firefox/159.0' },
    });
    expect(relay.status().state).toBe('connected');
  });

  it('lists, opens and closes tabs', async () => {
    const a = tabs.add('https://a.test/', 'A');
    tabs.add('about:debugging');
    relay.connect('ws://127.0.0.1:5/extension');
    expect(await socket.command(1, 'Tabs.list')).toEqual({
      id: 1,
      result: { tabs: [{ tabId: a.id, url: 'https://a.test/', title: 'A' }] },
    });
    const created = await socket.command(2, 'Tabs.create', { url: 'https://b.test/' });
    const b = [...tabs.tabs.values()].at(-1)!;
    expect(created).toEqual({ id: 2, result: { tabId: b.id } });
    expect(await socket.command(3, 'Tab.close', { tabId: b.id })).toEqual({ id: 3, result: {} });
    expect(tabs.tabs.has(b.id)).toBe(false);
  });

  it('navigates, runs scripts, clicks, fills and takes pictures', async () => {
    const a = tabs.add('https://a.test/', 'A');
    relay.connect('ws://127.0.0.1:5/extension');
    expect(await socket.command(1, 'Tab.goto', { tabId: a.id, url: 'https://c.test/' })).toEqual({
      id: 1,
      result: { url: 'https://c.test/', title: 'A' },
    });
    pages.values.set('6 * 7', 42);
    expect(await socket.command(2, 'Tab.evaluate', { tabId: a.id, expression: '6 * 7' })).toEqual({
      id: 2,
      result: { value: 42 },
    });
    await socket.command(3, 'Tab.click', { tabId: a.id, selector: 'button' });
    await socket.command(4, 'Tab.fill', { tabId: a.id, selector: 'input', value: 'hi' });
    expect(await socket.command(5, 'Tab.screenshot', { tabId: a.id })).toEqual({
      id: 5,
      result: { data: 'iVBORw0KGgo=' },
    });
    expect(pages.calls).toEqual([
      `navigate ${a.id} https://c.test/`,
      `evaluate ${a.id} 6 * 7`,
      `click ${a.id} button`,
      `fill ${a.id} input hi`,
      `screenshot ${a.id}`,
    ]);
    expect(await socket.command(6, 'Tab.info', { tabId: a.id })).toEqual({
      id: 6,
      result: { url: 'https://c.test/', title: 'A' },
    });
  });

  it('reports failures and refuses bad requests', async () => {
    const a = tabs.add('https://a.test/');
    relay.connect('ws://127.0.0.1:5/extension');
    const thrown = await socket.command(1, 'Tab.evaluate', { tabId: a.id, expression: 'boom' });
    expect((thrown.error as { message: string }).message).toBe('ReferenceError: boom is not defined');
    const missing = await socket.command(2, 'Tab.click', { tabId: a.id, selector: '#missing' });
    expect((missing.error as { message: string }).message).toBe('no element matches #missing');
    const noTab = await socket.command(3, 'Tab.goto', { url: 'https://x/' });
    expect((noTab.error as { message: string }).message).toBe('Tab.goto needs a number tabId');
    const unknown = await socket.command(4, 'Runtime.evaluate');
    expect((unknown.error as { message: string }).message).toMatch(
      /not available through the extension in this browser: Runtime\.evaluate/,
    );
  });

  it('lets go when the library leaves', async () => {
    relay.connect('ws://127.0.0.1:5/extension');
    socket.close();
    expect(relay.status()).toEqual({ state: 'idle', tabs: [] });
  });
});
