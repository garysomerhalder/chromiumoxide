# Electron apps are automatable

| | |
|---|---|
| Status | active |
| Owner | Gary Somerhalder |
| Proposed | 2026-11-23 |
| Started | 2026-11-23 |
| Finished | — |
| Repos | garysomerhalder/chromiumoxide |
| Environments | Linux container (Electron 45.4.1, Xvfb); CI ubuntu-latest (Electron from npm, Xvfb) |

## Outcome

`chromiumoxide::Electron::launch` starts an Electron app and gives
Playwright's `ElectronApplication` API: its windows as pages, the main
process through `evaluate`, and its events.

## Why

Gary asked for Electron (2026-11-23). Electron apps are Chromium inside, so
everything chromiumoxide already does for pages works on their windows once
they are reachable.

## Scope

In:
- `electron.launch({ executablePath, args, cwd, env, timeout, chromiumSandbox, ...context options })`,
  with Playwright's loader when Electron is found like `require('electron')`.
- `ElectronApplication`: `firstWindow`, `windows`, `context`, `evaluate`,
  `evaluateHandle`, `browserWindow`, `process`, `close`, events `window`,
  `console`, `close`.

Out:
- Electron on macOS and Windows CI runners (the code is portable; only Linux
  is tested here).
- Closing a single window through `webContents.close()` (Playwright's
  custom page-close handler); `page.close()` closes the target instead.

## Rules

- Test-first (`CLAUDE.md`).
- Chromium's default discovery, launch and rendering do not change;
  `tests/pixel_parity.rs` stays green.
- Main-process access only through the Node inspector (CDP `Runtime`), as
  Playwright does.

## Work

1. Backlog and scope: `docs/BACKLOG-PLATFORMS.md`, `CLAUDE.md`.
2. `test(red)`: `tests/integration/electron.rs` with the fixture app
   `tests/fixtures/electron/main.js`.
3. `feat(green)`: launcher port `launch_electron`, adapter, `ElectronApplication`.
4. CI installs Electron and Xvfb and requires the test
   (`CHROMIUMOXIDE_REQUIRE_ELECTRON=1`).
5. PARITY, CHANGELOG; draft PR.

## Evidence

- `tests/integration/electron.rs` (passes with Playwright's loader via
  `ELECTRON_OVERRIDE_DIST_PATH`, and with an explicit `ELECTRON_PATH`).
- The CI "Test Integration" job on the PR.

## Done when

- [x] `tests/integration/electron.rs` passes locally, with and without an explicit executable path.
- [ ] CI runs it with `CHROMIUMOXIDE_REQUIRE_ELECTRON=1` and it passes.
- [x] `cargo fmt --check`, `cargo clippy -D warnings`, the 1.88 MSRV check and the full test suite pass.
- [x] `docs/PARITY.md` lists every in-scope `ElectronApplication` member as done.
- [x] `CHANGELOG.md` has the Electron entry.
- [ ] A draft PR into the previous milestone's branch is open.

## Stop rule

Stop and ask Gary if Electron needs anything that changes Chromium's default
launch, or if CI cannot run Electron (no Xvfb or no npm access).

## Status

- 2026-11-23: proposed and started; the integration test passes locally with
  both launch paths.

## Set it

```
/goal Electron apps are automatable (goals/electron-apps-are-automatable.md): tests/integration/electron.rs passes locally with and without an explicit executable path; CI runs it with CHROMIUMOXIDE_REQUIRE_ELECTRON=1 and passes; fmt, clippy -D warnings, MSRV 1.88 and the full suite pass; PARITY lists every in-scope ElectronApplication member as done; CHANGELOG has the entry; a draft PR is open.
```
