# Android devices are automatable

| | |
|---|---|
| Status | met |
| Owner | Gary Somerhalder |
| Proposed | 2026-11-23 |
| Started | 2026-11-23 |
| Finished | 2026-11-23 |
| Repos | garysomerhalder/chromiumoxide |
| Environments | Linux container (adb 1.0.41, no emulator: no KVM); Gary's Windows PC through the Desktop Bridge (Android SDK, emulator `Galaxy_S26`, Android 37, Rust 1.99) for the real-device check |

## Outcome

`chromiumoxide::Android` gives Playwright's `_android` API: device listing
through the adb server, device commands, and Chrome and WebViews on the
device as pages.

## Why

Gary asked for Android (2026-11-23). Chrome on Android and Android WebViews
speak CDP, so the existing page API covers them once they are reachable.

## Scope

In:
- `android.devices({ host, port, omitDriverInstall })`.
- `AndroidDevice`: `serial`, `model`, `info`, `shell`, `open` (`AndroidSocket`), `push`,
  `installApk`, `screenshot`, `close`, `launchBrowser`, `webViews`, `webView`, event `webview`.
- `AndroidWebView`: `page`, `pid`, `pkg`, event `close`.

Out:
- `android.connect` and `android.launchServer`: they serve and reach a
  device over Playwright's own client-server protocol. Here a remote adb
  server (`host`, `port`) does that job.

Out (until a driver is available):
- UI-automator input (`tap`, `fill`, `press`, `drag`, `fling`, `longTap`, `pinch*`, `scroll`,
  `swipe`, `wait`, `input.*`): Playwright installs its own driver APK for these. Decide
  separately whether to ship one.

## Rules

- Test-first (`CLAUDE.md`); the device-mockup contract stays green.
- Speak the adb server protocol directly (port 5037), like Playwright; no
  `adb` binary needed at run time.
- Tests use a fake adb server that forwards the DevTools socket to a local
  Chromium, because the container has no device or emulator.

## Work

1. adb client (host services, `shell:`, `sync:` push, `localabstract:` sockets).
2. `Android::devices` and `AndroidDevice` basics, with the fake adb server.
3. `launchBrowser` and WebViews over `localabstract:chrome_devtools_remote` /
   `webview_devtools_remote_<pid>`.
4. PARITY, CHANGELOG, draft PR.

## Evidence

- `tests/integration/android.rs` against the fake adb server.
- A manual run against a real device or emulator, recorded in the PR.

## Done when

- [x] Every in-scope member above has an integration test that passes against the fake adb server.
- [x] A page from `launchBrowser()` runs `goto`, `evaluate` and `screenshot` in the test.
- [x] fmt, clippy -D warnings, MSRV 1.88 and the full suite pass locally, and the CI job's commands pass when run locally (GitHub CI skipped at Gary's request).
- [x] `docs/PARITY.md` lists the in-scope members as done and the out-of-scope ones with the reason.
- [x] `CHANGELOG.md` has the entry and a draft PR is open (#11).

## Stop rule

Stop and ask Gary before shipping or downloading an APK, and if a real-device
check is needed that the container cannot do.

## Status

- 2026-11-23: proposed.
- 2026-11-23: GitHub CI skipped at Gary's request: the CI box now asks for the
  CI job's commands run locally; "Set it" changed with it.
- 2026-11-23: active. Scope: `connect` / `launchServer` out (Playwright's own
  protocol); Gary's PC runs the emulator for the real-device check.
- 2026-11-23: real emulator (Gary's PC, Galaxy_S26, Android 17, Chrome 145)
  through `examples/android_device.rs`: devices, model
  (`sdk_gphone64_x86_64`), shell, push + read back, screenshot (1.38 MB PNG)
  all work. `launch_browser` timed out: on a `user` build Chrome ignores
  `/data/local/tmp/chrome-command-line` unless chrome://flags "Enable command
  line on non-rooted devices" is on (Playwright needs the same). Noted in
  PARITY; to re-check once the flag is on.
- 2026-11-23: draft PR #11 open; every box ticked: met. Open follow-up: re-run
  `examples/android_device.rs` with the Chrome flag on.

- 2026-11-23: follow-up done. The chrome://flags option was switched on
  through Chrome's DevTools socket on Gary's emulator, and
  `examples/android_device.rs` passed end to end. `launch_browser` loaded
  the page, with the right title, h1, user agent and a page screenshot. The
  first run showed a bug: a page without `<meta name=viewport>` was
  screenshotted tiled 3x3. Fixed on branch `claude/rewrite-m12-cdp` (PR #12):
  no-viewport screenshots measure the ideal viewport. The re-run gave a
  1080x1944 PNG of the page.

## Set it

```
/goal Android devices are automatable (goals/android-devices-are-automatable.md): every in-scope _android member has a passing integration test against the fake adb server; a launchBrowser() page runs goto, evaluate and screenshot; fmt, clippy -D warnings, MSRV 1.88 and the full suite pass locally, and the CI job's commands pass when run locally; PARITY lists in-scope members done and out-of-scope ones with the reason; CHANGELOG has the entry; a draft PR is open.
```
