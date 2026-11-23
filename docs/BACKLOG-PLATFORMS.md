# Backlog: other engines, apps, Android and the whole of CDP

Requested on 2026-11-23: Firefox, Safari (WebKit), Android, Electron, Tauri and
all of the Chrome DevTools Protocol. This widens the scope in `CLAUDE.md`
(Chromium only until now). Each item is done test-first, like the milestones in
[`PARITY.md`](PARITY.md).

Hard rule kept from the device-mockup contract: none of this may change how
Chromium is found, launched or rendered by default. `tests/pixel_parity.rs`
must stay green.

## Where we stand (2026-11-23)

| Target | Playwright 1.65.1 | Protocol | In this container | chromiumoxide |
|---|---|---|---|---|
| Chromium | `chromium` | CDP | Chrome for Testing 159, headless shell 159 | done (M1-M9) |
| Firefox | `firefox` (Juggler, Playwright's own Firefox protocol) and `_bidiFirefox` (WebDriver BiDi) | WebDriver BiDi | Playwright's Firefox 159 (`firefox-1572`); its WebDriver BiDi endpoint works (`--remote-debugging-port`, `ws://…/session`) | not started |
| WebKit (Safari's engine) | `webkit` | Playwright's WebKit inspector protocol (`protocol.json` ships with the build) | Playwright's WebKit 27.2 (`webkit-2388`, GTK and WPE builds) | not started |
| Android | `_android` (experimental) | adb, then CDP to Chrome or WebViews | `adb` 1.0.41; no device, no emulator (no KVM) | not started |
| Electron | `_electron` (experimental) | CDP for windows, Node inspector for the main process | Electron 45.4.1 | not started |
| Tauri | not supported | WebView2 on Windows: CDP. WebKitGTK on Linux: WebDriver (`WebKitWebDriver`) or the WebKit remote inspector. macOS: none | `webkit2gtk-driver` 2.52 installable | not started |
| CDP as a whole | `CDPSession.send` | CDP | — | every command is typed in `chromiumoxide_cdp` and can be sent; 92 of 596 browser commands and 19 of 82 JavaScript commands have a high-level API |

## The big change: engines behind a port

The application layer speaks CDP directly (155 method names in 37 files).
Firefox and WebKit need the same use cases over other protocols. Plan:

1. Add `chromiumoxide::{Chromium, Firefox, WebKit}` browser types with the same
   `launch` / `launch_persistent_context` / `connect` entry points.
2. Introduce an engine port: the page operations the application needs
   (navigate, evaluate in a world, resolve and act on elements, input,
   screenshot, emulation, network interception, events), not protocol calls.
3. Move today's CDP code behind it as the Chromium adapter, without behaviour
   changes. The integration suite and the pixel-parity test must pass at
   every step.
4. Add the Firefox (BiDi) and WebKit adapters, growing from the
   device-mockup subset (contexts, emulation, goto/setContent, evaluate,
   screenshots) to the full API. The injected script (Playwright's) is the
   same for all three engines, so locators, assertions and aria snapshots
   follow once evaluation works.

Electron, Android and the CDP domains reuse the Chromium adapter and need no
port change, so they come first.

## Milestones

### M10 Electron (`_electron`)

| Item | Playwright API | Notes |
|---|---|---|
| Launch | `electron.launch({ executablePath, args, cwd, env, timeout, ...context options })` | Starts the app with `--inspect=0 --remote-debugging-port=0`, like Playwright. |
| Main process | `electronApp.evaluate(fn, arg)`, `evaluateHandle`, `process()` | `fn(electronModule, arg)` in the main process, over the Node inspector (CDP `Runtime`). |
| Windows | `firstWindow()`, `windows()`, `on('window')`, `browserWindow(page)` | Pages over the renderer CDP connection; `browserWindow` returns the `BrowserWindow` handle. |
| Context | `context()` | The app's default context, with the context options applied. |
| Events and close | `on('console')` (main process), `on('close')`, `close()`, `waitForEvent` | |

### M11 Android (`_android`)

| Item | Playwright API | Notes |
|---|---|---|
| Devices | `android.devices({ host, port, omitDriverInstall })`, `android.connect`, `launchServer` | Talks to the adb server (port 5037) directly. |
| Device | `serial()`, `model()`, `info()`, `shell()`, `open()` (`AndroidSocket`), `push()`, `installApk()`, `screenshot()`, `close()` | |
| Input | `tap`, `fill`, `press`, `drag`, `fling`, `longTap`, `pinchOpen/Close`, `scroll`, `swipe`, `wait`, `input.*` | Playwright uses its UI-automator driver APK (`omitDriverInstall`); needs that APK or a replacement. |
| Browsers and WebViews | `launchBrowser()`, `webViews()`, `webView()`, `on('webview')`, `AndroidWebView.page()` | CDP over `localabstract:chrome_devtools_remote` / `webview_devtools_remote_<pid>`. |
| Tests | — | No emulator here: integration tests use a fake adb server that forwards the DevTools socket to a local Chromium. |

### M12 Firefox (WebDriver BiDi)

| Item | Notes |
|---|---|
| Engine port and Chromium adapter (refactor, no behaviour change) | See "The big change" above. |
| `Firefox::launch` | Playwright's Firefox build (and stock Firefox): profile, prefs, `--remote-debugging-port`, BiDi session. |
| Device-mockup subset | Contexts (`browser.createUserContext`), viewport and device emulation, `goto` / `setContent`, `evaluate`, screenshots (`browsingContext.captureScreenshot`). |
| Locators, input, assertions | Playwright's injected script through `script.callFunction`; `input.performActions`. |
| Network, routing, storage, downloads, dialogs | `network.addIntercept`, `storage.*`, `browsingContext.handleUserPrompt`. |
| Pixel parity with Playwright's Firefox | Compare screenshots with `firefox.launch()` of the same build. Playwright drives it over Juggler, so differences are possible. |
| Juggler (Playwright's own Firefox protocol) | Optional, only if BiDi lacks something Playwright's API needs. |

### M13 WebKit

| Item | Notes |
|---|---|
| `WebKit::launch` | Playwright's WebKit build (`pw_run.sh`, `--inspector-pipe`), GTK headless. |
| Protocol | Generate Rust types from the build's `protocol.json` (a second generator input, like the CDP PDL). |
| Device-mockup subset, then the full API | As for Firefox. |
| Pixel parity with Playwright's WebKit | Same build on both sides. |

### M14 Tauri

| Item | Notes |
|---|---|
| Windows (WebView2) | Launch the app with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>` and connect over CDP: the whole Chromium API. Needs a Windows CI runner. |
| Linux (WebKitGTK) | Through `WebKitWebDriver` (`webkit2gtk-driver`), as Tauri's `tauri-driver` does, or the WebKit remote inspector. Reuses the M13 WebKit work where the protocols match. |
| macOS | No supported automation for WKWebView; recorded as not possible. |
| Tests | A minimal Tauri app built in the test setup (Linux, WebKitGTK). |

### M15 The rest of CDP

**Done (2026-11-23, branch `claude/rewrite-m12-cdp`):** every domain below
has a generated typed client (`page.protocol().<domain>()`) and a
real-Chromium test; see `docs/PARITY.md` section 12. Hand-written,
Playwright-style APIs on top of these clients stay possible per domain.

Every CDP command is already available, typed, through `chromiumoxide_cdp`
and `page.send()` / `CdpSessionHandle`. This milestone adds a tested
high-level API for the domains that have none, most useful first.

| Priority | Domains |
|---|---|
| 1 | Performance (metrics), Memory, Profiler / HeapProfiler (profiles, heap snapshots), Accessibility (full tree), DOMSnapshot, Animation, Audits (issues), Security, ServiceWorker, CacheStorage, IndexedDB, Storage (quota, shared storage, interest groups), Overlay (inspect mode, highlights) |
| 2 | Emulation (the 39 unused commands: CPU throttling, sensors, idle, pressure, auto dark mode, ...), Network (throttling, cache, cookies by partition), Page (lifecycle, permissions policy, app manifest, ...), DOM / CSS editing and DOMDebugger breakpoints, Debugger (stepping, breakpoints), LayerTree, Log, Media, WebAudio, Preload, BackgroundService |
| 3 | Experimental device emulation: BluetoothEmulation, SmartCardEmulation, DeviceOrientation, DeviceAccess, FedCm, Autofill, DigitalCredentials, PWA, Extensions, Cast, Tethering, SystemInfo, FileSystem, FindInPage, HeadlessExperimental, EventBreakpoints, CrashReportContext, Ads, PerformanceTimeline |

Deprecated domains (Console, Schema) are left as raw commands.

Per-domain counts (command and event names the high-level API uses / total),
from the vendored protocol `v0.0.1751165`:

| Domain | Status | Commands | Events |
|---|---|---|---|
| Accessibility | experimental | 0 / 8 | 0 / 2 |
| Ads | experimental | 0 / 2 | 0 / 0 |
| Animation | experimental | 0 / 10 | 0 / 4 |
| Audits | experimental | 0 / 4 | 0 / 1 |
| Autofill | experimental | 0 / 4 | 0 / 1 |
| BackgroundService | experimental | 0 / 4 | 0 / 2 |
| BluetoothEmulation | experimental | 0 / 16 | 0 / 3 |
| Browser | stable | 8 / 21 | 2 / 2 |
| CSS | experimental | 5 / 40 | 1 / 6 |
| CacheStorage | experimental | 0 / 5 | 0 / 0 |
| Cast | experimental | 0 / 6 | 0 / 2 |
| CrashReportContext | experimental | 0 / 1 | 0 / 0 |
| DOM | stable | 9 / 57 | 0 / 19 |
| DOMDebugger | stable | 0 / 10 | 0 / 0 |
| DOMSnapshot | experimental | 0 / 4 | 0 / 0 |
| DOMStorage | experimental | 0 / 6 | 0 / 4 |
| DeviceAccess | experimental | 0 / 4 | 0 / 1 |
| DeviceOrientation | experimental | 0 / 2 | 0 / 0 |
| DigitalCredentials | experimental | 0 / 1 | 0 / 0 |
| Emulation | stable | 10 / 49 | 0 / 2 |
| EventBreakpoints | experimental | 0 / 3 | 0 / 0 |
| Extensions | experimental | 0 / 8 | 0 / 0 |
| FedCm | experimental | 0 / 7 | 0 / 2 |
| Fetch | stable | 8 / 9 | 2 / 2 |
| FileSystem | experimental | 0 / 1 | 0 / 0 |
| FindInPage | experimental | 0 / 4 | 0 / 0 |
| HeadlessExperimental | experimental | 0 / 3 | 0 / 0 |
| IO | stable | 2 / 3 | 0 / 0 |
| IndexedDB | experimental | 0 / 9 | 0 / 0 |
| Input | stable | 6 / 13 | 1 / 1 |
| Inspector | experimental | 0 / 2 | 1 / 4 |
| LayerTree | experimental | 0 / 9 | 0 / 2 |
| Log | stable | 1 / 5 | 1 / 1 |
| Media | experimental | 0 / 2 | 0 / 5 |
| Memory | experimental | 0 / 11 | 0 / 0 |
| Network | stable | 6 / 35 | 11 / 43 |
| Overlay | experimental | 0 / 31 | 0 / 6 |
| PWA | experimental | 0 / 7 | 0 / 0 |
| Page | stable | 21 / 67 | 12 / 28 |
| Performance | stable | 0 / 4 | 0 / 1 |
| PerformanceTimeline | experimental | 0 / 1 | 0 / 1 |
| Preload | experimental | 0 / 2 | 0 / 6 |
| Security | stable | 1 / 5 | 0 / 3 |
| ServiceWorker | experimental | 0 / 12 | 0 / 3 |
| SmartCardEmulation | experimental | 0 / 12 | 0 / 14 |
| Storage | experimental | 3 / 27 | 0 / 7 |
| SystemInfo | experimental | 0 / 3 | 0 / 0 |
| Target | stable | 5 / 19 | 2 / 7 |
| Tethering | experimental | 0 / 2 | 0 / 1 |
| Tracing | stable | 2 / 6 | 1 / 3 |
| WebAudio | experimental | 0 / 3 | 0 / 13 |
| WebAuthn | experimental | 5 / 13 | 0 / 4 |
| WebMCP | experimental | 0 / 4 | 0 / 4 |
| Console (JS) | deprecated | 0 / 3 | 0 / 1 |
| Debugger (JS) | stable | 5 / 34 | 2 / 5 |
| HeapProfiler (JS) | experimental | 1 / 12 | 0 / 5 |
| Profiler (JS) | stable | 5 / 9 | 0 / 3 |
| Runtime (JS) | stable | 8 / 23 | 6 / 8 |
| Schema (JS) | deprecated | 0 / 1 | 0 / 0 |

## Order of work

M10 Electron, M11 Android, M15 CDP priority 1, then the engine-port refactor
and M12 Firefox, M13 WebKit, M14 Tauri, and the rest of M15. Electron and
Android reuse the Chromium code as it is, and so does the CDP work. The
refactor touches everything, so it waits until those are in.
