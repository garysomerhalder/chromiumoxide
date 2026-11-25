import type { DownloadItem } from '../domain/downloads';

export type { DownloadItem };

/** The browser's downloads (`chrome.downloads`). */
export interface DownloadsPort {
  onCreated(listener: (item: DownloadItem) => void): void;
  onChanged(listener: (item: DownloadItem) => void): void;
}
