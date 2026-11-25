import type { ConnectionStatus } from '../domain/status';

/** The popup's view of the background relay. */
export interface StatusClient {
  status(): Promise<ConnectionStatus>;
  subscribe(listener: (status: ConnectionStatus) => void): () => void;
  disconnect(): Promise<void>;
}
