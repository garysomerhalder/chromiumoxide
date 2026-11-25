// Runs on the library's connect page and tells the background, which may
// still be starting (or asleep), where the library is listening.
export default defineContentScript({
  matches: [
    'http://127.0.0.1/chromiumoxide-connect*',
    'http://localhost/chromiumoxide-connect*',
    'http://[::1]/chromiumoxide-connect*',
  ],
  runAt: 'document_start',
  main() {
    void browser.runtime.sendMessage({ type: 'connect', url: location.href });
  },
});
