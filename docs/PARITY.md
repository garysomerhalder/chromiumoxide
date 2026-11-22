# Parity checklists

Legend: ✅ done (tested) · 🟡 partial · ⬜ not started. Milestones: M1 device-mockup
needs, M2 protocol regenerated, M3 navigation/waiting/locators/input, M4 network,
storage, downloads, dialogs, M5 tracing/video/HAR/PDF/assertions, M6 the rest.

Playwright reference: **1.63.0** (`playwright-core/types/types.d.ts`).
Firefox and WebKit are out of scope (this is a CDP client).

### Notes

- **Launch preset.** The default switches are the device-mockup contract
  (`domain::launch`), read from `/proc/<pid>/cmdline` of a real Playwright
  1.63 launch of `chromium_headless_shell-1243` in the container image (that
  install's wrapper adds `--utility-cmd-prefix`, which is left out). A
  `pw:browser` debug log of Playwright's own argument list differs because
  the image's browser wrapper adjusts switches. The parity test gives
  Playwright exactly our command line.
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
| `storageState` | M4 | ✅ cookies + localStorage, JSON-compatible with Playwright files (IndexedDB not yet) |
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
| `bind` | M3 | ➖ Playwright-protocol server; use `ws_endpoint()` / `connect_over_cdp` instead |
| `browserType` | M1 | ✅ |
| `close` | M1 | ✅ |
| `contexts` | M1 | ✅ |
| `isConnected` | M1 | ✅ |
| `newBrowserCDPSession` | M3 | ✅ |
| `newContext` | M1 | ✅ |
| `newPage` | M1 | ✅ |
| `startTracing` | M3 | ✅ |
| `stopTracing` | M3 | ✅ |
| `unbind` | M3 | ➖ Playwright-protocol server; use `ws_endpoint()` / `connect_over_cdp` instead |
| `version` | M1 | ✅ |

### BrowserContext

| Member | Milestone | Status |
|---|---|---|
| `exposeBinding` | M3 | ✅ |
| `addInitScript` | M3 | ✅ |
| `addCookies` | M4 | ✅ |
| `backgroundPages` | M4 | ➖ deprecated in Playwright (always empty with Manifest V3 extensions); not provided |
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
| `clock` | M6 | ✅ |
| `credentials` | M4 | ✅ via Chromium's virtual WebAuthn authenticator |
| `debugger` | M6 | ✅ requestPause, next, resume, pausedDetails (title), events; no source locations / `runTo` |
| `request` | M6 | ✅ |
| `tracing` | M5 | ✅ actions, console, network, screencast (no DOM snapshots yet) |

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
| `frames` | M3 | ✅ incl. out-of-process (cross-site) iframes; routing / init scripts inside those not yet |
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
| `pickLocator` | M6 | 🟡 main frame; cancelled by navigation |
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
| `waitForNavigation` | M3 | 🟡 the main frame's navigation |
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
| `toString` | M3 | 🟡 async `to_playwright_string()` (Playwright's generator); `Debug` prints the selector |
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
| `serviceWorker` | M4 | 🟡 service-worker requests are not tracked yet |
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
| `createReadStream` | M4 | ✅ as `read()` (bytes) |
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
| `tracing` | M6 | 🟡 requests are not traced yet |

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
| `launchServer` | M6 | ✅ `BrowserServer` (CDP endpoint) |
| `name` | M1 | ✅ |

### Selectors

| Member | Milestone | Status |
|---|---|---|
| `register` | M3 | ✅ |
| `setTestIdAttribute` | M3 | ✅ |
