/** The extension's first message: which protocol it speaks to the library. */
export type Protocol = 'cdp' | 'pages';

export function hello(protocol: Protocol, userAgent: string) {
  return { method: 'Extension.hello', params: { protocol, userAgent } };
}
