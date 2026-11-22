# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.10.0] (in progress)

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
