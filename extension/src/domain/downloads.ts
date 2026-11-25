// Downloads as the library expects them (`Browser.download*` events).

/** A download, as browsers' extension API describes it. */
export interface DownloadItem {
  id: number;
  url: string;
  referrer: string;
  /** Absolute path once known. */
  filename: string;
  state: 'in_progress' | 'interrupted' | 'complete';
  totalBytes?: number;
  bytesReceived?: number;
}

/** The file name a download suggests: its path's, else its URL's. */
export function suggestedFilename(item: DownloadItem): string {
  const fromPath = item.filename.split(/[\\/]/).pop();
  if (fromPath) return fromPath;
  try {
    const last = new URL(item.url).pathname.split('/').pop();
    return last ? decodeURIComponent(last) : 'download';
  } catch {
    return 'download';
  }
}

/** `Browser.downloadProgress` for a finished download; `null` while running. */
export function progress(item: DownloadItem) {
  if (item.state === 'in_progress') return null;
  const base = {
    guid: String(item.id),
    state: item.state === 'complete' ? 'completed' : 'canceled',
    totalBytes: item.totalBytes ?? 0,
    receivedBytes: item.bytesReceived ?? 0,
  };
  return item.state === 'complete' && item.filename ? { ...base, filePath: item.filename } : base;
}
