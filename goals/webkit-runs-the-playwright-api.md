# WebKit runs the Playwright API

| | |
|---|---|
| Status | active |
| Owner | Gary Somerhalder |
| Proposed | 2026-11-23 |
| Started | 2026-11-24 |
| Finished | — |
| Repos | garysomerhalder/chromiumoxide |
| Environments | Linux container (Playwright's WebKit 27.2, `webkit-2388`, GTK and WPE); CI ubuntu-latest |

## Outcome

`chromiumoxide::WebKit::launch` runs the same Playwright API on Playwright's
WebKit (Safari's engine), with screenshots pixel-identical to Playwright's
WebKit.

## Why

Gary asked for Safari (2026-11-23). Playwright's WebKit is the closest to
Safari that runs on Linux.

## Scope

In:
- Rust types generated from the WebKit build's `protocol.json` (a second
  input to the protocol generator).
- `WebKit::launch` over `--inspector-pipe`; the device-mockup subset, then the
  whole API.

Out:
- Real Safari on macOS (Safari has no automation protocol for this; only
  WebDriver classic through `safaridriver`).
- Chromium-only features; each listed in PARITY with the reason.

## Rules

- Test-first (`CLAUDE.md`).
- Built on the engine port from the Firefox goal; Chromium unchanged.

## Work

1. Generator input for `protocol.json`; a `chromiumoxide_webkit` protocol crate.
2. Pipe transport and `WebKit::launch`.
3. Device-mockup subset with pixel parity against Playwright's WebKit.
4. The rest of the API.
5. PARITY WebKit column, CHANGELOG, draft PRs.

## Evidence

- The integration suite run against WebKit.
- A WebKit pixel-parity test against Playwright 1.65.1's `webkit.launch()`.

## Done when

- [ ] The WebKit protocol crate is generated from `protocol.json` by the generator.
- [ ] The integration suite runs against WebKit; every test passes or is listed in PARITY as not applicable with the reason.
- [ ] The WebKit pixel-parity test passes against Playwright's WebKit.
- [ ] fmt, clippy -D warnings, MSRV 1.88 and the full suite pass, locally and in CI.
- [ ] `docs/PARITY.md` has a WebKit column; `CHANGELOG.md` has the entries; draft PRs are open.

## Stop rule

Stop and ask Gary if the Firefox goal's engine port is not merged yet, or if
the WebKit build needs system libraries CI cannot install.

## Status

- 2026-11-23: proposed.
- 2026-11-24: active. `chromiumoxide::WebKit` drives Playwright's WebKit
  build over its own protocol (`--inspector-pipe`): contexts with emulation,
  goto, set_content, evaluate, style tags, resize, waits and screenshots.
  The WebKit pixel-parity test passes: every shot matches Playwright's
  WebKit, except a pixelated `<img>` that headless WebKit paints only in
  some runs (Playwright's too), left out of the cases. Runs on Linux and
  Windows (pipe on the C runtime's fds 3 and 4). Real Safari needs a Mac:
  the Daytona key given for the cloud Mac is rejected ("Invalid
  credentials"). Next: locators, input, events, network, as for Firefox.
- 2026-11-24: locators and input; reload, back, forward; context cookies;
  init scripts (with Playwright's Safari shims); console, page errors and
  dialogs; `page.route` (fulfill, continue, abort); network events. All
  WebKit tests pass on Linux; locators and parity also on Windows. Next:
  context routes, frames, `expect`, recording.
- 2026-11-24: context routes, `expect`, frames, pop-ups, downloads and
  `recordVideo`; 15 WebKit tests. The full suite passes on Windows. The
  parity cases also leave out `autofocus`: its focus ring races in
  Playwright's own WebKit shots under load. Next: tracing, HAR, workers;
  real Safari needs the Mac.

## Set it

```
/goal WebKit runs the Playwright API (goals/webkit-runs-the-playwright-api.md): the WebKit protocol crate is generated from protocol.json; the integration suite runs against WebKit with every test passing or listed as not applicable with the reason; the WebKit pixel-parity test passes against Playwright's WebKit; fmt, clippy -D warnings, MSRV 1.88 and the full suite pass locally and in CI; PARITY has a WebKit column, CHANGELOG the entries, and draft PRs are open.
```
