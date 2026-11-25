/// <reference types="chrome" />
import type { Params } from '../domain/cdp';
import type { DebuggerPort, DebugTarget } from '../ports/debugger';

/** `chrome.debugger` (Chrome and Edge). */
export class ChromeDebugger implements DebuggerPort {
  async attach(tabId: number) {
    await chrome.debugger.attach({ tabId }, '1.3');
  }
  async detach(tabId: number) {
    await chrome.debugger.detach({ tabId });
  }
  async targetIdOf(tabId: number) {
    const targets = await chrome.debugger.getTargets();
    return targets.find((t) => t.tabId === tabId)?.id;
  }
  send(target: DebugTarget, method: string, params?: Params) {
    const session: chrome.debugger.DebuggerSession =
      target.sessionId === undefined ? { tabId: target.tabId } : { tabId: target.tabId, sessionId: target.sessionId };
    return chrome.debugger.sendCommand(session, method, params);
  }
  onEvent(listener: (target: DebugTarget, method: string, params: Params) => void) {
    chrome.debugger.onEvent.addListener((source, method, params) => {
      if (source.tabId === undefined) return;
      listener({ tabId: source.tabId, sessionId: source.sessionId }, method, (params ?? {}) as Params);
    });
  }
  onDetach(listener: (tabId: number) => void) {
    chrome.debugger.onDetach.addListener((source) => {
      if (source.tabId !== undefined) listener(source.tabId);
    });
  }
}

/** Whether this browser lets extensions debug tabs (Chrome, Edge). */
export function hasDebugger(): boolean {
  return typeof chrome !== 'undefined' && !!chrome.debugger;
}
