// The connect page the library serves tells the extension where to connect.

const CONNECT =
  /^http:\/\/(127\.0\.0\.1|localhost|\[::1\]):(\d+)\/chromiumoxide-connect(?:[?#].*)?$/;

/** The relay's WebSocket for a connect-page URL; `null` for any other page. */
export function parseConnectUrl(url: string | undefined): string | null {
  const m = url ? CONNECT.exec(url) : null;
  return m ? `ws://${m[1]}:${m[2]}/extension` : null;
}
