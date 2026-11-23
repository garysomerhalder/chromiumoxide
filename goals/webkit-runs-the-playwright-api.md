# WebKit runs the Playwright API

| | |
|---|---|
| Status | proposed |
| Owner | Gary Somerhalder |
| Proposed | 2026-11-23 |
| Started | — |
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

## Set it

```
/goal WebKit runs the Playwright API (goals/webkit-runs-the-playwright-api.md): the WebKit protocol crate is generated from protocol.json; the integration suite runs against WebKit with every test passing or listed as not applicable with the reason; the WebKit pixel-parity test passes against Playwright's WebKit; fmt, clippy -D warnings, MSRV 1.88 and the full suite pass locally and in CI; PARITY has a WebKit column, CHANGELOG the entries, and draft PRs are open.
```
