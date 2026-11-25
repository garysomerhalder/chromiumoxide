import type { BrowserCookie, CookieSetting } from '../domain/cookies';

/** The profile's cookies. */
export interface CookiesPort {
  getAll(): Promise<BrowserCookie[]>;
  set(cookie: CookieSetting): Promise<void>;
  remove(url: string, name: string): Promise<void>;
}
