/** Site permissions (`chrome.contentSettings`). */
export interface PermissionsPort {
  /** Allows these content settings for `origin` (every site when absent). */
  grant(settings: string[], origin?: string): Promise<void>;
  /** Clears what was granted. */
  reset(): Promise<void>;
}
