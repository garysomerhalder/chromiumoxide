import { beforeEach, describe, expect, it } from 'vitest';
import { CdpRelay } from '../src/application/cdpRelay';
import { FakeCookies, FakeDebugger, FakeDownloads, FakePermissions, FakeSocket, FakeTabs, FakeWindows } from './fakes';

const UA = 'Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.7727.15 Safari/537.36';

let tabs: FakeTabs;
let dbg: FakeDebugger;
let socket: FakeSocket;
let relay: CdpRelay;
let cookies: FakeCookies;
let permissions: FakePermissions;
let downloads: FakeDownloads;

beforeEach(() => {
  tabs = new FakeTabs();
  dbg = new FakeDebugger();
  socket = new FakeSocket();
  cookies = new FakeCookies();
  permissions = new FakePermissions();
  downloads = new FakeDownloads();
  relay = new CdpRelay({
    tabs, debugger: dbg, windows: new FakeWindows(), socket, userAgent: UA, cookies, permissions, downloads,
  });
});

describe('the relay', () => {
  it('connects to the relay named by the connect page, and says which protocol it speaks', () => {
    relay.connect('ws://127.0.0.1:5/extension');
    expect(socket.url).toBe('ws://127.0.0.1:5/extension');
    expect(socket.received[0]).toEqual({ method: 'Extension.hello', params: { protocol: 'cdp', userAgent: UA } });
    expect(relay.status().state).toBe('connected');
    expect(relay.connectedTo('ws://127.0.0.1:5/extension')).toBe(true);
    expect(relay.connectedTo('ws://127.0.0.1:6/extension')).toBe(false);
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

  it('names the opener of a tab a driven page opened (popups)', async () => {
    const page = tabs.add('https://a.test/');
    relay.connect('ws://127.0.0.1:5/extension');
    await socket.command(1, 'Target.setAutoAttach');
    const popup = tabs.add('https://pop.test/', 'Pop', page.id);
    await relay.tabChanged(popup.id, popup.url);
    const announced = socket.events('Target.attachedToTarget').at(-1)!;
    expect((announced.params as { targetInfo: { openerId?: string } }).targetInfo.openerId).toBe(`T${page.id}`);
    const first = socket.events('Target.attachedToTarget')[0]!;
    expect((first.params as { targetInfo: object }).targetInfo).not.toHaveProperty('openerId');
  });

  it('reads, writes and clears the profile\'s cookies', async () => {
    relay.connect('ws://127.0.0.1:5/extension');
    await socket.command(1, 'Storage.setCookies', { cookies: [{ name: 'k', value: 'v', url: 'http://a.test/' }] });
    const got = await socket.command(2, 'Storage.getCookies');
    expect((got.result as { cookies: { name: string; domain: string }[] }).cookies).toMatchObject([
      { name: 'k', value: 'v', domain: 'a.test' },
    ]);
    await socket.command(3, 'Storage.clearCookies');
    expect(cookies.jar).toEqual([]);
  });

  it('grants and resets permissions as content settings', async () => {
    relay.connect('ws://127.0.0.1:5/extension');
    await socket.command(1, 'Browser.grantPermissions', { permissions: ['geolocation'], origin: 'http://a.test:8' });
    expect(permissions.granted).toEqual(['location@http://a.test:8']);
    await socket.command(2, 'Browser.resetPermissions');
    expect(permissions.granted).toEqual([]);
    const refused = await socket.command(3, 'Browser.grantPermissions', { permissions: ['midi'] });
    expect((refused.error as { message: string }).message).toMatch(/cannot be granted through the extension: midi/);
  });

  it('reports downloads, from the tab whose request started them, once asked to', async () => {
    const page = tabs.add('https://a.test/');
    relay.connect('ws://127.0.0.1:5/extension');
    await socket.command(1, 'Target.setAutoAttach');
    downloads.start({ id: 1, url: 'https://a.test/early', referrer: '', filename: '', state: 'in_progress' });
    expect(socket.events('Browser.downloadWillBegin')).toEqual([]);

    await socket.command(2, 'Browser.setDownloadBehavior', { behavior: 'allowAndName', eventsEnabled: true });
    dbg.emit({ tabId: page.id }, 'Network.requestWillBeSent', {
      frameId: 'F1', request: { url: 'https://a.test/file.zip' },
    });
    downloads.start({ id: 7, url: 'https://a.test/file.zip', referrer: 'https://a.test/', filename: '', state: 'in_progress' });
    expect(socket.events('Browser.downloadWillBegin')[0]!.params).toEqual({
      frameId: 'F1', guid: '7', url: 'https://a.test/file.zip', suggestedFilename: 'file.zip',
    });
    downloads.update({
      id: 7, url: 'https://a.test/file.zip', referrer: '', filename: '/home/g/Downloads/file.zip',
      state: 'complete', totalBytes: 3, bytesReceived: 3,
    });
    expect(socket.events('Browser.downloadProgress')[0]!.params).toEqual({
      guid: '7', state: 'completed', totalBytes: 3, receivedBytes: 3, filePath: '/home/g/Downloads/file.zip',
    });

    // Without a matching request: the tab the download's page is open in.
    downloads.start({ id: 8, url: 'https://cdn.test/x.pdf', referrer: 'https://a.test/', filename: '', state: 'in_progress' });
    expect(socket.events('Browser.downloadWillBegin')[1]!.params).toMatchObject({ frameId: `T${page.id}`, guid: '8' });
    downloads.update({ id: 8, url: 'https://cdn.test/x.pdf', referrer: '', filename: '', state: 'interrupted' });
    expect(socket.events('Browser.downloadProgress')[1]!.params).toMatchObject({ guid: '8', state: 'canceled' });
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
