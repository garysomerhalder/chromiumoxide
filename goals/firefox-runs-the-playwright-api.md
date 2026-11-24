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
- 2026-11-23: device-mockup subset works on Firefox (draft PR #13):
  - launch, contexts with emulation, `goto`, `set_content`, `evaluate`,
    style tags, resize and screenshots;
  - the Firefox pixel-parity test passes: every shot matches Playwright's
    Firefox exactly.

  Findings:
  - over BiDi, Firefox needs a window to stay alive and opens pages as
    windows;
  - headless Juggler hides scrollbars, so the same rule goes in the
    profile's `userContent.css`;
  - BiDi in Firefox 159 has no command for isMobile, hasTouch or
    colorScheme; these are errors for now.
- 2026-11-24: on Firefox now too, each with a passing test against
  Playwright's Firefox (10 Firefox tests in all):
  - locators;
  - input;
  - navigation and waits;
  - `expect()` assertions;
  - page events (console, errors, dialogs);
  - cookies and init scripts;
  - `page.route`.

  Still to do: `context.route`, request/response events, HTTP
  credentials, downloads, frames, storage state, and the PARITY Firefox
  column.
- 2026-11-24: added on Firefox, each with a passing test (14 Firefox tests
  in all):
  - `context.route`;
  - request / response events;
  - HTTP credentials;
  - downloads;
  - iframes;
  - storage state.

  `docs/PARITY.md` section 13 is the Firefox column. It lists what still
  is ⬜ on Firefox: element handles, `dblclick` / drag / `select_option` /
  file inputs, raw mouse and keyboard, PDF, video, tracing, HAR,
  popups, workers, WebSockets, `launch_persistent_context`, `connect`.
- 2026-11-24: added on Firefox, each with a passing test (20 Firefox tests
  in all):
  - `dblclick`, `drag_to`, `select_option`, `set_input_files`,
    `page.mouse()`, `page.keyboard()`;
  - `page.pdf()`, HAR recording, video recording, tracing;
  - pop-ups, workers, WebSockets;
  - `launch_persistent_context`, `connect`.

  Findings:
  - Firefox's WebDriver actions start a drag but never send `drop`; the
    library finishes the drop in the page.
  - Firefox 159 cannot run code inside workers over BiDi, so
    `worker.evaluate` is a clear error (PARITY ❌).
  - Firefox screenshots are full size, so videos are scaled by ffmpeg
    (new `start_scaled` on the encoder port; Chromium's path unchanged).

  Still ⬜ in PARITY section 13: element handles, clock, aria snapshots,
  tracing DOM snapshots, some locator and route extras, `launchServer`.
- 2026-11-24: trace DOM snapshots, chunks and groups on Firefox (21
  Firefox tests). Drag and drop: Firefox's `remote.events.async.mouse`
  pref was tried and ends the drag without a drop too, so the in-page
  drop stays. `worker.evaluate` stays ❌: the only way round BiDi would
  start workers from another script URL and break their relative imports.

## Set it

```
/goal Firefox runs the Playwright API (goals/firefox-runs-the-playwright-api.md): chromiumoxide::firefox has the Playwright API, every member with a passing Firefox integration test or listed in PARITY's Firefox column with the reason, the Chromium suite and pixel parity green; the Firefox pixel-parity test passes against Playwright's Firefox; fmt, clippy -D warnings, MSRV 1.88 and the full suite pass locally, and the CI job's commands pass when run locally; PARITY has a Firefox column, CHANGELOG the entries, and draft PRs are open.
```
