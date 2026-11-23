# Android devices are automatable

| | |
|---|---|
| Status | proposed |
| Owner | Gary Somerhalder |
| Proposed | 2026-11-23 |
| Started | — |
| Finished | — |
| Repos | garysomerhalder/chromiumoxide |
| Environments | Linux container (adb 1.0.41, no device, no emulator: no KVM); a real device or emulator for a manual check |

## Outcome

`chromiumoxide::Android` gives Playwright's `_android` API: device listing
through the adb server, device commands, and Chrome and WebViews on the
device as pages.

## Why

Gary asked for Android (2026-11-23). Chrome on Android and Android WebViews
speak CDP, so the existing page API covers them once they are reachable.

## Scope

In:
- `android.devices({ host, port, omitDriverInstall })`, `android.connect`, `android.launchServer`.
- `AndroidDevice`: `serial`, `model`, `info`, `shell`, `open` (`AndroidSocket`), `push`,
  `installApk`, `screenshot`, `close`, `launchBrowser`, `webViews`, `webView`, event `webview`.
- `AndroidWebView`: `page`, `pid`, `pkg`, event `close`.

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

- [ ] Every in-scope member above has an integration test that passes against the fake adb server.
- [ ] A page from `launchBrowser()` runs `goto`, `evaluate` and `screenshot` in the test.
- [ ] fmt, clippy -D warnings, MSRV 1.88 and the full suite pass, locally and in CI.
- [ ] `docs/PARITY.md` lists the in-scope members as done and the out-of-scope ones with the reason.
- [ ] `CHANGELOG.md` has the entry and a draft PR is open.

## Stop rule

Stop and ask Gary before shipping or downloading an APK, and if a real-device
check is needed that the container cannot do.

## Status

- 2026-11-23: proposed.

## Set it

```
/goal Android devices are automatable (goals/android-devices-are-automatable.md): every in-scope _android member has a passing integration test against the fake adb server; a launchBrowser() page runs goto, evaluate and screenshot; fmt, clippy -D warnings, MSRV 1.88 and the full suite pass locally and in CI; PARITY lists in-scope members done and out-of-scope ones with the reason; CHANGELOG has the entry; a draft PR is open.
```
