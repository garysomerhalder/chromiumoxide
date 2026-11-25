import type { Params } from '../domain/cdp';

/** A tab, or a child session (iframe, worker) inside it. */
export interface DebugTarget {
  tabId: number;
  sessionId?: string;
}

/** The browser's debugger for tabs (`chrome.debugger`). */
export interface DebuggerPort {
  attach(tabId: number): Promise<void>;
  detach(tabId: number): Promise<void>;
  /** The tab's DevTools target id. */
  targetIdOf(tabId: number): Promise<string | undefined>;
  send(target: DebugTarget, method: string, params?: Params): Promise<unknown>;
  onEvent(listener: (target: DebugTarget, method: string, params: Params) => void): void;
  onDetach(listener: (tabId: number) => void): void;
}
