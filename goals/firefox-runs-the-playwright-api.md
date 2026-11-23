# Firefox runs the Playwright API

| | |
|---|---|
| Status | proposed |
| Owner | Gary Somerhalder |
| Proposed | 2026-11-23 |
| Started | — |
| Finished | — |
| Repos | garysomerhalder/chromiumoxide |
| Environments | Linux container (Playwright's Firefox 159, `firefox-1572`, WebDriver BiDi verified); CI ubuntu-latest |

## Outcome

`chromiumoxide::Firefox::launch` runs the same Playwright API as Chromium on
Firefox, over WebDriver BiDi, with screenshots pixel-identical to
Playwright's Firefox.

## Why

Gary asked for Firefox (2026-11-23). Cross-browser checks need it.

## Scope

In:
- The engine port refactor (`docs/BACKLOG-PLATFORMS.md`), with no behaviour
  change for Chromium.
- `Firefox::launch`, `launch_persistent_context`, `connect`; the
  device-mockup subset first, then the whole API.

Out:
- Chromium-only features (CDP sessions, coverage, tracing of the browser,
  PDF where Firefox lacks it); each listed in PARITY with the reason.
- Juggler (Playwright's own Firefox protocol), unless BiDi lacks something
  the API needs.

## Rules

- Test-first (`CLAUDE.md`).
- The refactor lands separately, with the full Chromium suite and the
  pixel-parity test green at every commit.
- Chromium's default discovery, launch and rendering do not change.

## Work

1. Engine port; move the CDP code behind it as the Chromium adapter.
2. BiDi transport and session; `Firefox::launch`.
3. Device-mockup subset: contexts, emulation, goto/setContent, evaluate,
   screenshots; pixel parity with Playwright's Firefox.
4. Locators, input, assertions, network, storage, downloads, dialogs.
5. PARITY Firefox column, CHANGELOG, draft PRs.

## Evidence

- The integration suite run against Firefox.
- A Firefox pixel-parity test against Playwright 1.65.1's `firefox.launch()`.

## Done when

- [ ] The engine-port refactor is merged with the Chromium suite and pixel parity green.
- [ ] The integration suite runs against Firefox; every test passes or is listed in PARITY as Chromium-only with the reason.
- [ ] The Firefox pixel-parity test passes against Playwright's Firefox.
- [ ] fmt, clippy -D warnings, MSRV 1.88 and the full suite pass, locally and in CI.
- [ ] `docs/PARITY.md` has a Firefox column; `CHANGELOG.md` has the entries; draft PRs are open.

## Stop rule

Stop and ask Gary if the refactor would change any Chromium behaviour, or if
pixel parity with Playwright's Firefox is impossible over BiDi (Playwright
uses Juggler).

## Status

- 2026-11-23: proposed.

## Set it

```
/goal Firefox runs the Playwright API (goals/firefox-runs-the-playwright-api.md): the engine-port refactor is merged with the Chromium suite and pixel parity green; the integration suite runs against Firefox with every test passing or listed as Chromium-only with the reason; the Firefox pixel-parity test passes against Playwright's Firefox; fmt, clippy -D warnings, MSRV 1.88 and the full suite pass locally and in CI; PARITY has a Firefox column, CHANGELOG the entries, and draft PRs are open.
```
