import type { Tab } from './tabs';

/** What a browser without a debugger lets an extension do in a tab. */
export interface PagePort {
  /** Loads `url` and resolves once it has loaded. */
  navigate(tabId: number, url: string): Promise<Tab>;
  /** Runs `expression` in the page; its (JSON) value. */
  evaluate(tabId: number, expression: string): Promise<unknown>;
  click(tabId: number, selector: string): Promise<void>;
  fill(tabId: number, selector: string, value: string): Promise<void>;
  /** The visible tab as a base64 PNG. */
  screenshot(tabId: number): Promise<string>;
}
