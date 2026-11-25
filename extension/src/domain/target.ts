// Which tabs can be driven, and their session ids.

const BLOCKED = [
  /^https:\/\/chromewebstore\.google\.com\//,
  /^https:\/\/chrome\.google\.com\/webstore/,
  /^https:\/\/microsoftedge\.microsoft\.com\/addons/,
];

/** Browsers refuse to debug their own pages (chrome://, edge://, extensions, the stores). */
export function isAttachable(url: string | undefined): boolean {
  if (url === undefined || url === '') return true;
  return /^(https?|file|about|data|blob):/.test(url) && !BLOCKED.some((b) => b.test(url));
}

/** A tab's own session id, as the library sees it. */
export function tabSession(tabId: number): string {
  return `tab-${tabId}`;
}

/** The tab of a tab's own session id; `null` for a child session (iframe, worker). */
export function parseTabSession(sessionId: string): number | null {
  const m = /^tab-(\d+)$/.exec(sessionId);
  return m ? Number(m[1]) : null;
}

/** A tab as the library sees it: `Target.TargetInfo`, in the browser's own profile. */
export function targetInfo(
  targetId: string,
  tab: { url: string; title: string } | undefined,
  openerId?: string,
) {
  return {
    targetId,
    type: 'page',
    title: tab?.title ?? '',
    url: tab?.url ?? '',
    attached: true,
    canAccessOpener: openerId !== undefined,
    browserContextId: '',
    ...(openerId === undefined ? {} : { openerId }),
  };
}
