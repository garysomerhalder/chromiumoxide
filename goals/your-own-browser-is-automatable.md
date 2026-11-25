# Your own browser is automatable through an extension

| | |
|---|---|
| Status | active |
| Owner | Gary Somerhalder |
| Proposed | 2026-11-25 |
| Started | 2026-11-25 |
| Finished | — |
| Repos | garysomerhalder/chromiumoxide |
| Environments | Linux container (Playwright's Chromium and Firefox builds); Gary's Windows PC; the cloud Mac for Safari |

## Outcome

A browser extension (TypeScript, built with WXT, in `extension/`) lets the
library drive the browser Gary already uses, with his tabs and logins, without
starting it in remote-debugging mode: the whole Chromium API on Chrome and
Edge, and the basic page actions on Firefox and Safari.

## Why

Gary asked for it (2026-11-25): browsers only accept automation when started
for it; an extension is the way into the everyday browser. It is for Gary's
own use, so it has no access token.

## Scope

In:
- Chrome and Edge: the extension relays Chrome's debugging protocol
  (`chrome.debugger`) to a local relay the library starts; the library sees
  a normal Chromium (`Chromium::extension_relay`, `Chromium::connect_extension`).
- Firefox: the same relay with the basic page actions (tabs, `goto`,
  `evaluate`, click, fill, screenshot).
- Safari: the same extension built for Safari.

Out:
- Firefox and Safari through the full API (their extensions get no
  debugging access).
- New isolated contexts through the extension: it drives the browser's own
  profile.

## Rules

- Test-first (`CLAUDE.md`).
- The device-mockup contract, Chromium's default discovery, launch and
  rendering do not change.
- Closing the library's connection never closes Gary's browser.

## Work

1. Chrome and Edge through `chrome.debugger`.
2. Firefox basic page actions.
3. Safari build (and a Mac test once the cloud Mac is reachable).

## Evidence

- `tests/integration/extension.rs` against Playwright's Chromium and Firefox
  with the extension loaded.

## Done when

- [x] Through the extension, a Chromium with its own tabs is driven with the page API (existing tabs, new tabs, `goto`, `evaluate`, locators, screenshots), and disconnecting leaves it running.
- [x] Through the extension, Firefox runs `goto`, `evaluate`, click, fill and screenshot on a tab.
- [x] The Safari build is produced; it is tested on a Mac or listed in PARITY as untested with the reason.
- [ ] fmt, clippy -D warnings and the full suite pass; PARITY, CHANGELOG and a README for `extension/` describe it.

## Stop rule

Stop and ask Gary if driving the everyday browser needs a step he must take
by hand each time (beyond loading the extension once).

## Status

- 2026-11-25: proposed and active.
- 2026-11-25: Chrome (whole API) and Firefox (basic page actions) pass
  through the extension. The extension is TypeScript, DDD + hexagonal, with
  an MUI popup in atomic design (Gary's request). Safari's build is made
  (`safari-mv3`) but untested: the cloud Mac is unreachable.

## Set it

```
/goal Your own browser is automatable (goals/your-own-browser-is-automatable.md): through the WXT extension a Chromium with its own tabs is driven with the page API and disconnecting leaves it running; Firefox runs goto, evaluate, click, fill and screenshot through it; the Safari build is produced and tested on a Mac or listed as untested with the reason; fmt, clippy and the full suite pass; PARITY, CHANGELOG and extension/README describe it.
```
