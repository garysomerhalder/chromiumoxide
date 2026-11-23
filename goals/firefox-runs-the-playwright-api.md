# Firefox runs the Playwright API

| | |
|---|---|
| Status | active |
| Owner | Gary Somerhalder |
| Proposed | 2026-11-23 |
| Started | 2026-11-23 |
| Finished | — |
| Repos | garysomerhalder/chromiumoxide |
| Environments | Linux container (Playwright's Firefox 159, `firefox-1572`, WebDriver BiDi verified) |

## Outcome

`chromiumoxide::Firefox::launch` runs the same Playwright API as Chromium on
Firefox, over WebDriver BiDi, with screenshots pixel-identical to
Playwright's Firefox.

## Why

Gary asked for Firefox (2026-11-23). Cross-browser checks need it.

## Scope

In:
- A Firefox engine (`chromiumoxide::firefox`) next to the Chromium one:
  its own `Browser`, `BrowserContext`, `Page`, `Locator`, ... with the same
  method names, over WebDriver BiDi, sharing the domain layer (selectors,
  screenshot maths, keyboard layouts, emulation, launch presets).
- `Firefox::launch`, `launch_persistent_context`, `connect`; the
  device-mockup subset first, then the whole API.

Out:
- Chromium-only features (CDP sessions, coverage, tracing of the browser,
  PDF where Firefox lacks it); each listed in PARITY with the reason.
- Juggler (Playwright's own Firefox protocol), unless BiDi lacks something
  the API needs.

## Rules

- Test-first (`CLAUDE.md`).
- The Chromium engine's code does not change for Firefox: the full Chromium
  suite and the pixel-parity test stay green at every commit.
- Chromium's default discovery, launch and rendering do not change.

## Work

1. BiDi transport and session (port + WebSocket adapter); `Firefox::launch`
   with Playwright's Firefox arguments.
2. (was: the engine-port refactor; replaced, see Status 2026-11-23.)
3. Device-mockup subset: contexts, emulation, goto/setContent, evaluate,
   screenshots; pixel parity with Playwright's Firefox.
4. Locators, input, assertions, network, storage, downloads, dialogs.
5. PARITY Firefox column, CHANGELOG, draft PRs.

## Evidence

- The integration suite run against Firefox.
- A Firefox pixel-parity test against Playwright 1.65.1's `firefox.launch()`.

## Done when

- [ ] `chromiumoxide::firefox` has the Playwright API; every member has a passing Firefox integration test or is listed in PARITY's Firefox column with the reason; the Chromium suite and pixel parity stay green.
- [ ] The Firefox pixel-parity test passes against Playwright's Firefox.
- [ ] fmt, clippy -D warnings, MSRV 1.88 and the full suite pass locally, and the CI job's commands pass when run locally (GitHub CI skipped at Gary's request).
- [ ] `docs/PARITY.md` has a Firefox column; `CHANGELOG.md` has the entries; draft PRs are open.

## Stop rule

Stop and ask Gary if the refactor would change any Chromium behaviour, or if
pixel parity with Playwright's Firefox is impossible over BiDi (Playwright
uses Juggler).

## Status

- 2026-11-23: proposed.
- 2026-11-23: active. Design change: no engine-port refactor of the Chromium
  code. The application layer speaks CDP in 155 places across 37 files.
  Moving all of it behind a port would rewrite most of the 22,000 lines
  device-mockup depends on, for no gain to Chromium. Firefox gets its own
  engine module over WebDriver BiDi instead, with the same API names. It
  shares the pure domain layer, and the Chromium code stays untouched. The
  first two "Done when" lines changed with it, and the CI line follows
  Gary's "skip GitHub". "Set it" changed in the same commit.

## Set it

```
/goal Firefox runs the Playwright API (goals/firefox-runs-the-playwright-api.md): chromiumoxide::firefox has the Playwright API, every member with a passing Firefox integration test or listed in PARITY's Firefox column with the reason, the Chromium suite and pixel parity green; the Firefox pixel-parity test passes against Playwright's Firefox; fmt, clippy -D warnings, MSRV 1.88 and the full suite pass locally, and the CI job's commands pass when run locally; PARITY has a Firefox column, CHANGELOG the entries, and draft PRs are open.
```
