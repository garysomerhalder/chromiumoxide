// Runs on the library's connect page and tells the background where the
// library is listening. The background may still be starting (a fresh
// profile, or asleep), so it asks again until the background has connected.
export default defineContentScript({
  matches: [
    'http://127.0.0.1/chromiumoxide-connect*',
    'http://localhost/chromiumoxide-connect*',
    'http://[::1]/chromiumoxide-connect*',
  ],
  runAt: 'document_start',
  async main() {
    for (let attempt = 0; attempt < 120; attempt++) {
      try {
        const connected = await browser.runtime.sendMessage({ type: 'connect', url: location.href });
        if (connected === true) return;
      } catch {
        // No background listening yet.
      }
      await new Promise((r) => setTimeout(r, 250));
    }
  },
});
