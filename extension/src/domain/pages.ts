// The page protocol (Firefox, Safari): what each command needs.
import { type Params, Unavailable } from './cdp';

export const PAGE_METHODS = [
  'Tabs.list',
  'Tabs.create',
  'Tab.info',
  'Tab.goto',
  'Tab.evaluate',
  'Tab.click',
  'Tab.fill',
  'Tab.screenshot',
  'Tab.close',
] as const;

export type PageMethod = (typeof PAGE_METHODS)[number];

export function pageMethod(method: string): PageMethod {
  if ((PAGE_METHODS as readonly string[]).includes(method)) return method as PageMethod;
  throw new Unavailable(`not available through the extension in this browser: ${method}`);
}

export function numberParam(method: string, p: Params, key: string): number {
  const v = p[key];
  if (typeof v !== 'number') throw new Error(`${method} needs a number ${key}`);
  return v;
}

export function stringParam(method: string, p: Params, key: string): string {
  const v = p[key];
  if (typeof v !== 'string') throw new Error(`${method} needs a string ${key}`);
  return v;
}
