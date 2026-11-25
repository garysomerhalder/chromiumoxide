// What the popup shows.

export interface TabSummary {
  id: number;
  title: string;
  url: string;
}

export type ConnectionStatus =
  | { state: 'idle'; tabs: TabSummary[] }
  | { state: 'connected'; relayUrl: string; tabs: TabSummary[] };

export const IDLE: ConnectionStatus = { state: 'idle', tabs: [] };
