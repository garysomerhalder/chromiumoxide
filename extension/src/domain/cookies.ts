// DevTools cookies <-> the extension cookie API.
/** A cookie as browsers' extension API gives it (`chrome.cookies.Cookie`). */
export interface BrowserCookie {
  name: string;
  value: string;
  domain: string;
  path: string;
  secure: boolean;
  httpOnly: boolean;
  sameSite: 'no_restriction' | 'lax' | 'strict' | 'unspecified';
  session: boolean;
  expirationDate?: number;
  hostOnly: boolean;
}

/** What `chrome.cookies.set` takes. */
export interface CookieSetting {
  url: string;
  name: string;
  value: string;
  domain?: string;
  path?: string;
  secure?: boolean;
  httpOnly?: boolean;
  sameSite?: BrowserCookie['sameSite'];
  expirationDate?: number;
}


const TO_CDP = { no_restriction: 'None', lax: 'Lax', strict: 'Strict', unspecified: 'Lax' } as const;
const FROM_CDP = { None: 'no_restriction', Lax: 'lax', Strict: 'strict' } as const;

/** `Network.Cookie`. */
export function fromBrowserCookie(c: BrowserCookie) {
  return {
    name: c.name,
    value: c.value,
    domain: c.domain,
    path: c.path,
    expires: c.session || c.expirationDate === undefined ? -1 : c.expirationDate,
    size: c.name.length + c.value.length,
    httpOnly: c.httpOnly,
    secure: c.secure,
    session: c.session,
    sameSite: TO_CDP[c.sameSite],
  };
}

/** A `Network.CookieParam` (from `Storage.setCookies`). */
export interface CookieParam {
  name: string;
  value: string;
  url?: string;
  domain?: string;
  path?: string;
  secure?: boolean;
  httpOnly?: boolean;
  sameSite?: 'None' | 'Lax' | 'Strict';
  expires?: number;
}

export function toBrowserCookie(p: CookieParam): CookieSetting {
  const host = (p.domain ?? '').replace(/^\./, '');
  const url = p.url ?? `http${p.secure ? 's' : ''}://${host}${p.path ?? '/'}`;
  const out: CookieSetting = { url, name: p.name, value: p.value };
  // A leading dot marks a domain cookie; without it the cookie is host-only.
  if (p.domain?.startsWith('.')) out.domain = p.domain;
  if (p.path !== undefined) out.path = p.path;
  if (p.secure !== undefined) out.secure = p.secure;
  if (p.httpOnly !== undefined) out.httpOnly = p.httpOnly;
  if (p.sameSite !== undefined) out.sameSite = FROM_CDP[p.sameSite];
  if (p.expires !== undefined && p.expires > 0) out.expirationDate = p.expires;
  return out;
}

/** The URL a stored cookie belongs to (to remove it). */
export function cookieUrl(c: BrowserCookie): string {
  return `http${c.secure ? 's' : ''}://${c.domain.replace(/^\./, '')}${c.path}`;
}
