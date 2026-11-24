# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.10.0] (in progress)

### Milestone 13: Firefox (in progress)

- `chromiumoxide::Firefox::launch` and `chromiumoxide::firefox::{Browser,
  BrowserContext, Page, Locator}`: a second engine over WebDriver BiDi,
  with the Playwright method names. The Chromium engine is unchanged.
- So far:
  - contexts with emulation (viewport, device scale factor, user agent,
    locale, timezone, screen, geolocation, extra headers);
  - `goto`, `set_content`, `evaluate`, `add_style_tag`,
    `set_viewport_size`;
  - screenshots (viewport, clip, full page, JPEG), pixel-identical to
    Playwright's Firefox;
  - locators (`locator`, `get_by_*`, `nth`, reads, visibility, waiting,
    strictness);
  - input (`click`, `hover`, `fill`, `press`, `press_sequentially`,
    `check`, `uncheck`);
  - navigation and waits (`url`, `content`, `reload`, `go_back`,
    `go_forward`, `wait_for_function`, `wait_for_timeout`,
    `locator.wait_for`);
  - web-first assertions through the shared `expect()`;
  - page events (console messages, page errors, dialogs, load, close);
  - cookies and init scripts;
  - request routing (`page.route` and `context.route` with fulfil /
    continue / abort, `unroute`);
  - request / response / requestfinished / requestfailed events;
  - HTTP credentials;
  - downloads;
  - iframes (`frame_locator`, `frames`);
  - storage state (save and restore);
  - more input: `dblclick`, `drag_to`, `select_option`, `set_input_files`,
    `page.mouse()` and `page.keyboard()`;
  - `page.pdf()`, `recordHar`, `recordVideo` and `context.tracing()`
    (actions, console, network, screencast frames; no DOM snapshots yet);
  - pop-ups, workers (events, url, close) and WebSockets;
  - `Firefox::launch_persistent_context` and `Firefox::connect`.
- `worker.evaluate` is an error on Firefox: Firefox 159's BiDi cannot run
  scripts in worker realms yet.
- The video encoder port gains `start_scaled` (Firefox frames are full
  size); Chromium's ffmpeg arguments are unchanged.
- `docs/PARITY.md` section 13: the Firefox column (what works, the test,
  or why not).
- Over BiDi in Firefox 159 there is no command for isMobile, hasTouch or
  colorScheme (among others, listed in PARITY section 13); those context
  options are an error.

### Milestone 12: every CDP domain

- `page.protocol()` and `browser.protocol()`: a typed client for every
  protocol domain, generated from the PDL (`chromiumoxide_cdp::cdp::domains`).
  One async method per command (`protocol().performance().get_metrics(..)`)
  and one typed stream per event (`protocol().network().on_request_will_be_sent()`),
  with the protocol's descriptions as docs: 57 domains, 634 commands, 224
  events. Deprecated items keep their raw types only.
- `chromiumoxide_types::CommandExecutor`: the trait the clients run on, for
  other sessions too.
- A coverage check fails when a command or event has no typed method or a
  domain has no integration test; `tests/integration/cdp_domains.rs` has one
  real-Chromium test per domain.
- `LaunchOptions::ignored_default_args`: Playwright's
  `ignoreDefaultArgs: [...]`, dropping single default switches (e.g.
  `--disable-extensions`); the default set is unchanged.
- Fixed: on Chrome for Android, a page without `<meta name=viewport>`
  (zoomed out to fit the screen) was screenshotted tiled 3x3; with no
  viewport option, screenshots now measure the ideal viewport. Desktop
  screenshots are unchanged. Found on a real emulator, where
  `examples/android_device.rs` now passes end to end.
- Fixed: required arrays were left out of the message when empty (the
  generator skipped them), so Chromium rejected calls such as
  `DOMSnapshot.captureSnapshot` with no computed styles or
  `Network.setCookies([])` as "Invalid parameters".

### Milestone 11: Android

- `Android::devices(AndroidDevicesOptions)` and `AndroidDevice` (Playwright's
  `_android`): shell, sockets, push, APK install, screenshots, Chrome
  (`launch_browser`) and WebViews as browser contexts, over the adb server's
  protocol (new `AdbClient` port). Tested against a fake adb server and,
  with `examples/android_device.rs`, a real emulator. UI-automator input
  (tap, swipe, ...) is not included (it needs Playwright's driver APK).

### Milestone 10: Electron

- `Electron::launch(ElectronLaunchOptions)` and `ElectronApplication`
  (Playwright's `_electron`): windows as pages, the main process through
  `evaluate`, `browser_window`, console, window and close events, `close()`.
  Playwright's Electron loader is vendored. CI runs the test with Electron
  under Xvfb.
- `docs/BACKLOG-PLATFORMS.md` (Firefox, WebKit, Android, Electron, Tauri, all
  of CDP) and the `goals/` folder (one file per goal, `goals/README.md`).

### Milestone 9: Playwright 1.66 alpha

- `ContextOptions::locate_only_visible` (Playwright 1.66's
  `locateOnlyVisible`, built from the alpha of 2026-11-22): locators find
  only visible elements and `first()` / `last()` / `nth()` pick among
  visible ones; explicit visibility filters win; selector shortcuts, `$`
  and element handles are unaffected.
- The pixel-parity test passes on Chromium 159.0.8154.13 (the 1.66 alpha's
  browser).

### Milestone 8: the 1.65.1 backlog (options, events, methods)

- Input `delay` on clicks, presses and typing (locators, shortcuts,
  `Keyboard`, `Mouse`).
- `page.$`, `$$`, `$eval`, `$$eval` on pages, frames and element handles;
  `ContextOptions::strict_selectors`. Selector shortcuts now use the first
  match unless `strict_selectors` (they were always strict).
- Events: page `Crash`, `DialogClosed`, `FrameAttached`, `FrameDetached`,
  `FrameNavigated`; the matching context events plus `Download`,
  `PageLoad`, `PageClose`, `BackgroundPage`; `Browser::events()` with
  `Context` and `Disconnected`.
- `page.close_with(PageCloseOptions { run_before_unload })`.
- `max_retries` on API requests and `route.fetch()`; OPFS in storage state
  (`StorageStateOptions { indexed_db, opfs }`).
- `Debugger::run_to(SourceLocation)`, `PausedDetails::location`.
- `BrowserServer::process()`, `Download::create_read_stream()`.
- `LaunchOptions`: `slow_mo`, `downloads_path`, `traces_dir`,
  `artifacts_dir`, `chromium_sandbox`, `handle_sigint` / `handle_sigterm` /
  `handle_sighup` (opt-in). A context's downloads are deleted when it closes.
- `Chromium::launch_server_with(LaunchServerOptions { host, port, ws_path })`;
  `Browser::bind_with(BindOptions { title, host, port, workspace_dir,
  metadata })` and `Chromium::bound_browsers()`. The bridge serves only its
  (by default unguessable) path.
- `Selectors::register_with(.., SelectorRegisterOptions { content_script })`;
  custom engines run in the page's main world unless they are content scripts.
- `RouteFromHar::update / update_content / update_mode`;
  `Tracing::start_har_with(path, HarOptions { content, mode, resources_dir,
  url_filter })`; attached bodies, minimal mode and `.zip` HARs.
- `ClientCertificate` (`ContextOptions::client_certificates`,
  `ApiRequestOptions::client_certificates`): PEM/DER, encrypted PKCS#8 and
  PFX. New dependencies `p12-keystore` and `pkcs8`.
- `Browser::new_page_with(ContextOptions)`.
- Fixes: the blocked-click log names the element;
  `disconnected` comes after the temporary profile is removed.
- Breaking for struct literals without `..Default::default()`: new fields on
  `LaunchOptions`, `ContextOptions`, `ApiRequestOptions`, `RecordHar`,
  `LaunchSpec` (`chromium_sandbox`), `PausedDetails`, `HttpRequest` and
  `Services` (`signals`); the `CdpBridge` port takes a `BridgeAddress`.

### Milestone 7: Playwright 1.65.1

- Vendored Playwright scripts updated to playwright-core 1.65.1; elements in
  hidden iframes are hidden, actions wait for non-inert elements, unrendered
  text leaves accessible names.
- `aria_snapshot_with(AriaSnapshotOptions { mode, depth, boxes })` (page and
  locator; 'ai' mode merges iframes), `Page::get_by_ref`, `Locator::within`,
  `content_with(ContentOptions { include_shadow })`.
- `page.webmcp()` / `frame.webmcp()` (`tools`, `call_tool`).
- `ApiRequestContext::{cookies, cookies_for, add_cookies, clear_cookies,
  clear_cookies_matching}`; RFC 6265 cookie matching; nameless cookies.
- `LaunchOptions::proxy`, `ContextOptions::proxy`; proxy credentials answer
  only proxy challenges.
- `RecordVideo::fps`, `ScreencastOptions::fps`, VP9 videos (VP8 fallback),
  `ShowActions::style(ActionStyle)`, resting action cursor.
- `NewCredential::sign_count`, `Credential::sign_count`.
- `CdpSessionHandle::send_with_timeout`.
- Default launch switches, device descriptors and trace format from 1.65.1
  (the 1.63 switch list is kept in the tests as `PRESET_1_63`).
- CI runs the pixel-parity test against Playwright 1.65.1.

### Breaking Changes

- Rewrite as DDD + hexagonal architecture with a Playwright-like API
  (see `docs/ARCHITECTURE.md` for how the 0.9 API maps onto the new one).

### Added

- `docs/ARCHITECTURE.md`, `docs/PARITY.md` (Chrome and Playwright checklists), `CLAUDE.md`.
- Milestone 1 (device-mockup needs):
  - Playwright-style browser discovery: explicit path, `MOCKUP_CHROMIUM`/`CHROME`,
    `PLAYWRIGHT_BROWSERS_PATH`, `/opt/pw-browsers`, Playwright caches (headless
    shell first, newest first), Chrome for Testing caches, system Chrome.
  - Launch with Playwright's Chromium switch preset, a unique temporary profile
    per launch (removed on close), over `--remote-debugging-pipe` (default) or
    WebSocket; `Chromium::connect_over_cdp`.
  - `Browser::new_context(ContextOptions)` with viewport, screen, device scale
    factor, mobile, touch, user agent (+ client hints), locale, timezone, colour
    scheme, reduced motion, forced colours, contrast, geolocation, permissions,
    offline, extra headers, HTTP credentials; `device("iPhone 15")` descriptors.
  - `Page::goto` with `WaitUntil::{Commit, DomContentLoaded, Load, NetworkIdle}`,
    `set_content` (waits for load, images, fonts), `wait_for_function`,
    `wait_for_load_state`, `add_style_tag`, `evaluate`/`evaluate_with`,
    `set_viewport_size`, `content`, `title`.
  - `Page::screenshot` (viewport, clip, full page, omit background, css/device
    scale, png/jpeg/webp), pixel-identical to Playwright 1.63 (tested).

- Milestone 2 (protocol):
  - CDP regenerated from ChromeDevTools/devtools-protocol `v0.0.1751165`
    (new domains Ads, CrashReportContext, DigitalCredentials, FindInPage,
    SmartCardEmulation, WebMCP); deprecated domains, commands and fields are
    now generated too (`Console`, `Schema`, ...).
  - `Page::execute(cmd)` / `Browser::execute(cmd)` for any generated command,
    `Page::cdp_events::<E>()` / `Browser::cdp_events::<E>()` for any generated event.

- Milestone 3 (in progress):
  - `reload`, `go_back`, `go_forward`, `wait_for_url` (Playwright globs and
    regexes), `wait_for_timeout`.
  - `add_init_script` (page and context), `add_script_tag`, `expose_function`
    (page and context).
  - `keyboard()`, `mouse()`, `touchscreen()` with Playwright's US layout.
  - Locators: `locator`, `get_by_role/text/label/placeholder/alt_text/title/test_id`,
    `nth/first/last/filter/and/or`, auto-waiting with Playwright's actionability
    checks and strict mode, click/dblclick/hover/tap/fill/clear/press/
    press_sequentially/check/uncheck/select_option/set_input_files/focus/blur/
    dispatch_event/scroll_into_view_if_needed/screenshot, and text/state queries.
    Selector engines run Playwright's injected script (see THIRD_PARTY_NOTICES.md).

- Milestone 4 (in progress):
  - Page events: `page.events()` stream (`PageEvent`), console messages and
    page errors (with history), dialogs (`on_dialog`, accept/dismiss; dismissed
    automatically when unhandled), `wait_for_event` and typed waiters with
    `.run(action)`.
  - Network: `Request` / `Response` objects (redirect chains, raw headers,
    post data, timing, server address, body/text/json), request/response/
    finished/failed events, `wait_for_request` / `wait_for_response`.
  - Routing: `page.route` / `context.route` with `fulfill`, `abort`,
    `continue_`, `fallback`, `times`, `unroute`, `unroute_all`.
  - Cookies and storage: `cookies`, `add_cookies`, `clear_cookies(_matching)`,
    `storage_state` and `ContextOptions::storage_state` (Playwright's JSON
    format); runtime `set_extra_http_headers` (context and page),
    `set_offline`, `set_geolocation`, `grant_permissions`, `clear_permissions`.
  - Popups (`wait_for_popup`, `opener`), context events (`context.events()`,
    `wait_for_page`), downloads (`Download`, `ContextOptions::accept_downloads`),
    WebSockets (`WebSocket`, frames and close) and dedicated workers (`Worker`).

- Milestone 4 (completed): request sizes, HTTP version, service workers and
  `ServiceWorkers::Block`, runtime `set_http_credentials`, `route.fetch()`,
  `fulfill().path()/.response()`, `routeWebSocket`, `page.local_storage()` /
  `session_storage()`, `context.credentials()` (WebAuthn), file chooser,
  `page.requests()`, `set_storage_state`.
- Milestone 5 (completed):
  - `page.pdf()` with Playwright's options.
  - Web-first assertions: `expect(&locator | &page | &response)` with
    Playwright's matchers, retry schedule and failure messages.
  - HAR: `ContextOptions::record_har`, `route_from_har`.
  - Video: `ContextOptions::record_video`, `page.video()` (needs ffmpeg; uses
    Playwright's build when installed).
  - Tracing: `context.tracing().start/stop` writing a Playwright-style trace.zip.
  - `ContextOptions::base_url`, `java_script_enabled`, `bypass_csp` (tested);
    `browser.selectors()` (`set_test_id_attribute`, `register`);
    `locator.aria_snapshot_json()`; `Chromium::launch_persistent_context`;
    request/response `headers_array`/`header_value`, `post_data_json`.
  - `add_script_tag` fails fast when the page's CSP blocks it.
  - Screenshot `mask` / `mask_color`; `locator.screenshot_builder()`.
  - `expect(page | locator).to_have_screenshot(path)` with Playwright's
    pixelmatch comparison, update modes and messages (PNG snapshots).
  - `expect(page | locator).to_match_aria_snapshot(template)` and
    `to_match_aria_snapshot_file(path)`.
  - `page.screencast()`: frames, `.webm` recording, overlays, chapters and
    `show_actions` annotations.
- Milestone 3/4 follow-ups: HTML5 drag-and-drop and `locator.drop()`,
  out-of-process (cross-site) iframes, an `ignoreHTTPSErrors` test.
- Milestone 6 (completed):
  - `page.clock()` / `context.clock()` (Playwright's fake timers).
  - `page.coverage()`: JS and CSS coverage.
  - `context.request()`, `page.request()`, `Chromium::request_context()`:
    `APIRequestContext` / `APIResponse` over a small HTTP/1.1 client (adds
    `tokio-rustls` and `rustls-native-certs`); `Services` gains `http`.
  - `context.debugger()`, `page.pause()`, `page.request_gc()`,
    `page.pick_locator()` / `cancel_pick_locator()`.
  - `Chromium::launch_server()` / `Chromium::connect()`; closing a connected
    browser now disconnects (like Playwright) instead of closing it.
  - `APIResponse` `server_addr`, `security_details`, `timing`;
    `request.tracing()`.
  - Context-level events (`request`, `response`, `console`, `dialog`,
    `weberror`, ...) and `request.service_worker()`.
  - `frame.wait_for_navigation()` for child frames; routes, init scripts and
    bindings inside cross-site iframes.
  - Storage state with IndexedDB (`StorageStateOptions`).
  - Trace DOM snapshots (`TracingStart { snapshots: true }`).
  - `Locator` implements `Display` like Playwright's `locator.toString()`.
  - `browser.bind()` / `unbind()`: share a launched browser with other
    clients over a local CDP WebSocket; `Services` gains `bridge`.

### Fixed

- `ContextOptions::permissions` now takes Playwright's permission names
  (`camera`, `clipboard-read`, ...) and maps them to Chromium's, like Playwright;
  unknown names are rejected.
- PDL generator: deprecated event variants produced invalid code; array
  builder setters could collide with real fields (`currentTime`/`currentTimes`).

### Changed

- Version 0.10.0 for all workspace crates; minimum Rust 1.88.
- `Network.InterceptionId` conversions removed (the type left the protocol).
- `ContextOptions` gained `storage_state` and `accept_downloads` (use
  `..Default::default()` when building it); `Response` gained methods and
  private state (its public fields are unchanged).
- `ContextOptions` also gained `service_workers`, `record_har`, `record_video`;
  `Services` gained `video` (an encoder port) and the `Clock` port `wall_ms()`;
  `Error` gained `Assertion`.
- Typed protocol events moved from `page.events::<E>()` to
  `page.cdp_events::<E>()`; `page.events()` now streams Playwright page events.
- Derive `Clone` for `Element`, `ScreenshotParams` and `ScreenshotParamsBuilder`

## [0.9.1] 2026-02-25

### Fixed

- Invalid import when using zip0

## [0.9.0] 2026-02-20

### Breaking Changes

- Removed async-std support

### Changed

- Updated edition to 2024
- Bump fetcher chromium to `r1585606`
- Bump CDP to `r1566079`
- Use a struct `Arg` for arguments to combine flags automatically
- Browser process no longer inherits stdout
- Update `reqwest` to v0.13
- Update `thiserror` to v2
- Update `heck` to v0.5
- Add support for `zip` v8 and make it default

### Added

- Add `add_init_script` to `Page` for scripts before navigation
- Add `click_with` to `Page` and `Element` for custom click behavior

## [0.8.0] 2025-11-28

### Breaking Changes

Due to the support of new browsers in the fetcher, we changed the API.
The changes are mainly related to how you can request a specific version of the browser.
Previously you could only request a chromium revision, now we support chrome versions, channel and milestone.

```rust
BrowserFetcherOptions::builder()
  .with_kind(BrowserKind::Chrome)
  .with_version(BrowserVersion::Channel(Channel::Beta))
  .build()
```

We also change the output format of the `fetch`. It is now called a `BrowserFetcherInstallation`
and contains a `BuildInfo`. We garantee that at least the version or revision will be present
in that struct, but not always both.

```rust
let installation = chromiumoxide_fetcher::BrowserFetcher::new(options).fetch().await?;
println!("Executable path: {}", installation.executable_path.display());
```

Finally, we switched the async runtime tokio by default. We will remove for support for async-std in the next release.
If you want to part of the discussion on other runtime support, see [#273](https://github.com/mattsse/chromiumoxide/issues/273).

### Changed

- Bumped MSRV to 1.85 to support edition 2024
- Update `async-tungstenite` to 0.32.0
- Update `which` to 8.0.0
- Replace `winreg` by `windows-registry`
- Updated PDL to r1519099 (Chromium 142.0.7431.0)
- Updated fetcher to r1520176 (Chromium 142.0.7435.0)
- Fetch now supports `Chrome for testing` and `Chrome Headless Shell`
- Now uses tokio by default

### Added

- Add option to disable automation detection
- Expose the `cmd` module for access to `CommandChain`

### Fixed

- Fixed typo in feature `_fetcher-rustls-tokio`
- More resilient message parsing, it should now not crash on unknown events coming from the browser
- Extensions should only be disabled when no extensions are provided

[Unreleased]: https://github.com/mattsse/chromiumoxide/compare/v0.7.0...HEAD
