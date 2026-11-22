# Backlog

What chromiumoxide still lacks for the latest Chrome and the latest Playwright,
found on 2026-11-22. Each item is done test-first, like the milestones in
[`PARITY.md`](PARITY.md); tick it there when it lands.

## Where we stand

| | Latest (2026-11-22) | chromiumoxide |
|---|---|---|
| Chrome stable | 158.0.8127.36 (beta 159.0.8154.13, canary 160.0.8178.0) | tested on Chrome for Testing 159.0.8154.6 and chrome-headless-shell 159 |
| Chrome DevTools Protocol | devtools-protocol `0.0.1751165` (published 2026-11-21, canary level) | vendored `v0.0.1751165`: current |
| Playwright stable | 1.65.1 (Chromium 159.0.8154.6) | 1.65.1 parity: methods in milestone 7 (PR #7), options and events in milestone 8 |
| Playwright next | 1.66.0 alpha of 2026-11-22 (Chromium 159.0.8154.13) | not started (see below) |

Sources:
- the npm registry (`playwright`, `devtools-protocol`);
- the Chrome for Testing "last known good versions" list;
- the Chrome version-history API;
- the Playwright release notes (1.64.0 to 1.65.1);
- a diff of `playwright-core/types/types.d.ts` across 1.63.0, 1.65.1 and the 1.66 alpha.

The audit checked every 1.65.1 method, option and event of the Chromium-relevant
classes against the code. Earlier milestones tracked methods only; the gaps
below are mostly options and events, and almost all already existed in 1.63.

**Status: sections 1 to 4 are done (milestone 8, see
[`PARITY.md`](PARITY.md) section 8).** What is left is section 5, which waits
for the Playwright 1.66 release.

## 1. Options of existing methods (done, M8)

| Item | Playwright API | Notes |
|---|---|---|
| Input delays | `delay` on `click`, `dblclick`, `press`, `type`, `pressSequentially` (Locator, Frame, Page, ElementHandle), `keyboard.press/type`, `mouse.click/dblclick` | Wait between down/up and between keys. |
| Client certificates | `clientCertificates` (`certPath`, `keyPath`, `pfxPath`, `pfx`, `passphrase`, `origin`) on contexts, `newPage`, `launchPersistentContext`, `request.newContext` | Done: the HTTP adapter presents the certificate; a context's requests to that origin go through it (Playwright uses a local proxy). |
| Strict selectors | `strictSelectors` context option | Every selector call is strict. |
| Request retries | `maxRetries` on `request.fetch/get/post/...` and `route.fetch` | Retry on `ECONNRESET`. |
| Request TLS per call | `ignoreHTTPSErrors` on `request.fetch/get/post/...` | Per-request override. |
| OPFS in storage state | `storageState({ opfs })` (context and request) | Origin private file system; the vendored storage script already supports it. |
| HAR replay updates | `routeFromHAR({ update, updateContent, updateMode })` | Record into the HAR while routing. |
| HAR resources | `tracing.startHar({ resourcesDir })` | Bodies as separate files. |
| Page close | `page.close({ runBeforeUnload })` | Run `beforeunload` handlers (then a `dialog` event). |
| Launch tuning | `slowMo`, `downloadsPath`, `tracesDir`, `artifactsDir`, `chromiumSandbox`, `handleSIGINT` / `handleSIGTERM` / `handleSIGHUP` | Done. The default command line is unchanged; the signal handlers are opt-in (a library should not take over Ctrl-C). |
| Browser server path | `launchServer({ wsPath })` | Fixed WebSocket path. |
| Bind workspace | `browser.bind({ workspaceDir })` | |
| Custom selector engines | `selectors.register({ contentScript })` | Run the engine in the main world. |

## 2. Methods (done, M8)

| Item | Playwright API | Notes |
|---|---|---|
| Debugger run-to | `debugger.runTo(location)` | Resume until a source location. |
| Server process | `browserServer.process()` | The browser's process (pid, kill). |
| Download stream | `download.createReadStream()` | A byte stream (Rust: `AsyncRead`). |

## 3. Events (done, M8)

| Item | Playwright API | Notes |
|---|---|---|
| Page events | `crash`, `dialogclosed`, `frameattached`, `framedetached`, `framenavigated` | Add to `PageEvent`. |
| Context events | `dialogclosed`, `download`, `frameattached`, `framedetached`, `framenavigated`, `pageclose`, `pageload`, `backgroundpage` | Add to `ContextEvent`; `backgroundpage` never fires in Chromium (like `backgroundPages()`). |
| Browser events | `context`, `disconnected` | Needs a browser event stream. |

## 4. Bugs found during the audit (done, M8)

| Item | Notes |
|---|---|
| Hit-target message | A blocked click retries with `{"hitTargetDescription":"<body>…</body>"} intercepts pointer events`; Playwright logs `<body>…</body> intercepts pointer events`. |
| Environment note | `CLAUDE.md` still says the image has Playwright 1.63; this session installed 1.65.1 (fresh containers start with 1.63 until the image is updated). Left for Gary to update; not a code change. |

## 5. Next Playwright (1.66, not released yet)

Seen in the 1.66 alpha of 2026-11-22. Wait for the release notes before building.

| Item | Notes |
|---|---|
| `locateOnlyVisible` | New context option (also on `browser.newPage` and `launchPersistentContext`): locators match only visible elements, except where the chain already filters on visibility. |
| Chromium 159.0.8154.13 | A patch build of the same Chromium; re-run the pixel-parity test against it. |
| Test runner `launchOptions` | Test-runner config; out of scope. |

## 6. Out of scope (recorded so they are not re-found)

- Firefox and WebKit, Android and Electron.
- The test runner: reporters, `--shuffle`, test locks, `testProject.default`, Bun, JSX and `toHaveScreenshot.type`.
- `logger` options; Rust code uses the `tracing` crate instead.
- `firefoxUserPrefs`.
- `AbortSignal` options; a Rust future is cancelled by dropping it.
- `Disposable` return values (`await using`); Rust has no async drop, so call the matching stop or remove method.
