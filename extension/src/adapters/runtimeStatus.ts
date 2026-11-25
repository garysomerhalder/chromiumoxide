// The popup and the background talk over a runtime port named "status".
import type { ConnectionStatus } from '../domain/status';
import type { StatusClient } from '../ports/status';

const PORT = 'status';

/** What the background shares with the popup. */
export interface StatusSource {
  status(): ConnectionStatus;
  onStatus(listener: (s: ConnectionStatus) => void): () => void;
  disconnect(): Promise<void>;
}

/** Background side: answers popups. */
export function serveStatus(source: StatusSource) {
  browser.runtime.onConnect.addListener((port) => {
    if (port.name !== PORT) return;
    port.postMessage(source.status());
    const stop = source.onStatus((s) => port.postMessage(s));
    port.onMessage.addListener((m: unknown) => {
      if ((m as { type?: string })?.type === 'disconnect') void source.disconnect();
    });
    port.onDisconnect.addListener(stop);
  });
}

/** Popup side. */
export class RuntimeStatusClient implements StatusClient {
  private port = browser.runtime.connect({ name: PORT });
  private latest: ConnectionStatus | null = null;
  private listeners: ((s: ConnectionStatus) => void)[] = [];
  constructor() {
    this.port.onMessage.addListener((m: unknown) => {
      this.latest = m as ConnectionStatus;
      this.listeners.forEach((l) => l(this.latest!));
    });
  }
  async status() {
    return this.latest ?? { state: 'idle' as const, tabs: [] };
  }
  subscribe(listener: (s: ConnectionStatus) => void) {
    this.listeners.push(listener);
    return () => {
      this.listeners = this.listeners.filter((l) => l !== listener);
    };
  }
  async disconnect() {
    this.port.postMessage({ type: 'disconnect' });
  }
}
