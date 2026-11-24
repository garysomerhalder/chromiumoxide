# Firefox and WebKit offer every Playwright method Chromium does

| | |
|---|---|
| Status | active |
| Owner | Gary Somerhalder |
| Proposed | 2026-11-24 |
| Started | 2026-11-24 |
| Finished | — |
| Repos | garysomerhalder/chromiumoxide |
| Environments | Linux container (Playwright 1.65.1 browsers); Gary's Windows PC; the cloud Mac |

## Outcome

`chromiumoxide::firefox` and `chromiumoxide::webkit` have every public method
of Playwright 1.65.1's classes that Chromium has (419 of 419 by name). Each
works, or fails with an error that names why the engine cannot do it.

## Why

Gary asked for every browser to match Playwright ("do all"). Chromium covers
all 419 methods. On 2026-11-24 Firefox had 251 and WebKit 199.

## Scope

In:
- Playwright's Page, Frame, Locator, FrameLocator, ElementHandle, JSHandle,
  BrowserContext, Browser, Route, Request, Response, Keyboard, Mouse,
  Worker, Download, Dialog, Tracing, Clock and APIRequestContext, for Firefox
  (WebDriver BiDi) and WebKit (Playwright's WebKit build).
- PARITY sections 13 and 14 updated row by row.

Out:
- Real Safari and Tauri on Linux and macOS: WebDriver classic offers only the
  basics (PARITY 15 and 16).
- CDP-only methods (`newCDPSession`, `newBrowserCDPSession`) on Firefox and
  WebKit: they return an error that says the method is Chromium-only.

## Rules

- Test-first: a `test(red):` commit with the failure output, then
  `feat(green):` / `fix(green):` (see `CLAUDE.md`).
- The device-mockup contract and `tests/pixel_parity.rs` stay green. No
  change to Chromium code paths.
- Where the engine's protocol cannot do something, the method returns an
  error that names the reason. PARITY records it as ❌ with the reason.
- Share code between the engines where they share Playwright's injected
  script.

## Work

1. Locator, Page and Frame helpers (state checks, `type`, `tap`,
   `set_checked`, `dispatch_event`, `wait_for_selector`, `wait_for_url`,
   default timeouts, `inner_html`, …).
2. Request and Response details; route `continue`, `fallback`, `fetch`.
3. Context: permissions, geolocation, offline, extra headers, HTTP
   credentials, `expose_function` / `expose_binding`, storage state.
4. JSHandle and ElementHandle; `evaluate_handle`.
5. Clock; ARIA snapshots; locator handlers; API request context.
6. WebKit: tracing, HAR, PDF, workers, mouse and keyboard steps.
7. `routeFromHAR`, `routeWebSocket`, the remaining items.

## Evidence

- `scripts/api_coverage.py <playwright-core>/types/types.d.ts`: methods per
  class and engine.
- `tests/integration/firefox.rs`, `tests/integration/webkit.rs`.

## Done when

- [ ] `scripts/api_coverage.py` reports 419/419 for firefox and webkit.
- [ ] Every new method has an integration test against the real browser.
- [ ] PARITY 13 and 14 list each method as ✅ or ❌ with the reason.
- [ ] fmt, clippy `-D warnings` and the full suite pass on Linux; the full
      suite passes on Windows and macOS.
- [ ] CHANGELOG has the entry; the PR is updated.

## Stop rule

Stop and ask Gary when a method would need a change to Chromium's default
discovery, launch or rendering, or a browser build other than Playwright's.

## Status

- 2026-11-24: active. Baseline by name: chromium 419/419, firefox 251/419,
  webkit 199/419.

## Set it

```
/goal Firefox and WebKit offer every Playwright method Chromium does (goals/firefox-and-webkit-match-the-playwright-api.md): scripts/api_coverage.py reports 419/419 for firefox and webkit; each new method has an integration test; PARITY 13 and 14 updated; fmt, clippy and the full suite pass on Linux, Windows and macOS; CHANGELOG and PR updated.
```
