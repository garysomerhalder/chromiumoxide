# Parity checklists

Legend: ✅ done (tested) · 🟡 partial · ⬜ not started. Milestones: M1 device-mockup
needs, M2 protocol regenerated, M3 navigation/waiting/locators/input, M4 network,
storage, downloads, dialogs, M5 tracing/video/HAR/PDF/assertions, M6 the rest,
M7 Playwright 1.64/1.65 (section 7), M8 the backlog of options, events and
methods (section 8).

Playwright reference: **1.65.1** (`playwright-core/types/types.d.ts`; M1-M6
were built against 1.63.0, M7 brings everything to 1.65.1).
Firefox (WebDriver BiDi) has its own engine since M13; section 13 is its column: what works on Firefox, the test that shows it, or why not. WebKit is not started.

The member tables below track methods; section 8 covers the options, events
and methods found by the audit of 2026-11-22 ([`BACKLOG.md`](BACKLOG.md)),
all done in M8. Only the unreleased Playwright 1.66 remains there.

### Notes

- **Launch preset.** The default switches are the device-mockup contract
  (`domain::launch`), read from `/proc/<pid>/cmdline` of a real Playwright
  launch of `chromium_headless_shell-1243` in the container image (that
  install's wrapper adds `--utility-cmd-prefix`, which is left out). A
  `pw:browser` debug log of Playwright's own argument list differs because
  the image's browser wrapper adjusts switches. The parity test gives
  Playwright exactly our command line. M7 moved the preset from 1.63 to
  1.65.1 (agreed with device-mockup); the 1.63 list is kept in the
  `domain::launch` tests as `PRESET_1_63` so pixel changes can be traced.
- **Known Chromium race (shared with Playwright).** Chromium sometimes styles a
  new page's initial `about:blank` document before the emulated device metrics
  reach the renderer; `setContent` (`document.open`) reuses that document, so
  a text control can be laid out 1 CSS px off (about 1 in 30 pages, seen with
  both Playwright and chromiumoxide). Replacing the initial document would
  avoid it but breaks parity (Playwright keeps it; e.g. mobile focus rings
  differ), so chromiumoxide behaves like Playwright. The parity test
  re-renders a mismatching shot up to 3 times on both sides.

## 1. Chrome DevTools Protocol

Target: latest `browser_protocol.pdl` + `js_protocol.pdl` from
ChromeDevTools/devtools-protocol (vendored: tag **v0.0.1751165**, deprecated items included), tested against current stable Chrome,
Chrome for Testing and chrome-headless-shell.

| Item | Milestone | Status |
|---|---|---|
| Protocol regenerated from latest PDL (all domains, experimental included) | M2 | ✅ |
| Generator handles every PDL construct in the latest files | M2 | ✅ |
| Revision recorded in `chromiumoxide_cdp::revision` | M2 | ✅ |
| Pipe transport (`--remote-debugging-pipe`) | M1 | ✅ |
| WebSocket transport (`connect_over_cdp`) | M1 | ✅ |
| Flat sessions (`Target.setAutoAttach { flatten: true }`) | M1 | ✅ |
| Raw/typed CDP escape hatch (`page.execute(cmd)`, `page.cdp_events::<E>()`, `browser.execute`, `page.send(method, json)`) | M2 | ✅ |
| Tested against chrome-headless-shell | M1 | ✅ |
| Tested against Chromium / Chrome for Testing (new headless) | M1 | ✅ |

## 2. Launch and discovery (device-mockup contract)

| Item | Milestone | Status |
|---|---|---|
| Explicit executable path | M1 | ✅ |
| `MOCKUP_CHROMIUM`, then `CHROME` env | M1 | ✅ |
| `PLAYWRIGHT_BROWSERS_PATH` | M1 | ✅ |
| `/opt/pw-browsers` | M1 | ✅ |
| `~/.cache/ms-playwright`, `~/Library/Caches/ms-playwright` | M1 | ✅ |
| `chromium_headless_shell-*` before `chromium-*`, newest revision first | M1 | ✅ |
| Chrome for Testing (fetcher cache) | M1 | ✅ |
| System Chrome / Chromium | M1 | ✅ |
| Playwright default Chromium argument preset (exact list) | M1 | ✅ |
| `--headless` (old) for headless shell, `--headless=new` for full Chrome | M1 | ✅ |
| Unique temporary user-data-dir per launch, removed on close | M1 | ✅ |
| Parallel launches do not interfere | M1 | ✅ |
| Playwright's `channel` (`chrome`, `chrome-beta`, `chrome-dev`, `chrome-canary`, `msedge`, `msedge-beta`, `msedge-dev`, `msedge-canary`) as `LaunchOptions::browser_channel`; default discovery unchanged | M14 | ✅ `edge_channel_runs_the_installed_edge` (Edge 158 on Linux and Windows) |
| Windows: `--remote-debugging-pipe` on the C runtime's fds 3 and 4 (as Node passes them for Playwright); the browser quits when its parent goes away | M14 | ✅ the whole suite on Windows 11 |

## 3. Browser context options

| Option | Milestone | Status |
|---|---|---|
| `viewport` / `noViewport` (default 1280×720) | M1 | ✅ |
| `screen` | M1 | ✅ |
| `deviceScaleFactor` | M1 | ✅ |
| `isMobile` | M1 | ✅ |
| `hasTouch` | M1 | ✅ |
| `userAgent` (+ UA client-hint metadata) | M1 | ✅ |
| `locale` | M1 | ✅ |
| `timezoneId` | M1 | ✅ |
| `colorScheme`, `reducedMotion`, `forcedColors`, `contrast` | M1 | ✅ |
| `httpCredentials` | M1 | ✅ |
| `geolocation` | M4 | ✅ (and `context.set_geolocation`) |
| `permissions` | M4 | ✅ Playwright permission names (and `grant_permissions` / `clear_permissions`) |
| `offline` | M4 | ✅ (and `context.set_offline`) |
| `extraHTTPHeaders` | M4 | ✅ (and context / page `set_extra_http_headers`) |
| `javaScriptEnabled`, `bypassCSP`, `ignoreHTTPSErrors` | M4 | ✅ |
| `storageState` | M4 | ✅ cookies, localStorage and IndexedDB (`StorageStateOptions`), JSON-compatible with Playwright files |
| `acceptDownloads` | M4 | ✅ |
| `recordHar` | M5 | ✅ `record_har` (embedded or omitted content, URL filter) |
| `recordVideo` | M5 | ✅ VP8 `.webm` via Playwright's ffmpeg; default size like Playwright |
| `serviceWorkers` | M4 | ✅ `allow` / `block` |
| `baseURL` | M3 | ✅ |
| Device descriptors (`playwright.devices`) | M1 | ✅ |

## 4. Screenshots

| Option | Milestone | Status |
|---|---|---|
| Viewport screenshot | M1 | ✅ |
| `clip` | M1 | ✅ |
| `fullPage` | M1 | ✅ |
| Waits for `document.fonts.ready`, hides caret by default | M1 | ✅ |
| `type` png/jpeg, `quality` | M1 | ✅ |
| `omitBackground` | M1 | ✅ |
| `scale: css / device` | M1 | ✅ |
| `animations: disabled` | M3 | ✅ |
| `caret: initial` | M3 | ✅ |
| `style` | M3 | ✅ |
| `mask`, `maskColor` | M3 | ✅ |
| Element / locator screenshots | M3 | ✅ |
| Pixel parity with Playwright proven by test | M1 | ✅ |

## 5. Web-first assertions (`expect`)

| Assertion | Milestone | Status |
|---|---|---|
| locator `toBeAttached` | M5 | ✅ |
| locator `toBeChecked` | M5 | ✅ |
| locator `toBeDisabled` | M5 | ✅ |
| locator `toBeEditable` | M5 | ✅ |
| locator `toBeEmpty` | M5 | ✅ |
| locator `toBeEnabled` | M5 | ✅ |
| locator `toBeFocused` | M5 | ✅ |
| locator `toBeHidden` | M5 | ✅ |
| locator `toBeInViewport` | M5 | ✅ |
| locator `toBeVisible` | M5 | ✅ |
| locator `toContainClass` | M5 | ✅ |
| locator `toContainText` | M5 | ✅ |
| locator `toHaveAccessibleDescription` | M5 | ✅ |
| locator `toHaveAccessibleErrorMessage` | M5 | ✅ |
| locator `toHaveAccessibleName` | M5 | ✅ |
| locator `toHaveAttribute` | M5 | ✅ |
| locator `toHaveCSS` | M5 | ✅ |
| locator `toHaveClass` | M5 | ✅ |
| locator `toHaveCount` | M5 | ✅ |
| locator `toHaveId` | M5 | ✅ |
| locator `toHaveJSProperty` | M5 | ✅ |
| locator `toHaveRole` | M5 | ✅ |
| locator `toHaveScreenshot` | M5 | ✅ PNG snapshots (no webp) |
| locator `toHaveText` | M5 | ✅ |
| locator `toHaveValue` | M5 | ✅ |
| locator `toHaveValues` | M5 | ✅ |
| locator `toMatchAriaSnapshot` | M5 | ✅ (+ template files) |
| page `toHaveScreenshot` | M5 | ✅ PNG snapshots (no webp) |
| page `toHaveTitle` | M5 | ✅ |
| page `toHaveURL` | M5 | ✅ |
| page `toMatchAriaSnapshot` | M5 | ✅ (+ template files) |
| response `toBeOK` | M5 | ✅ |

## 6. Playwright API, member by member

### Browser

| Member | Milestone | Status |
|---|---|---|
| `bind` | M3 | ✅ `browser.bind()` (CDP endpoint, works for pipe launches) |
| `browserType` | M1 | ✅ |
| `close` | M1 | ✅ |
| `contexts` | M1 | ✅ |
| `isConnected` | M1 | ✅ |
| `newBrowserCDPSession` | M3 | ✅ |
| `newContext` | M1 | ✅ |
| `newPage` | M1 | ✅ |
| `startTracing` | M3 | ✅ |
| `stopTracing` | M3 | ✅ |
| `unbind` | M3 | ✅ `browser.unbind()` |
| `version` | M1 | ✅ |

### BrowserContext

| Member | Milestone | Status |
|---|---|---|
| `exposeBinding` | M3 | ✅ |
| `addInitScript` | M3 | ✅ |
| `addCookies` | M4 | ✅ |
| `backgroundPages` | M4 | ✅ always empty, like Playwright (deprecated there) |
| `browser` | M1 | ✅ |
| `clearCookies` | M4 | ✅ |
| `clearPermissions` | M4 | ✅ |
| `close` | M1 | ✅ |
| `cookies` | M4 | ✅ |
| `exposeFunction` | M3 | ✅ |
| `grantPermissions` | M4 | ✅ |
| `isClosed` | M1 | ✅ |
| `newCDPSession` | M3 | ✅ |
| `newPage` | M1 | ✅ |
| `pages` | M1 | ✅ |
| `route` | M4 | ✅ |
| `routeFromHAR` | M5 | ✅ |
| `routeWebSocket` | M4 | ✅ |
| `serviceWorkers` | M4 | ✅ |
| `setDefaultNavigationTimeout` | M3 | ✅ |
| `setDefaultTimeout` | M3 | ✅ |
| `setExtraHTTPHeaders` | M4 | ✅ |
| `setGeolocation` | M4 | ✅ |
| `setHTTPCredentials` | M1 | ✅ |
| `setOffline` | M4 | ✅ |
| `setStorageState` | M4 | ✅ |
| `storageState` | M4 | ✅ |
| `unroute` | M4 | ✅ |
| `unrouteAll` | M4 | ✅ |
| `waitForEvent` | M3 | ✅ `events()` stream, `wait_for_page()` |
| events: `page`, `serviceworker`, `close`, `request`, `response`, `requestfinished`, `requestfailed`, `console`, `dialog`, `weberror` | M4 | ✅ `ContextEvent` |
| `clock` | M6 | ✅ |
| `credentials` | M4 | ✅ via Chromium's virtual WebAuthn authenticator |
| `debugger` | M6, M8 | ✅ requestPause, next, resume, runTo, pausedDetails (title, location from backtraces), events |
| `request` | M6 | ✅ |
| `tracing` | M5 | ✅ actions, console, network, screencast, DOM snapshots |

### Page

| Member | Milestone | Status |
|---|---|---|
| `evaluate` | M1 | ✅ |
| `evaluateHandle` | M3 | ✅ |
| `addInitScript` | M3 | ✅ |
| `waitForFunction` | M1 | ✅ |
| `waitForSelector` | M3 | ✅ |
| `exposeBinding` | M3 | ✅ |
| `addLocatorHandler` | M3 | ✅ |
| `addScriptTag` | M3 | ✅ |
| `addStyleTag` | M1 | ✅ |
| `ariaSnapshot` | M5 | ✅ |
| `ariaSnapshotJSON` | M5 | ✅ |
| `bringToFront` | M3 | ✅ |
| `cancelPickLocator` | M6 | ✅ |
| `check` | M3 | ✅ |
| `clearConsoleMessages` | M4 | ✅ |
| `clearPageErrors` | M4 | ✅ |
| `click` | M3 | ✅ |
| `close` | M1 | ✅ |
| `consoleMessages` | M4 | ✅ |
| `content` | M1 | ✅ |
| `context` | M1 | ✅ |
| `dblclick` | M3 | ✅ |
| `dispatchEvent` | M3 | ✅ |
| `dragAndDrop` | M3 | ✅ mouse and HTML5 drags |
| `emulateMedia` | M3 | ✅ |
| `exposeFunction` | M3 | ✅ |
| `fill` | M3 | ✅ |
| `focus` | M3 | ✅ |
| `frame` | M3 | ✅ |
| `frameLocator` | M3 | ✅ |
| `frames` | M3 | ✅ incl. out-of-process (cross-site) iframes, with routes, init scripts and bindings |
| `getAttribute` | M3 | ✅ |
| `getByAltText` | M3 | ✅ |
| `getByLabel` | M3 | ✅ |
| `getByPlaceholder` | M3 | ✅ |
| `getByRole` | M3 | ✅ |
| `getByTestId` | M3 | ✅ |
| `getByText` | M3 | ✅ |
| `getByTitle` | M3 | ✅ |
| `goBack` | M3 | ✅ |
| `goForward` | M3 | ✅ |
| `goto` | M1 | ✅ |
| `hideHighlight` | M3 | ✅ |
| `hover` | M3 | ✅ |
| `innerHTML` | M3 | ✅ |
| `innerText` | M3 | ✅ |
| `inputValue` | M3 | ✅ |
| `isChecked` | M3 | ✅ |
| `isClosed` | M1 | ✅ |
| `isDisabled` | M3 | ✅ |
| `isEditable` | M3 | ✅ |
| `isEnabled` | M3 | ✅ |
| `isHidden` | M3 | ✅ |
| `isVisible` | M3 | ✅ |
| `locator` | M3 | ✅ |
| `mainFrame` | M1 | ✅ |
| `opener` | M3 | ✅ |
| `pageErrors` | M4 | ✅ |
| `pause` | M6 | ✅ resumed through `context.debugger()` |
| `pdf` | M5 | ✅ |
| `pickLocator` | M6 | ✅ |
| `press` | M3 | ✅ |
| `reload` | M3 | ✅ |
| `removeLocatorHandler` | M3 | ✅ |
| `requestGC` | M6 | ✅ |
| `requests` | M4 | ✅ |
| `route` | M4 | ✅ |
| `routeFromHAR` | M5 | ✅ |
| `routeWebSocket` | M4 | ✅ |
| `screenshot` | M1 | ✅ |
| `selectOption` | M3 | ✅ |
| `setChecked` | M3 | ✅ |
| `setContent` | M1 | ✅ |
| `setDefaultNavigationTimeout` | M3 | ✅ |
| `setDefaultTimeout` | M3 | ✅ |
| `setExtraHTTPHeaders` | M4 | ✅ |
| `setInputFiles` | M3 | ✅ |
| `setViewportSize` | M1 | ✅ |
| `tap` | M3 | ✅ |
| `textContent` | M3 | ✅ |
| `title` | M1 | ✅ |
| `type` | M3 | ✅ |
| `uncheck` | M3 | ✅ |
| `unroute` | M4 | ✅ |
| `unrouteAll` | M4 | ✅ |
| `url` | M1 | ✅ |
| `video` | M5 | ✅ |
| `viewportSize` | M1 | ✅ |
| `waitForEvent` | M3 | ✅ `events()` stream, `wait_for_event(matcher)` and typed waiters (console, pageerror, popup, download, websocket, worker, request, response) |
| `waitForLoadState` | M1 | ✅ |
| `waitForNavigation` | M3 | ✅ |
| `waitForRequest` | M4 | ✅ |
| `waitForResponse` | M4 | ✅ |
| `waitForTimeout` | M3 | ✅ |
| `waitForURL` | M3 | ✅ |
| `workers` | M3 | ✅ |
| `clock` | M6 | ✅ |
| `coverage` | M6 | ✅ |
| `keyboard` | M3 | ✅ |
| `localStorage` | M4 | ✅ |
| `mouse` | M3 | ✅ |
| `request` | M6 | ✅ |
| `screencast` | M5 | ✅ start/stop (frames, `.webm`), overlays, chapters, actions |
| `sessionStorage` | M4 | ✅ |
| `touchscreen` | M3 | ✅ |

### Frame

| Member | Milestone | Status |
|---|---|---|
| `evaluate` | M1 | ✅ |
| `evaluateHandle` | M3 | ✅ |
| `waitForFunction` | M1 | ✅ |
| `waitForSelector` | M3 | ✅ |
| `addScriptTag` | M3 | ✅ |
| `addStyleTag` | M1 | ✅ |
| `check` | M3 | ✅ |
| `childFrames` | M3 | ✅ |
| `click` | M3 | ✅ |
| `content` | M1 | ✅ |
| `dblclick` | M3 | ✅ |
| `dispatchEvent` | M3 | ✅ |
| `dragAndDrop` | M3 | ✅ |
| `fill` | M3 | ✅ |
| `focus` | M3 | ✅ |
| `frameElement` | M3 | ✅ |
| `frameLocator` | M3 | ✅ |
| `getAttribute` | M3 | ✅ |
| `getByAltText` | M3 | ✅ |
| `getByLabel` | M3 | ✅ |
| `getByPlaceholder` | M3 | ✅ |
| `getByRole` | M3 | ✅ |
| `getByTestId` | M3 | ✅ |
| `getByText` | M3 | ✅ |
| `getByTitle` | M3 | ✅ |
| `goto` | M1 | ✅ |
| `hover` | M3 | ✅ |
| `innerHTML` | M3 | ✅ |
| `innerText` | M3 | ✅ |
| `inputValue` | M3 | ✅ |
| `isChecked` | M3 | ✅ |
| `isDetached` | M3 | ✅ |
| `isDisabled` | M3 | ✅ |
| `isEditable` | M3 | ✅ |
| `isEnabled` | M3 | ✅ |
| `isHidden` | M3 | ✅ |
| `isVisible` | M3 | ✅ |
| `locator` | M3 | ✅ |
| `name` | M1 | ✅ |
| `page` | M1 | ✅ |
| `parentFrame` | M3 | ✅ |
| `press` | M3 | ✅ |
| `selectOption` | M3 | ✅ |
| `setChecked` | M3 | ✅ |
| `setContent` | M1 | ✅ |
| `setInputFiles` | M3 | ✅ |
| `tap` | M3 | ✅ |
| `textContent` | M3 | ✅ |
| `title` | M1 | ✅ |
| `type` | M3 | ✅ |
| `uncheck` | M3 | ✅ |
| `url` | M1 | ✅ |
| `waitForLoadState` | M1 | ✅ |
| `waitForNavigation` | M3 | ✅ |
| `waitForTimeout` | M3 | ✅ |
| `waitForURL` | M3 | ✅ |

### Locator

| Member | Milestone | Status |
|---|---|---|
| `evaluate` | M3 | ✅ |
| `evaluateHandle` | M3 | ✅ |
| `evaluateAll` | M3 | ✅ |
| `waitForFunction` | M3 | ✅ |
| `elementHandle` | M3 | ✅ |
| `highlight` | M3 | ✅ |
| `toString` | M3 | ✅ `Display` (Playwright's generator, ported) and async `to_playwright_string()` |
| `all` | M3 | ✅ |
| `allInnerTexts` | M3 | ✅ |
| `allTextContents` | M3 | ✅ |
| `and` | M3 | ✅ |
| `ariaSnapshot` | M5 | ✅ |
| `ariaSnapshotJSON` | M5 | ✅ |
| `blur` | M3 | ✅ |
| `boundingBox` | M3 | ✅ |
| `check` | M3 | ✅ |
| `clear` | M3 | ✅ |
| `click` | M3 | ✅ |
| `contentFrame` | M3 | ✅ |
| `count` | M3 | ✅ |
| `dblclick` | M3 | ✅ |
| `describe` | M3 | ✅ |
| `description` | M3 | ✅ |
| `dispatchEvent` | M3 | ✅ |
| `dragTo` | M3 | ✅ mouse and HTML5 drags |
| `drop` | M3 | ✅ |
| `elementHandles` | M3 | ✅ |
| `fill` | M3 | ✅ |
| `filter` | M3 | ✅ |
| `first` | M3 | ✅ |
| `focus` | M3 | ✅ |
| `frameLocator` | M3 | ✅ |
| `getAttribute` | M3 | ✅ |
| `getByAltText` | M3 | ✅ |
| `getByLabel` | M3 | ✅ |
| `getByPlaceholder` | M3 | ✅ |
| `getByRole` | M3 | ✅ |
| `getByTestId` | M3 | ✅ |
| `getByText` | M3 | ✅ |
| `getByTitle` | M3 | ✅ |
| `hideHighlight` | M3 | ✅ |
| `hover` | M3 | ✅ |
| `innerHTML` | M3 | ✅ |
| `innerText` | M3 | ✅ |
| `inputValue` | M3 | ✅ |
| `isChecked` | M3 | ✅ |
| `isDisabled` | M3 | ✅ |
| `isEditable` | M3 | ✅ |
| `isEnabled` | M3 | ✅ |
| `isHidden` | M3 | ✅ |
| `isVisible` | M3 | ✅ |
| `last` | M3 | ✅ |
| `locator` | M3 | ✅ |
| `normalize` | M3 | ✅ |
| `nth` | M3 | ✅ |
| `or` | M3 | ✅ |
| `page` | M3 | ✅ |
| `press` | M3 | ✅ |
| `pressSequentially` | M3 | ✅ |
| `screenshot` | M3 | ✅ |
| `scrollIntoViewIfNeeded` | M3 | ✅ |
| `selectOption` | M3 | ✅ |
| `selectText` | M3 | ✅ |
| `setChecked` | M3 | ✅ |
| `setInputFiles` | M3 | ✅ |
| `tap` | M3 | ✅ |
| `textContent` | M3 | ✅ |
| `type` | M3 | ✅ |
| `uncheck` | M3 | ✅ |
| `visible` | M3 | ✅ |
| `waitFor` | M3 | ✅ |

### ElementHandle

| Member | Milestone | Status |
|---|---|---|
| `waitForSelector` | M3 | ✅ |
| `boundingBox` | M3 | ✅ |
| `check` | M3 | ✅ |
| `click` | M3 | ✅ |
| `contentFrame` | M3 | ✅ |
| `dblclick` | M3 | ✅ |
| `dispatchEvent` | M3 | ✅ |
| `fill` | M3 | ✅ |
| `focus` | M3 | ✅ |
| `getAttribute` | M3 | ✅ |
| `hover` | M3 | ✅ |
| `innerHTML` | M3 | ✅ |
| `innerText` | M3 | ✅ |
| `inputValue` | M3 | ✅ |
| `isChecked` | M3 | ✅ |
| `isDisabled` | M3 | ✅ |
| `isEditable` | M3 | ✅ |
| `isEnabled` | M3 | ✅ |
| `isHidden` | M3 | ✅ |
| `isVisible` | M3 | ✅ |
| `ownerFrame` | M3 | ✅ |
| `press` | M3 | ✅ |
| `screenshot` | M3 | ✅ |
| `scrollIntoViewIfNeeded` | M3 | ✅ |
| `selectOption` | M3 | ✅ |
| `selectText` | M3 | ✅ |
| `setChecked` | M3 | ✅ |
| `setInputFiles` | M3 | ✅ |
| `tap` | M3 | ✅ |
| `textContent` | M3 | ✅ |
| `type` | M3 | ✅ |
| `uncheck` | M3 | ✅ |
| `waitForElementState` | M3 | ✅ |

### JSHandle

| Member | Milestone | Status |
|---|---|---|
| `evaluate` | M3 | ✅ |
| `evaluateHandle` | M3 | ✅ |
| `jsonValue` | M3 | ✅ |
| `asElement` | M3 | ✅ |
| `dispose` | M3 | ✅ |
| `getProperties` | M3 | ✅ |
| `getProperty` | M3 | ✅ |

### Keyboard

| Member | Milestone | Status |
|---|---|---|
| `down` | M3 | ✅ |
| `insertText` | M3 | ✅ |
| `press` | M3 | ✅ |
| `type` | M3 | ✅ |
| `up` | M3 | ✅ |

### Mouse

| Member | Milestone | Status |
|---|---|---|
| `click` | M3 | ✅ |
| `dblclick` | M3 | ✅ |
| `down` | M3 | ✅ |
| `move` | M3 | ✅ |
| `up` | M3 | ✅ |
| `wheel` | M3 | ✅ |

### Touchscreen

| Member | Milestone | Status |
|---|---|---|
| `tap` | M3 | ✅ |

### Request

| Member | Milestone | Status |
|---|---|---|
| `allHeaders` | M4 | ✅ |
| `existingResponse` | M4 | ✅ |
| `failure` | M4 | ✅ |
| `frame` | M4 | ✅ |
| `headers` | M4 | ✅ |
| `headersArray` | M4 | ✅ |
| `headerValue` | M4 | ✅ |
| `isNavigationRequest` | M4 | ✅ |
| `method` | M4 | ✅ |
| `postData` | M4 | ✅ |
| `postDataBuffer` | M4 | ✅ |
| `postDataJSON` | M4 | ✅ |
| `redirectedFrom` | M4 | ✅ |
| `redirectedTo` | M4 | ✅ |
| `resourceType` | M4 | ✅ |
| `response` | M4 | ✅ |
| `serviceWorker` | M4 | ✅ |
| `sizes` | M4 | ✅ |
| `timing` | M4 | ✅ |
| `url` | M4 | ✅ |

### Response

| Member | Milestone | Status |
|---|---|---|
| `allHeaders` | M4 | ✅ |
| `body` | M4 | ✅ |
| `finished` | M4 | ✅ |
| `frame` | M4 | ✅ |
| `fromServiceWorker` | M4 | ✅ |
| `headers` | M4 | ✅ |
| `headersArray` | M4 | ✅ |
| `headerValue` | M4 | ✅ |
| `headerValues` | M4 | ✅ |
| `httpVersion` | M4 | ✅ |
| `json` | M4 | ✅ |
| `ok` | M4 | ✅ |
| `request` | M4 | ✅ |
| `securityDetails` | M4 | ✅ `None` for plain HTTP |
| `serverAddr` | M4 | ✅ |
| `status` | M4 | ✅ |
| `statusText` | M4 | ✅ |
| `text` | M4 | ✅ |
| `url` | M4 | ✅ |

### Route

| Member | Milestone | Status |
|---|---|---|
| `abort` | M4 | ✅ |
| `continue` | M4 | ✅ |
| `fallback` | M4 | ✅ |
| `fetch` | M4 | ✅ via the browser (Fetch response stage) |
| `fulfill` | M4 | ✅ status, headers, contentType, body, json, path, response |
| `request` | M4 | ✅ |

### WebSocket

| Member | Milestone | Status |
|---|---|---|
| `isClosed` | M4 | ✅ |
| `url` | M4 | ✅ |
| `waitForEvent` | M4 | ✅ `events()` (replays from the start) and `wait_for_close()` |

### WebSocketRoute

| Member | Milestone | Status |
|---|---|---|
| `onMessage` | M4 | ✅ |
| `onClose` | M4 | ✅ |
| `close` | M4 | ✅ |
| `connectToServer` | M4 | ✅ |
| `protocols` | M4 | ✅ |
| `send` | M4 | ✅ |
| `url` | M4 | ✅ |

### Download

| Member | Milestone | Status |
|---|---|---|
| `cancel` | M4 | ✅ |
| `createReadStream` | M4, M8 | ✅ `create_read_stream()` (`AsyncRead`); also `read()` (bytes) |
| `delete` | M4 | ✅ |
| `failure` | M4 | ✅ |
| `page` | M4 | ✅ |
| `path` | M4 | ✅ |
| `saveAs` | M4 | ✅ |
| `suggestedFilename` | M4 | ✅ |
| `url` | M4 | ✅ |

### Dialog

| Member | Milestone | Status |
|---|---|---|
| `accept` | M4 | ✅ |
| `defaultValue` | M4 | ✅ |
| `dismiss` | M4 | ✅ |
| `message` | M4 | ✅ |
| `page` | M4 | ✅ |
| `type` | M4 | ✅ |

### FileChooser

| Member | Milestone | Status |
|---|---|---|
| `element` | M4 | ✅ |
| `isMultiple` | M4 | ✅ |
| `page` | M4 | ✅ |
| `setFiles` | M4 | ✅ |

### ConsoleMessage

| Member | Milestone | Status |
|---|---|---|
| `args` | M4 | ✅ |
| `location` | M4 | ✅ |
| `page` | M4 | ✅ |
| `text` | M4 | ✅ |
| `timestamp` | M4 | ✅ |
| `type` | M4 | ✅ |
| `worker` | M4 | ✅ |

### Worker

| Member | Milestone | Status |
|---|---|---|
| `evaluate` | M4 | ✅ |
| `evaluateHandle` | M4 | ✅ dedicated workers |
| `url` | M4 | ✅ |
| `waitForEvent` | M4 | ✅ `wait_for_close()`; worker console goes to page console events |

### Tracing

| Member | Milestone | Status |
|---|---|---|
| `group` | M5 | ✅ |
| `groupEnd` | M5 | ✅ |
| `start` | M5 | ✅ |
| `startChunk` | M5 | ✅ |
| `startHar` | M5 | ✅ |
| `stop` | M5 | ✅ |
| `stopChunk` | M5 | ✅ |
| `stopHar` | M5 | ✅ |

### Video

| Member | Milestone | Status |
|---|---|---|
| `delete` | M5 | ✅ |
| `path` | M5 | ✅ |
| `saveAs` | M5 | ✅ |

### Clock

| Member | Milestone | Status |
|---|---|---|
| `fastForward` | M6 | ✅ |
| `install` | M6 | ✅ |
| `pauseAt` | M6 | ✅ |
| `resume` | M6 | ✅ |
| `runFor` | M6 | ✅ |
| `setFixedTime` | M6 | ✅ |
| `setSystemTime` | M6 | ✅ |

### Coverage

| Member | Milestone | Status |
|---|---|---|
| `startCSSCoverage` | M6 | ✅ |
| `startJSCoverage` | M6 | ✅ |
| `stopCSSCoverage` | M6 | ✅ |
| `stopJSCoverage` | M6 | ✅ |

### CDPSession

| Member | Milestone | Status |
|---|---|---|
| `send` | M2 | ✅ |
| `detach` | M3 | ✅ |

### APIRequestContext

| Member | Milestone | Status |
|---|---|---|
| `delete` | M6 | ✅ |
| `dispose` | M6 | ✅ |
| `fetch` | M6 | ✅ |
| `get` | M6 | ✅ |
| `head` | M6 | ✅ |
| `patch` | M6 | ✅ |
| `post` | M6 | ✅ |
| `put` | M6 | ✅ |
| `storageState` | M6 | ✅ |
| `tracing` | M6 | ✅ |

### APIResponse

| Member | Milestone | Status |
|---|---|---|
| `json` | M6 | ✅ |
| `body` | M6 | ✅ |
| `dispose` | M6 | ✅ |
| `headers` | M6 | ✅ |
| `headersArray` | M6 | ✅ |
| `ok` | M6 | ✅ |
| `securityDetails` | M6 | ✅ |
| `serverAddr` | M6 | ✅ |
| `status` | M6 | ✅ |
| `statusText` | M6 | ✅ |
| `text` | M6 | ✅ |
| `timing` | M6 | ✅ |
| `url` | M6 | ✅ |

### BrowserType

| Member | Milestone | Status |
|---|---|---|
| `connectOverCDP` | M1 | ✅ |
| `connect` | M6 | ✅ to a `launch_server` (CDP endpoint, not Playwright's protocol) |
| `executablePath` | M1 | ✅ |
| `launch` | M1 | ✅ |
| `launchPersistentContext` | M3 | ✅ |
| `launch({ ignoreDefaultArgs: [...] })` | M12 | ✅ `LaunchOptions::ignored_default_args` (exact switches; `ignoreDefaultArgs: true` is `ignore_default_args`) |
| `launchServer` | M6 | ✅ `BrowserServer` (CDP endpoint) |
| `name` | M1 | ✅ |

### Selectors

| Member | Milestone | Status |
|---|---|---|
| `register` | M3 | ✅ |
| `setTestIdAttribute` | M3 | ✅ |

## 7. Playwright 1.64 and 1.65 (M7)

From the 1.64.0, 1.64.1, 1.65.0 and 1.65.1 release notes and a diff of
`types.d.ts` 1.63.0 -> 1.65.1. Expected values recorded with Playwright 1.65.1.

| Item | Milestone | Status |
|---|---|---|
| Vendored injected, clock, storage, snapshot and WebMCP scripts from playwright-core 1.65.1 | M7 | ✅ |
| Elements inside hidden iframes are hidden (actions wait, `isVisible`, `toBeVisible`) | M7 | ✅ |
| Actions wait until the element is not inert ("element is inert") | M7 | ✅ |
| Unrendered text left out of accessible names and aria snapshots | M7 | ✅ |
| `ariaSnapshot({ mode: 'ai', depth, boxes })` (page and locator; iframes merged) | M7 | ✅ |
| `page.getByRef(ref)` (refs in child frames: `f1e3`) | M7 | ✅ |
| `locator.within(locator)` | M7 | ✅ |
| `page.content({ includeShadow })`, `frame.content({ includeShadow })` | M7 | ✅ |
| `page.webmcp` / `frame.webmcp`: `tools()`, `callTool(name, input)` | M7 | ✅ |
| `apiRequestContext.addCookies()`, `cookies()`, `clearCookies()` | M7 | ✅ |
| RFC 6265 cookie matching for `cookies(urls)`; nameless cookies | M7 | ✅ |
| `params` appended to an existing URL query | M7 | ✅ (already) |
| `proxy` option (launch and context); proxy credentials only for proxy challenges | M7 | ✅ |
| `recordVideo.fps`, `screencast.start({ fps })`; VP9 videos (VP8 if ffmpeg lacks VP9) | M7 | ✅ |
| `screencast.showActions({ style })`; `fontSize` folded into the title style | M7 | ✅ |
| Action cursor rests at the last point and survives navigations | M7 | ✅ |
| `credentials.create({ signCount })`, `signCount` on create/get | M7 | ✅ |
| `cdpSession.send(method, params, { timeout })` | M7 | ✅ |
| Device descriptors from 1.65.1 | M7 | ✅ |
| Default launch switches from 1.65.1 | M7 | ✅ |
| Trace format 10 | M7 | ✅ |
| Device `screen` forwarded to contexts | M7 | ✅ (already) |
| `signal` (AbortSignal) options | M7 | n/a: drop the Rust future to cancel |
| `Disposable` return values (`await using`) | M7 | n/a: no async drop in Rust; call the matching stop/remove |
| Test runner: reporters, `--shuffle`, locks, `testProject.default`, `toHaveScreenshot.type`, Bun, JSX | M7 | out of scope (test runner) |

## 8. Backlog: options, events and methods (M8)

From the audit of every 1.65.1 method, option and event of the
Chromium-relevant classes ([`BACKLOG.md`](BACKLOG.md)). Tests:
`tests/integration/backlog.rs`, `tests/integration/client_certs.rs`.

| Item | Milestone | Status |
|---|---|---|
| `delay` on click, dblclick, press, type, pressSequentially; `keyboard.press/type`, `mouse.click/dblclick` | M8 | ✅ |
| `clientCertificates` (`cert`/`key` PEM or DER, encrypted keys + `passphrase`, `pfx`) on contexts, `newPage`, `launchPersistentContext`, `request.newContext` | M8 | ✅ requests to the origin go through the HTTP client (Playwright: a local proxy) |
| `strictSelectors` context option | M8 | ✅ |
| `page.$`, `$$`, `$eval`, `$$eval` (page, frame, element handle) | M8 | ✅ |
| Selector shortcuts (`page.click(selector)`, ...) use the first match unless `strictSelectors` | M8 | ✅ (were always strict) |
| `maxRetries` on `request.fetch/get/post/...` and `route.fetch` | M8 | ✅ |
| `ignoreHTTPSErrors` per request | M8 | ✅ |
| `storageState({ indexedDB, opfs })` (context and request) | M8 | ✅ |
| `routeFromHAR({ update, updateContent, updateMode })` | M8 | ✅ |
| `tracing.startHar(path, { content, mode, resourcesDir, urlFilter })`; `.zip` HARs | M8 | ✅ |
| `page.close({ runBeforeUnload })` | M8 | ✅ |
| Launch `slowMo`, `downloadsPath`, `tracesDir`, `artifactsDir`, `chromiumSandbox` | M8 | ✅ default command line unchanged |
| Launch `handleSIGINT` / `handleSIGTERM` / `handleSIGHUP` | M8 | ✅ opt-in (Playwright: on by default) |
| `launchServer({ host, port, wsPath })` | M8 | ✅ |
| `browser.bind(title, { host, port, workspaceDir, metadata })`; bound-browser registry | M8 | ✅ `bind_with`, `Chromium::bound_browsers()` |
| `selectors.register(name, script, { contentScript })`; engines run in the main world otherwise | M8 | ✅ (all engines ran isolated before) |
| `debugger.runTo(location)`, `pausedDetails().location` | M8 | ✅ needs debug info |
| `browserServer.process()` | M8 | ✅ pid |
| `download.createReadStream()` | M8 | ✅ `AsyncRead` |
| Page events `crash`, `dialogclosed`, `frameattached`, `framedetached`, `framenavigated` | M8 | ✅ |
| Context events `dialogclosed`, `download`, `frameattached`, `framedetached`, `framenavigated`, `pageclose`, `pageload`, `backgroundpage` | M8 | ✅ (`backgroundpage` never fires in Chromium) |
| Browser events `context`, `disconnected` | M8 | ✅ `browser.events()` |
| Blocked-click log names the element (`<body>…</body> intercepts pointer events`) | M8 | ✅ |

## 9. Playwright 1.66 alpha (M9)

From the 1.66.0 alpha of 2026-11-22 (not released yet; re-check against the
release notes when it ships).

| Item | Milestone | Status |
|---|---|---|
| `locateOnlyVisible` context option (also `newPage`, `launchPersistentContext`) | M9 | ✅ selector rewrite checked against the alpha; selector shortcuts, `$` and element handles unaffected |
| Chromium 159.0.8154.13 | M9 | ✅ pixel-parity test passes on Chrome for Testing 159.0.8154.13 |

## 10. Electron (M10)

Playwright's `_electron` (`tests/integration/electron.rs`, under Xvfb).
Goal: [`goals/electron-apps-are-automatable.md`](../goals/electron-apps-are-automatable.md).

| Item | Milestone | Status |
|---|---|---|
| `electron.launch({ executablePath, args, cwd, env, timeout, chromiumSandbox, ...context options })` | M10 | ✅ `Electron::launch`; Playwright's loader when Electron is found like `require('electron')` |
| `electronApp.firstWindow({ timeout })`, `windows()`, `on('window')` | M10 | ✅ |
| `electronApp.context()` | M10 | ✅ no default viewport, as in Playwright |
| `electronApp.evaluate(fn, arg)`, `evaluateHandle` | M10 | ✅ `fn(electron, arg)` in the main process; `ElectronHandle` |
| `electronApp.browserWindow(page)` | M10 | ✅ |
| `electronApp.on('console')` | M10 | ✅ `ElectronConsoleMessage` |
| `electronApp.process()`, `close()`, `on('close')` | M10 | ✅ |
| `waitForEvent` | M10 | ✅ through `events()` |
| Closing one window with `webContents.close()` | M10 | out of scope for now: `page.close()` closes the target |

## 11. Android (M11)

Playwright's `_android` (`tests/integration/android.rs` against the fake adb
server in `tests/integration/fake_adb.rs`; `examples/android_device.rs` on a
real device or emulator). Goal:
[`goals/android-devices-are-automatable.md`](../goals/android-devices-are-automatable.md).

| Item | Milestone | Status |
|---|---|---|
| `android.devices({ host, port, omitDriverInstall })` | M11 | ✅ `Android::devices`; talks to the adb server directly; `omitDriverInstall` changes nothing (no driver) |
| `androidDevice.serial()`, `model()` | M11 | ✅ |
| `androidDevice.shell(command)` | M11 | ✅ |
| `androidDevice.open(command)`, `AndroidSocket` (`write`, `on('data')`, `close`) | M11 | ✅ |
| `androidDevice.push(file, path, { mode })` | M11 | ✅ `sync:` protocol |
| `androidDevice.installApk(file, { args })` | M11 | ✅ |
| `androidDevice.screenshot()` | M11 | ✅ |
| `androidDevice.launchBrowser({ pkg, args, ...context options })` | M11 | ✅ Playwright's command line and DevTools socket; checked on a real emulator (Chrome 145). On non-rooted devices Chrome reads that command line only with chrome://flags "Enable command line on non-rooted devices" on (same as Playwright). Screenshots of zoomed-out pages (no `<meta name=viewport>`) capture the screen, not a 3x3 tiling (M12 fix) |
| `androidDevice.webViews()`, `webView({ pkg })`, `on('webview')`, `AndroidWebView.page()`, `pid()`, `pkg()` | M11 | ✅ |
| `androidDevice.close()`, `on('close')` | M11 | ✅ |
| `android.connect`, `android.launchServer` | M11 | out of scope: Playwright's own client-server protocol; use `AndroidDevicesOptions { host, port }` for a remote adb server |
| `tap`, `fill`, `press`, `drag`, `fling`, `longTap`, `pinchOpen/Close`, `scroll`, `swipe`, `wait`, `info`, `input.*` | M11 | out of scope for now: they need Playwright's UI-automator driver APK installed on the device |

## 12. Every CDP domain (M12)

Every non-deprecated command and event of protocol `r1751165` has a typed method, generated from the PDL by `chromiumoxide_pdl` into `chromiumoxide_cdp::cdp::domains`: `page.protocol().<domain>().<command>(params)` returns the typed result; `.on_<event>()` is a typed stream. `browser.protocol()` does the same on the browser target. The coverage check (`chromiumoxide_cdp/tests/domains.rs`) fails when a command or event has no method, or a domain has no `cdp_<domain>` test in `tests/integration/cdp_domains.rs`.

Totals: 57 domains, 634 commands, 224 events, all with a typed method.

Deprecated, raw types only (`page.execute` / `page.cdp_events`), by design: the domains Console and Schema, and every command or event marked deprecated:

- Console: (whole domain: clearMessages, disable, enable, messageAdded)
- Debugger: `getWasmBytecode`, `pauseOnAsyncCall`, `setScriptSource`, `breakpointResolved`
- Schema: (whole domain: getDomains)
- Browser: `grantPermissions`
- CSS: `setContainerQueryText`
- DOM: `getFlattenedDocument`
- DOMDebugger: `removeInstrumentationBreakpoint`, `setInstrumentationBreakpoint`
- DOMSnapshot: `getSnapshot`
- Emulation: `canEmulate`, `setNavigatorOverrides`, `setVisibleSize`
- HeadlessExperimental: `disable`, `enable`
- IO: `resolveBlob`
- Network: `canClearBrowserCache`, `canClearBrowserCookies`, `canEmulateNetworkConditions`, `emulateNetworkConditions`, `getAllCookies`
- Overlay: `highlightFrame`, `setShowHitTestBorders`, `setShowWebVitals`
- Page: `addScriptToEvaluateOnLoad`, `clearDeviceMetricsOverride`, `clearDeviceOrientationOverride`, `clearGeolocationOverride`, `deleteCookie`, `getManifestIcons`, `removeScriptToEvaluateOnLoad`, `setDeviceMetricsOverride`, `setDeviceOrientationOverride`, `setDownloadBehavior`, `setGeolocationOverride`, `setTouchEmulationEnabled`, `frameClearedScheduledNavigation`, `frameScheduledNavigation`, `downloadWillBegin`, `downloadProgress`
- Performance: `setTimeDomain`
- Security: `handleCertificateError`, `setOverrideCertificateErrors`, `certificateError`, `securityStateChanged`
- Storage: `getStorageKeyForFrame`
- Target: `sendMessageToTarget`

### CDP Accessibility

- [x] `protocol().accessibility()`: 8 commands, 2 events (experimental domain).
- [x] Test: `cdp_accessibility` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Ads

- [x] `protocol().ads()`: 2 commands, 0 events (experimental domain).
- [x] Test: `cdp_ads` in `tests/integration/cdp_domains.rs`, on the page target.
- Note: The test checks Chromium's answer.

### CDP Animation

- [x] `protocol().animation()`: 10 commands, 4 events (experimental domain).
- [x] Test: `cdp_animation` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Audits

- [x] `protocol().audits()`: 4 commands, 1 events (experimental domain).
- [x] Test: `cdp_audits` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Autofill

- [x] `protocol().autofill()`: 4 commands, 1 events (experimental domain).
- [x] Test: `cdp_autofill` in `tests/integration/cdp_domains.rs`, on the page (full Chrome) target.
- Note: Full Chrome only (not in the headless shell).

### CDP BackgroundService

- [x] `protocol().background_service()`: 4 commands, 2 events (experimental domain).
- [x] Test: `cdp_background_service` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP BluetoothEmulation

- [x] `protocol().bluetooth_emulation()`: 16 commands, 3 events (experimental domain).
- [x] Test: `cdp_bluetooth_emulation` in `tests/integration/cdp_domains.rs`, on the browser (full Chrome) target.
- Note: Full Chrome only, on the browser target.

### CDP Browser

- [x] `protocol().browser()`: 20 commands, 2 events.
- [x] Test: `cdp_browser` in `tests/integration/cdp_domains.rs`, on the browser target.

### CDP CSS

- [x] `protocol().css()`: 39 commands, 6 events (experimental domain).
- [x] Test: `cdp_css` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP CacheStorage

- [x] `protocol().cache_storage()`: 5 commands, 0 events (experimental domain).
- [x] Test: `cdp_cache_storage` in `tests/integration/cdp_domains.rs`, on the page target.
- Note: The test checks Chromium's answer.

### CDP Cast

- [x] `protocol().cast()`: 6 commands, 2 events (experimental domain).
- [x] Test: `cdp_cast` in `tests/integration/cdp_domains.rs`, on the page (full Chrome) target.
- Note: Full Chrome only. Playwright's default arguments disable Media Router, so Chrome answers `Cast.enable` with "You must enable the Media Router feature"; the test checks that answer.

### CDP CrashReportContext

- [x] `protocol().crash_report_context()`: 1 commands, 0 events (experimental domain).
- [x] Test: `cdp_crash_report_context` in `tests/integration/cdp_domains.rs`, on the page target.
- Note: The test checks Chromium's answer.

### CDP DOM

- [x] `protocol().dom()`: 56 commands, 19 events.
- [x] Test: `cdp_dom` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP DOMDebugger

- [x] `protocol().dom_debugger()`: 8 commands, 0 events.
- [x] Test: `cdp_dom_debugger` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP DOMSnapshot

- [x] `protocol().dom_snapshot()`: 3 commands, 0 events (experimental domain).
- [x] Test: `cdp_dom_snapshot` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP DOMStorage

- [x] `protocol().dom_storage()`: 6 commands, 4 events (experimental domain).
- [x] Test: `cdp_dom_storage` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Debugger

- [x] `protocol().debugger()`: 31 commands, 4 events.
- [x] Test: `cdp_debugger` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP DeviceAccess

- [x] `protocol().device_access()`: 4 commands, 1 events (experimental domain).
- [x] Test: `cdp_device_access` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP DeviceOrientation

- [x] `protocol().device_orientation()`: 2 commands, 0 events (experimental domain).
- [x] Test: `cdp_device_orientation` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP DigitalCredentials

- [x] `protocol().digital_credentials()`: 1 commands, 0 events (experimental domain).
- [x] Test: `cdp_digital_credentials` in `tests/integration/cdp_domains.rs`, on the page target.
- Note: The test checks Chromium's answer.

### CDP Emulation

- [x] `protocol().emulation()`: 46 commands, 2 events.
- [x] Test: `cdp_emulation` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP EventBreakpoints

- [x] `protocol().event_breakpoints()`: 3 commands, 0 events (experimental domain).
- [x] Test: `cdp_event_breakpoints` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Extensions

- [x] `protocol().extensions()`: 8 commands, 0 events (experimental domain).
- [x] Test: `cdp_extensions` in `tests/integration/cdp_domains.rs`, on the browser (full Chrome, `--enable-unsafe-extension-debugging`) target.
- Note: Full Chrome only, with `--enable-unsafe-extension-debugging` and without the default `--disable-extensions` (`LaunchOptions::ignored_default_args`); the test loads, lists and uninstalls an unpacked extension.

### CDP FedCm

- [x] `protocol().fed_cm()`: 7 commands, 2 events (experimental domain).
- [x] Test: `cdp_fed_cm` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Fetch

- [x] `protocol().fetch()`: 9 commands, 2 events.
- [x] Test: `cdp_fetch` in `tests/integration/cdp_domains.rs`, on the browser (on a page, `page.route` owns Fetch) target.
- Note: On a page, `Fetch` belongs to `page.route` / `context.route`, which continue every request they are not asked to handle; drive the raw domain on the browser target (as the test does) or use routing.

### CDP FileSystem

- [x] `protocol().file_system()`: 1 commands, 0 events (experimental domain).
- [x] Test: `cdp_file_system` in `tests/integration/cdp_domains.rs`, on the page target.
- Note: Chromium does not answer `getDirectory` while the origin has no private file system yet; the test writes a file through `navigator.storage.getDirectory()` first and reads it back.

### CDP FindInPage

- [x] `protocol().find_in_page()`: 4 commands, 0 events (experimental domain).
- [x] Test: `cdp_find_in_page` in `tests/integration/cdp_domains.rs`, on the page target.
- Note: The test checks Chromium's answer.

### CDP HeadlessExperimental

- [x] `protocol().headless_experimental()`: 1 commands, 0 events (experimental domain).
- [x] Test: `cdp_headless_experimental` in `tests/integration/cdp_domains.rs`, on the page target.
- Note: `beginFrame` needs `--enable-begin-frame-control` at launch; the test checks Chromium's answer.

### CDP HeapProfiler

- [x] `protocol().heap_profiler()`: 12 commands, 5 events (experimental domain).
- [x] Test: `cdp_heap_profiler` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP IO

- [x] `protocol().io()`: 2 commands, 0 events.
- [x] Test: `cdp_io` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP IndexedDB

- [x] `protocol().indexed_db()`: 9 commands, 0 events (experimental domain).
- [x] Test: `cdp_indexed_db` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Input

- [x] `protocol().input()`: 13 commands, 1 events.
- [x] Test: `cdp_input` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Inspector

- [x] `protocol().inspector()`: 2 commands, 4 events (experimental domain).
- [x] Test: `cdp_inspector` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP LayerTree

- [x] `protocol().layer_tree()`: 9 commands, 2 events (experimental domain).
- [x] Test: `cdp_layer_tree` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Log

- [x] `protocol().log()`: 5 commands, 1 events.
- [x] Test: `cdp_log` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Media

- [x] `protocol().media()`: 2 commands, 5 events (experimental domain).
- [x] Test: `cdp_media` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Memory

- [x] `protocol().memory()`: 11 commands, 0 events (experimental domain).
- [x] Test: `cdp_memory` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Network

- [x] `protocol().network()`: 30 commands, 43 events.
- [x] Test: `cdp_network` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Overlay

- [x] `protocol().overlay()`: 28 commands, 6 events (experimental domain).
- [x] Test: `cdp_overlay` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP PWA

- [x] `protocol().pwa()`: 7 commands, 0 events (experimental domain).
- [x] Test: `cdp_pwa` in `tests/integration/cdp_domains.rs`, on the browser (full Chrome) target.
- Note: Full Chrome only, on the browser target (Chrome 159 crashes when it is sent to a page session); the test checks the "Unknown web-app manifest id" answer.

### CDP Page

- [x] `protocol().page()`: 55 commands, 24 events.
- [x] Test: `cdp_page` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Performance

- [x] `protocol().performance()`: 3 commands, 1 events.
- [x] Test: `cdp_performance` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP PerformanceTimeline

- [x] `protocol().performance_timeline()`: 1 commands, 1 events (experimental domain).
- [x] Test: `cdp_performance_timeline` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Preload

- [x] `protocol().preload()`: 2 commands, 6 events (experimental domain).
- [x] Test: `cdp_preload` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Profiler

- [x] `protocol().profiler()`: 9 commands, 3 events.
- [x] Test: `cdp_profiler` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Runtime

- [x] `protocol().runtime()`: 23 commands, 8 events.
- [x] Test: `cdp_runtime` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP Security

- [x] `protocol().security()`: 3 commands, 1 events.
- [x] Test: `cdp_security` in `tests/integration/cdp_domains.rs`, on the page (full Chrome) target.
- Note: `visibleSecurityStateChanged` comes from full Chrome only (the headless shell has only the content-level handler); the test runs on full Chrome.

### CDP ServiceWorker

- [x] `protocol().service_worker()`: 12 commands, 3 events (experimental domain).
- [x] Test: `cdp_service_worker` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP SmartCardEmulation

- [x] `protocol().smart_card_emulation()`: 12 commands, 14 events (experimental domain).
- [x] Test: `cdp_smart_card_emulation` in `tests/integration/cdp_domains.rs`, on the page (full Chrome) target.
- Note: **Not served by Chrome for Testing 159 on Linux** (Chromium answers "wasn't found"; the Web Smart Card API ships on ChromeOS). The client methods exist and are typed; the test pins the current answer and fails when a Linux build starts serving the domain.

### CDP Storage

- [x] `protocol().storage()`: 26 commands, 7 events (experimental domain).
- [x] Test: `cdp_storage` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP SystemInfo

- [x] `protocol().system_info()`: 3 commands, 0 events (experimental domain).
- [x] Test: `cdp_system_info` in `tests/integration/cdp_domains.rs`, on the browser target.

### CDP Target

- [x] `protocol().target()`: 18 commands, 7 events.
- [x] Test: `cdp_target` in `tests/integration/cdp_domains.rs`, on the browser target.

### CDP Tethering

- [x] `protocol().tethering()`: 2 commands, 1 events (experimental domain).
- [x] Test: `cdp_tethering` in `tests/integration/cdp_domains.rs`, on the browser (WebSocket connection) target.
- Note: Served on the browser target only when connected over a WebSocket (`Channel::WebSocket`), not over the default pipe.

### CDP Tracing

- [x] `protocol().tracing()`: 6 commands, 3 events.
- [x] Test: `cdp_tracing` in `tests/integration/cdp_domains.rs`, on the browser target.

### CDP WebAudio

- [x] `protocol().web_audio()`: 3 commands, 13 events (experimental domain).
- [x] Test: `cdp_web_audio` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP WebAuthn

- [x] `protocol().web_authn()`: 13 commands, 4 events (experimental domain).
- [x] Test: `cdp_web_authn` in `tests/integration/cdp_domains.rs`, on the page target.

### CDP WebMCP

- [x] `protocol().web_mcp()`: 4 commands, 4 events (experimental domain).
- [x] Test: `cdp_web_mcp` in `tests/integration/cdp_domains.rs`, on the page target.
- Note: The test checks Chromium's answer.

## 13. Firefox (M13)

`chromiumoxide::firefox` drives Playwright's Firefox build over WebDriver
BiDi, with the Playwright method names. Tests are in
`tests/integration/firefox.rs` and `tests/integration/firefox_parity.rs`,
against `firefox-1572` (Firefox 159). ✅ works on Firefox (tested) ·
❌ not possible over BiDi in Firefox 159 (reason given) · ⬜ not done yet on
Firefox.

| Area | Firefox | Test / reason |
|---|---|---|
| `firefox.launch` (Playwright's arguments, `executablePath`, `headless`, `args`, `env`, `timeout`) | ✅ | `firefox_launches_and_runs_a_page` |
| `browser.version`, `newPage`, `newContext`, `close` | ✅ | `firefox_launches_and_runs_a_page`, `firefox_contexts_emulate_devices` |
| Context emulation: `viewport`, `deviceScaleFactor`, `userAgent`, `locale`, `timezoneId`, `screen`, `geolocation`, `extraHTTPHeaders`, `ignoreHTTPSErrors` | ✅ | `firefox_contexts_emulate_devices` |
| Context options `isMobile`, `hasTouch`, `colorScheme` (dark / no-preference), `reducedMotion`, `forcedColors`, `contrast`, `javaScriptEnabled: false`, `offline`, `permissions` | ❌ | No BiDi command in Firefox 159 (Playwright uses its Juggler protocol); creating such a context is an error naming the options |
| Context options `bypassCSP`, `proxy`, `clientCertificates`, `baseURL`, `serviceWorkers` | ⬜ | An error naming the option for now |
| `recordHar` (bodies from a BiDi network data collector; `content`, `mode`, `urlFilter`, `.zip`) | ✅ | `firefox_pdf_and_har` |
| `recordVideo` (viewport screenshots on a 25 fps timeline, scaled into the `.webm`) | ✅ | `firefox_records_video` |
| `httpCredentials` | ✅ | `firefox_network_events_context_routes_and_auth` |
| `storageState` (option) and `context.storageState()` | ✅ | `firefox_storage_state_round_trip` |
| `page.goto`, `setContent`, `title`, `url`, `content`, `reload`, `goBack`, `goForward` | ✅ | `firefox_launches_and_runs_a_page`, `firefox_navigates_and_waits` |
| `page.evaluate` / `evaluate(fn, arg)` | ✅ | `firefox_launches_and_runs_a_page` |
| `page.addStyleTag`, `setViewportSize`, `viewportSize` | ✅ | `firefox_contexts_emulate_devices` |
| `page.screenshot` (viewport, clip, fullPage, JPEG, caret, animations, style) | ✅ | `firefox_screenshots_are_pixel_identical_to_playwright` (pixel-identical to Playwright's Firefox) |
| `page.screenshot` `omitBackground`, `scale: "css"` at a device scale factor above 1, WebP, `mask` | ⬜ | An error for now (BiDi has no background or scale option) |
| `page.waitForFunction`, `waitForTimeout`, `locator.waitFor` | ✅ | `firefox_navigates_and_waits` |
| `page.waitForURL`, `waitForLoadState`, `waitForEvent`, `waitForRequest` / `Response` | ⬜ | |
| Locators: `locator`, `getBy*`, `nth` / `first` / `last`, chaining, `textContent`, `innerText`, `innerHTML`, `getAttribute`, `inputValue`, `count`, `allTextContents`, `allInnerTexts`, `isVisible`, `isHidden`, `isChecked`, strictness | ✅ | `firefox_locators_query_the_page` |
| Locator `filter`, `and`, `or`, `all`, `boundingBox`, `evaluate`, `ariaSnapshot`, `screenshot`, `highlight` | ⬜ | |
| Input: `click`, `hover`, `fill`, `focus`, `press`, `pressSequentially`, `check`, `uncheck` | ✅ | `firefox_locators_act_on_elements` |
| Input: `dblclick`, `dragTo`, `selectOption`, `setInputFiles`, `page.mouse` (`move`, `down`, `up`, `click`, `dblclick`, `wheel`), `page.keyboard` (`down`, `up`, `press`, `type`, `insertText`) | ✅ | `firefox_more_input`. Firefox's WebDriver actions start a drag but end it without a `drop`, so `dragTo` finishes the drop in the page the way the browser does |
| Input: `tap` | ❌ | `hasTouch` cannot be set over BiDi in Firefox 159 |
| `expect()` locator assertions (state, text, attribute, class, count, CSS, id, role, value) and page `toHaveTitle` / `toHaveURL` | ✅ | `firefox_expect_assertions` |
| Events: console, page errors, dialogs (auto-dismissed without a listener), load, DOMContentLoaded, close | ✅ | `firefox_page_events` |
| Events: request, response, requestfinished, requestfailed | ✅ | `firefox_network_events_context_routes_and_auth` |
| Events: popup (`context.pages()` keeps pop-ups), websocket (frames both ways, close), worker (`page.workers()`, url, close; worker console messages reach the page) | ✅ | `firefox_popups_workers_and_websockets` |
| `worker.evaluate` | ❌ | Firefox 159's BiDi only runs scripts in window realms; a clear error. Running code in a worker any other way means starting it from a different script URL, which breaks the worker's own relative imports, so it is not done |
| Events: filechooser, frameattached / framedetached | ⬜ | |
| Cookies: `context.cookies`, `addCookies`, `clearCookies` | ✅ | `firefox_cookies_and_init_scripts` |
| `context.addInitScript`, `page.addInitScript` | ✅ | `firefox_cookies_and_init_scripts` |
| `page.route` / `context.route` (fulfill, continue, abort), `unroute`, `unrouteAll` | ✅ | `firefox_routes_requests`, `firefox_network_events_context_routes_and_auth` |
| Route `fetch`, `fallback`, continue / fulfill with overrides, `routeFromHAR`, `routeWebSocket` | ⬜ | |
| Downloads: event, `suggestedFilename`, `url`, `path`, `saveAs`, `failure` | ✅ | `firefox_downloads` |
| Frames: `frameLocator` (nested), clicks and fills inside iframes, `page.frames()`, `frame.evaluate` | ✅ | `firefox_frames` |
| `page.pdf` (format, size, margins, landscape, scale, page ranges, background) | ✅ | `firefox_pdf_and_har` (`browsingContext.print`); `displayHeaderFooter` is an error (BiDi prints none) |
| `context.tracing()` `start` / `stop` (actions, console, network resources, screencast frames) | ✅ | `firefox_tracing` |
| Tracing DOM snapshots (`snapshots: true`, Playwright's snapshot streamer), `startChunk` / `stopChunk`, `group` / `groupEnd` | ✅ | `firefox_trace_snapshots_chunks_and_groups` |
| Clock, aria snapshots, accessibility, `exposeBinding`, element handles | ⬜ | |
| CDP sessions, coverage, `browser.startTracing`, CDP domain clients (section 12) | ❌ | Chromium-only: Firefox does not speak CDP |
| `firefox.connect` (a running Firefox's BiDi endpoint; `close` leaves it running), `launchPersistentContext` (kept profile) | ✅ | `firefox_persistent_context_and_connect` |
| `launchServer` | ⬜ | |

## 14. WebKit (M14)

`chromiumoxide::webkit` drives Playwright's WebKit build (Safari's engine)
over its own protocol through `--inspector-pipe`, with the Playwright method
names. Tests: `tests/integration/webkit.rs`, `tests/integration/webkit_parity.rs`,
against `webkit-2388` (Playwright 1.65.1) on Linux and `webkit-2359` on
Windows. Real Safari needs a Mac (not reachable yet: see the goal).

| Area | WebKit | Test / reason |
|---|---|---|
| `webkit.launch` (Playwright's arguments, `executablePath`, `headless`, `args`, `env`, `timeout`); Linux, macOS and Windows (pipe) | ✅ | `webkit_launches_and_runs_a_page` (Linux, Windows) |
| `browser.version`, `newPage`, `newContext`, `close` | ✅ | `webkit_launches_and_runs_a_page` |
| Context options `viewport`, `deviceScaleFactor`, `screen`, `userAgent` (default: Playwright's Safari 27.2 one), `locale`, `timezoneId`, `geolocation`, `ignoreHTTPSErrors`, `extraHTTPHeaders`, `hasTouch` | ✅ | `webkit_contexts_emulate_devices` |
| `page.goto`, `setContent`, `title`, `url`, `content`, `evaluate`, `addStyleTag`, `setViewportSize`, `waitForFunction`, `waitForTimeout` | ✅ | `webkit_launches_and_runs_a_page`, `webkit_contexts_emulate_devices` |
| `page.screenshot` (viewport, clip, fullPage, JPEG, scale) | ✅ | `webkit_screenshots_are_pixel_identical_to_playwright`: pixel-identical to Playwright's WebKit (desktop, mobile 3x, mockup 2x) |
| Pixelated `<img>` in parity pages | ❌ | Headless WebKit paints it in some runs only, in Playwright's own runs too; left out of the parity cases |
| Locators, input, events, network, storage, frames, recording | ⬜ | Next, as for Firefox |

## 15. Tauri (M14)

`chromiumoxide::Tauri` drives Tauri apps. Test:
`tests/integration/tauri.rs` with the app in `tests/fixtures/tauri-app`.

| System | How | Status |
|---|---|---|
| Windows | WebView2 over CDP: the app passes `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` on to its WebView2 arguments (Tauri sets its own, and WebView2 then ignores the variable); each window is a Chromium `Page` | ✅ see the goal for the Windows run |
| Linux | WebKitGTK through `WebKitWebDriver` (`webkit2gtk-driver`), with `TAURI_WEBVIEW_AUTOMATION=true` like `tauri-driver`; goto, title, evaluate, waits, screenshot, CSS locators (click, inner text) | ✅ `tauri_window_runs_the_page_api` (Xvfb) |
| macOS | ❌ WKWebView has no WebDriver or DevTools interface for embedding apps (Tauri's `tauri-driver` has no macOS support either) | error naming the reason |
