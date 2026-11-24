# Tauri apps are automatable

| | |
|---|---|
| Status | done |
| Owner | Gary Somerhalder |
| Proposed | 2026-11-23 |
| Started | 2026-11-24 |
| Finished | 2026-11-24 |
| Repos | garysomerhalder/chromiumoxide |
| Environments | Linux container (WebKitGTK; `webkit2gtk-driver` 2.52 installable); a Windows CI runner for WebView2 |

## Outcome

chromiumoxide drives Tauri apps: on Windows through WebView2 (CDP, the full
Chromium API), on Linux through WebKitGTK.

## Why

Gary asked for Tauri (2026-11-23). Tauri has no browser of its own; it uses
the system web view, so the way in depends on the platform.

## Scope

In:
- `Tauri::launch` on Windows: the app started with
  `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>`,
  connected over CDP.
- Linux: WebKitGTK through `WebKitWebDriver`, or through the WebKit work of
  the WebKit goal where the protocols match.

Out:
- macOS: WKWebView has no supported automation, and Tauri's own `tauri-driver`
  does not support macOS either. Recorded as not possible.

## Rules

- Test-first (`CLAUDE.md`); Chromium unchanged.
- Tests build a minimal Tauri app in the test setup.

## Work

1. Windows (WebView2) launch and connect; Windows CI job.
2. Linux (WebKitGTK): pick WebDriver or the WebKit protocol after the WebKit
   goal; implement and test.
3. PARITY, CHANGELOG, draft PRs.

## Evidence

- `tests/integration/tauri.rs` on Linux; the Windows CI job.

## Done when

- [x] A Tauri app's window runs `goto`, `evaluate`, a locator click and a screenshot on Windows CI.
- [x] The same test passes on Linux (WebKitGTK).
- [x] fmt, clippy -D warnings, MSRV 1.88 and the full suite pass, locally and in CI.
- [x] `docs/PARITY.md` lists Tauri support per platform (macOS as not possible, with the reason).
- [x] `CHANGELOG.md` has the entry and a draft PR is open.

## Stop rule

Stop and ask Gary if a Windows CI runner is not available, or before the
Linux path if the WebKit goal is not met yet.

## Status

- 2026-11-23: proposed.
- 2026-11-24: active. `chromiumoxide::Tauri` with a test app
  (`tests/fixtures/tauri-app`). Linux: WebKitGTK through WebKitWebDriver
  (`TAURI_WEBVIEW_AUTOMATION=true`); the test passes (goto, evaluate, click,
  screenshot). Windows: WebView2 over CDP. Finding: Tauri always sets its
  own WebView2 arguments, and WebView2 then ignores
  `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` (and the per-app policy key), so
  the app has to pass the variable on itself (the test app does). The
  Windows CI runner is Gary's PC (GitHub skipped at his request).
- 2026-11-24: `tauri_window_runs_the_page_api` passes on Gary's PC
  (WebView2 158). Two fixes on the way: WebView2's DevTools answers
  `/json/version` only over HTTP/1.1, and keeps the connection open, so
  the answer is read up to its Content-Length.
- 2026-11-24: the full workspace suite passes on Gary's PC (Windows 11):
  224 unit, 257 integration and every other test crate, Tauri included.

## Set it

```
/goal Tauri apps are automatable (goals/tauri-apps-are-automatable.md): a Tauri app's window runs goto, evaluate, a locator click and a screenshot on Windows CI and on Linux (WebKitGTK); fmt, clippy -D warnings, MSRV 1.88 and the full suite pass locally and in CI; PARITY lists Tauri per platform with macOS as not possible and the reason; CHANGELOG has the entry; a draft PR is open.
```
