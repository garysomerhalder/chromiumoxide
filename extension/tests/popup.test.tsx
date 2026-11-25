// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { ConnectionStatus } from '../src/domain/status';
import type { StatusClient } from '../src/ports/status';
import { StatusPage } from '../src/ui/pages/StatusPage';

class FakeClient implements StatusClient {
  disconnected = 0;
  private listener: ((s: ConnectionStatus) => void) | null = null;
  constructor(private current: ConnectionStatus) {}
  async status() { return this.current; }
  subscribe(listener: (s: ConnectionStatus) => void) {
    this.listener = listener;
    return () => { this.listener = null; };
  }
  async disconnect() { this.disconnected++; }
  push(s: ConnectionStatus) { this.current = s; this.listener?.(s); }
}

afterEach(cleanup);

describe('the popup', () => {
  it('says how to connect when the library is not connected', async () => {
    render(<StatusPage client={new FakeClient({ state: 'idle', tabs: [] })} />);
    expect(await screen.findByText('Not connected')).toBeTruthy();
    expect(screen.getByText(/Chromium::connect_extension/)).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Disconnect' })).toBeNull();
  });

  it('lists the tabs being driven and disconnects', async () => {
    const client = new FakeClient({
      state: 'connected',
      relayUrl: 'ws://127.0.0.1:5/extension',
      tabs: [{ id: 1, title: 'Hello', url: 'https://a.test/' }],
    });
    render(<StatusPage client={client} />);
    expect(await screen.findByText('Connected')).toBeTruthy();
    expect(screen.getByText('Hello')).toBeTruthy();
    expect(screen.getByText('https://a.test/')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Disconnect' }));
    expect(client.disconnected).toBe(1);
    act(() => client.push({ state: 'idle', tabs: [] }));
    expect(await screen.findByText('Not connected')).toBeTruthy();
  });
});
