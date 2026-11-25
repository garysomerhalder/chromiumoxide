// DevTools permission types -> extension content settings.
import { Unavailable } from './cdp';

const SETTINGS: Record<string, string> = {
  geolocation: 'location',
  notifications: 'notifications',
  videoCapture: 'camera',
  audioCapture: 'microphone',
  clipboardReadWrite: 'clipboard',
  clipboardSanitizedWrite: 'clipboard',
};

export function contentSettingsFor(permissions: string[]): string[] {
  const out: string[] = [];
  for (const p of permissions) {
    const s = SETTINGS[p];
    if (!s) throw new Unavailable(`cannot be granted through the extension: ${p}`);
    if (!out.includes(s)) out.push(s);
  }
  return out;
}
