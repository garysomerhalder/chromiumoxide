# Architecture

chromiumoxide 0.10 is a Chrome DevTools Protocol (CDP) client whose public API
follows Playwright (Chromium flavour). It is organised as Domain-Driven Design
plus hexagonal (ports and adapters) architecture, and built test-first.

```
          +-------------------------------------------------------+
          |                      adapters                         |
          |  cdp::Connection   transport::{Pipe, WebSocket}       |
          |  process::TokioLauncher   fs::OsFileSystem  env::Os   |
          |  clock::TokioClock   chromiumoxide_cdp (generated)    |
          |  chromiumoxide_fetcher                                |
          |   +-----------------------------------------------+   |
          |   |                application                    |   |
          |   |  Browser  BrowserContext  Page  Frame         |   |
          |   |  (use cases: launch, new_context, goto, ...)  |   |
          |   |   +---------------------------------------+   |   |
          |   |   |              ports                    |   |   |
          |   |   |  Transport  CdpSession  ProcessLauncher|  |   |
          |   |   |  FileSystem  Environment  Clock        |  |   |
          |   |   |   +-------------------------------+   |   |   |
          |   |   |   |           domain              |   |   |   |
          |   |   |   | launch args, discovery rules, |   |   |   |
          |   |   |   | emulation, devices, frames &  |   |   |   |
          |   |   |   | lifecycle, waiting, selectors,|   |   |   |
          |   |   |   | screenshot geometry, errors   |   |   |   |
          |   |   |   +-------------------------------+   |   |   |
          |   |   +---------------------------------------+   |   |
          |   +-----------------------------------------------+   |
          +-------------------------------------------------------+
                      dependencies point inward only
```

## Layers

| Layer | Module | May depend on | Contents |
|---|---|---|---|
| Domain | `src/domain` | `std`, `serde`, `thiserror` | Pure model and rules. No I/O, no async runtime, no protocol crate. |
| Ports | `src/ports` | domain, `futures` | Traits the application needs from the outside world. |
| Application | `src/application` | domain, ports, `chromiumoxide_cdp` (data types only) | Use cases; this is the public Playwright-like API. |
| Adapters | `src/adapters` | everything | Implementations of ports: CDP connection, transports, process launcher, OS file system and environment, clock. |
| Composition root | `src/lib.rs` | everything | Wires adapters into the application (`chromiumoxide::launch`, `connect`). |

The generated protocol crate `chromiumoxide_cdp` is a *published language*:
plain data types (`Serialize`/`Deserialize`) for every CDP command and event,
with no I/O. The application uses them to build typed commands, and sends them
through the `CdpSession` port, so it never touches a socket or a process.

`tests/architecture.rs` scans the sources and fails if a domain file imports
tokio, the protocol crate, ports, application or adapters, or if ports or
application import adapters.

## Workspace layout

| Crate | Role |
|---|---|
| `chromiumoxide` | Domain, ports, application, adapters, composition root. |
| `chromiumoxide_pdl` | PDL parser and Rust code generator (kept as its own crate). |
| `chromiumoxide_types` | Shared kernel: `Command`, `Method`, `Event` traits used by generated code. |
| `chromiumoxide_cdp` | Generated protocol from `browser_protocol.pdl` and `js_protocol.pdl`. |
| `chromiumoxide_fetcher` | Downloads Chrome for Testing / chrome-headless-shell (adapter). |

## Domain model

- `launch` – `LaunchOptions` → exact Chromium argument list (Playwright preset),
  headless mode per binary kind, unique temporary `--user-data-dir`.
- `discovery` – ordered candidate rules (explicit path, `MOCKUP_CHROMIUM` /
  `CHROME`, `PLAYWRIGHT_BROWSERS_PATH`, `/opt/pw-browsers`, `~/.cache/ms-playwright`,
  `~/Library/Caches/ms-playwright`, Chrome for Testing, system Chrome), evaluated
  over a `FileSystem`/`Environment` snapshot.
- `emulation` – `ContextOptions` (viewport, screen, deviceScaleFactor, isMobile,
  hasTouch, userAgent, locale, timezone, geolocation, permissions, colorScheme,
  reducedMotion, forcedColors, contrast, offline, extraHTTPHeaders, httpCredentials),
  and the derived CDP-agnostic overrides (device metrics, screen orientation,
  user-agent metadata, emulated media features).
- `devices` – Playwright's device descriptors.
- `frame` – frame tree, navigation and lifecycle state machine (commit,
  domcontentloaded, load, networkidle).
- `waiting` – `WaitUntil`, timeouts, polling (`raf` / interval), actionability rules.
- `screenshot` – Playwright's screenshot geometry (viewport, clip, fullPage,
  scale css/device, trimming) as pure functions.
- `selectors` – selector engine model (css, text, role, …) for locators.
- `error` – the error taxonomy (`Timeout`, `TargetClosed`, `Navigation`, `Evaluation`, …).

## Ports

| Port | Purpose | Production adapter | Test adapter |
|---|---|---|---|
| `Transport` | send/receive raw CDP JSON messages | `PipeTransport` (fd 3/4, `--remote-debugging-pipe`), `WebSocketTransport` | in-memory channel pair |
| `CdpSession` | request/response + event stream, optional flat `sessionId` | `cdp::Connection` over any `Transport` | scripted `FakeCdp` |
| `ProcessLauncher` | spawn the browser with pipes, kill/wait | `TokioLauncher` | — (integration only) |
| `FileSystem` | existence, executability, directory listing, temp dirs | `OsFileSystem` | `MemoryFileSystem` |
| `Environment` | env vars, home dir, OS | `OsEnvironment` | `MapEnvironment` |
| `Clock` | sleep / now, for timeouts and polling | `TokioClock` | tokio paused time |

## Application (public API)

```rust
use chromiumoxide::{Chromium, LaunchOptions, ContextOptions, Viewport, WaitUntil};

let browser = Chromium::launch(LaunchOptions::default()).await?;
let context = browser.new_context(ContextOptions {
    viewport: Some(Viewport::new(390, 844)),
    device_scale_factor: Some(3.0),
    ..Default::default()
}).await?;
let page = context.new_page().await?;
page.goto("https://example.com").wait_until(WaitUntil::NetworkIdle).await?;
let png = page.screenshot().full_page(true).await?;
browser.close().await?;
```

Option-taking calls return builders that implement `IntoFuture`, so the
Playwright defaults apply unless you chain an option.

## Testing strategy

- Domain: plain unit tests next to the code (`#[cfg(test)]`).
- Application: tests in `src/application/**/tests.rs` run use cases against the
  scripted `FakeCdp` and in-memory file system / environment, asserting the exact
  CDP traffic (compared with Playwright's recorded protocol logs).
- Integration: `tests/integration/*` against a real Chromium found by discovery,
  including `pixel_parity`, which renders identical HTML with this crate and with
  Playwright and compares the PNGs pixel by pixel.

## Mapping from the 0.9 API

| 0.9 (`chromiumoxide`) | 0.10 |
|---|---|
| `Browser::launch(BrowserConfig)` + spawning the `Handler` stream | `Chromium::launch(LaunchOptions)`; the event loop is internal |
| `BrowserConfig::builder().chrome_executable(p)` | `LaunchOptions { executable_path: Some(p), .. }` |
| `BrowserConfig::with_head()` | `LaunchOptions { headless: false, .. }` |
| `Browser::connect(ws_url)` | `Chromium::connect_over_cdp(ws_url)` |
| `browser.new_page(url)` | `browser.new_page()` / `context.new_page()` then `page.goto(url)` |
| `browser.create_browser_context(..)` | `browser.new_context(ContextOptions)` |
| `page.goto(url)` (waits for load) | `page.goto(url)` (default `WaitUntil::Load`) |
| `page.wait_for_navigation()` | `page.wait_for_load_state(..)` / `page.wait_for_url(..)` |
| `page.set_content(html)` | `page.set_content(html)` (waits for load and fonts) |
| `page.evaluate(expr)` → `EvaluationResult` | `page.evaluate::<T>(expr)` → `T: DeserializeOwned` |
| `page.find_element(sel)` → `Element` | `page.locator(sel)` → `Locator` (auto-waiting) |
| `element.click()` / `type_str` | `locator.click()` / `fill` / `type_text` / `press` |
| `page.screenshot(ScreenshotParams)` | `page.screenshot()` builder (Playwright semantics) |
| `page.pdf(PrintToPdfParams)` | `page.pdf()` builder |
| `page.execute(cmd)` (raw CDP) | `page.cdp().execute(cmd)` (escape hatch, still typed) |
| `page.event_listener::<E>()` | `page.cdp().events::<E>()` and high-level `page.on_*` streams |
| `chromiumoxide::cdp::*` | unchanged: `chromiumoxide::cdp::*` re-exports the generated protocol |
| `chromiumoxide_fetcher` | unchanged crate, used by discovery as a fallback |
