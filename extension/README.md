# chromiumoxide extension

Lets the chromiumoxide library drive the browser you already use (your tabs,
your logins), without starting it in remote-debugging mode.

| Browser | What the library gets |
|---|---|
| Chrome, Edge | The whole page API (`Chromium::connect_extension`): your profile is `browser.contexts()[0]`, its tabs are pages. |
| Firefox, Safari | Basic page actions (`ExtensionRelay::wait_pages`): list and open tabs, `goto`, `evaluate`, click, fill, screenshot. Their extensions get no debugger. |

Built with [WXT](https://wxt.dev) in TypeScript. It talks to the Rust
library over a local WebSocket the library opens.

## Install it once

```sh
cd extension
npm ci
npm run build          # .output/{chrome-mv3,edge-mv3,firefox-mv2,safari-mv3}
```

- **Chrome / Edge:** `chrome://extensions` (or `edge://extensions`), turn on
  Developer mode, "Load unpacked", pick `.output/chrome-mv3` (or `edge-mv3`).
- **Firefox:** `about:debugging#/runtime/this-firefox`, "Load Temporary
  Add-on", pick `.output/firefox-mv2/manifest.json` (temporary: until Firefox
  restarts; for a permanent install use Firefox Developer Edition with
  `xpinstall.signatures.required` set to `false`, and `npm run zip -- -b firefox`).
- **Safari (Mac):** `npm run package:safari` makes an Xcode project in
  `.output/safari-xcode`; build and run it in Xcode, then in Safari turn on
  Develop > Allow Unsigned Extensions and enable chromiumoxide in Settings >
  Extensions.

## Use it from Rust

The easiest: let the library start a separate Chrome or Edge with the
extension already loaded and its own profile (kept between runs, so logins
stay). It is an ordinary browser: no debugging port, no automation switches.

```rust
let browser = chromiumoxide::Chromium::launch_with_extension(
    chromiumoxide::ExtensionLaunchOptions {
        executable_path: Some(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into()),
        extension_path: "extension/.output/edge-mv3".into(),
        user_data_dir: "my-edge-profile".into(),
        headless: false,
        ..Default::default()
    },
).await?;
let page = browser.contexts()[0].new_page().await?;
browser.close().await?; // ends that browser
```

(Google Chrome 137 and later no longer loads unpacked extensions from the
command line; use Edge, Chromium or Chrome for Testing for this, or load the
extension by hand and use `connect_extension`.)

Or drive the browser you already have open:

```rust
// Chrome or Edge, your default browser:
let browser = chromiumoxide::Chromium::connect_extension(Default::default()).await?;
let me = browser.contexts()[0].clone();
for page in me.pages() {
    println!("{}", page.url());
}
let page = me.new_page().await?;
page.goto("https://example.com").await?;
browser.close().await?; // disconnects; your browser stays open

// Firefox or Safari: open the relay's page in that browser.
let relay = chromiumoxide::Chromium::extension_relay(Default::default()).await?;
println!("open {}", relay.connect_url());
let firefox = relay.wait_pages().await?;
let tab = firefox.new_tab("https://example.com").await?;
let title: String = tab.evaluate("document.title").await?;
```

Everything in the Playwright API works through the extension (see
`docs/PARITY.md` section 15), with two differences: a `data:` URL is shown
as the tab's content (Chrome refuses `data:` navigations from extensions,
so the address stays `about:blank`), and there are no new isolated contexts
(start another `launch_with_extension` with its own profile folder instead).
Video and network recording are set on the launch or relay options
(`record_video`, `record_har`) and cover every page driven.

While connected, Chrome shows "chromiumoxide started debugging this browser"
on the tabs it drives; that bar is Chrome's and cannot be hidden.

## How it works

1. The library listens on `127.0.0.1` (any free port) and serves
   `http://127.0.0.1:<port>/chromiumoxide-connect`.
2. Opening that page tells the extension (through a small script on the
   page) where to connect; it opens a WebSocket to `/extension` and says which
   protocol it speaks (`Extension.hello`).
3. Chrome and Edge: the extension relays Chrome's DevTools protocol to the
   tabs with `chrome.debugger`, and answers the browser-wide commands
   (targets, windows, version) itself. Firefox and Safari: a small page
   protocol (`Tabs.*`, `Tab.*`).

No token: it is for one person on one computer, and listens only on that
computer.

## Code layout

DDD + hexagonal, like the library; `tests/architecture.test.ts` checks it.

| Folder | What |
|---|---|
| `src/domain` | Pure rules: the connect page, which tabs can be driven, sessions, version, status. |
| `src/ports` | Interfaces: tabs, debugger, windows, page scripting, socket, status. |
| `src/application` | `CdpRelay` (Chrome, Edge) and `PageRelay` (Firefox, Safari). |
| `src/adapters` | The browser APIs and the WebSocket. |
| `src/ui` | The popup, in MUI, by atomic design: `atoms` → `molecules` → `organisms` → `templates` → `pages`. |
| `src/entrypoints` | Composition: background, popup, the connect page's script. |

```sh
npm test         # vitest: domain, relays (in-memory ports), layers, popup
npm run check    # TypeScript
```

The Rust tests (`tests/integration/extension.rs`) build the extension and
drive Playwright's Chromium and Firefox through it.
