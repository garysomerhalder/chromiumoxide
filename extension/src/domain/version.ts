/** `Browser.getVersion`, from the user agent (Chrome and Edge report Chrome/<version>). */
export function browserVersion(userAgent: string) {
  const version = /Chrome\/([\d.]+)/.exec(userAgent)?.[1] ?? '';
  return { protocolVersion: '1.3', product: `Chrome/${version}`, userAgent, jsVersion: '', revision: '' };
}
