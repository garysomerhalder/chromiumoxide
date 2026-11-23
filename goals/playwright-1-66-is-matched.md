# Playwright 1.66 is matched

| | |
|---|---|
| Status | proposed |
| Owner | Gary Somerhalder |
| Proposed | 2026-11-23 |
| Started | — |
| Finished | — |
| Repos | garysomerhalder/chromiumoxide, garysomerhalder/device-mockup (screenshot check) |
| Environments | Linux container; CI ubuntu-latest |

## Outcome

chromiumoxide matches the released Playwright 1.66, not just its alpha.

## Why

`locateOnlyVisible` and the Chromium 159.0.8154.13 check (PR #9) were built
from the 1.66 alpha of 2026-11-22. The release may differ.

## Scope

In:
- Everything in the 1.66 release notes and the `types.d.ts` diff
  1.65.1 → 1.66.0 that applies to Chromium.
- CI and the pixel-parity test on Playwright 1.66 and its Chromium.

Out:
- Test-runner-only changes (as in `docs/BACKLOG.md` section 6).

## Rules

- Test-first (`CLAUDE.md`); device-mockup is told before the default launch
  switches change.

## Work

1. When 1.66.0 is on npm: diff `types.d.ts`, read the release notes.
2. Re-record the `locateOnlyVisible` oracle with 1.66.0; fix differences.
3. Move CI and the parity test to 1.66.0; update PARITY, BACKLOG, CHANGELOG.

## Evidence

- The re-recorded oracle in `src/domain/visible_only.rs`; CI on 1.66.0.

## Done when

- [ ] `npm view playwright version` is 1.66.0 or later.
- [ ] Every Chromium-relevant item of the 1.66 release notes and `types.d.ts` diff is in PARITY as done or out of scope with the reason.
- [ ] The `locateOnlyVisible` cases are re-recorded with the release and pass.
- [ ] CI installs Playwright 1.66 and the pixel-parity test passes.
- [ ] device-mockup was told about any launch-switch change before it landed.

## Stop rule

Not started until 1.66.0 is released. Stop and ask Gary if 1.66 changes the
default launch switches (device-mockup contract).

## Status

- 2026-11-23: proposed; waiting for the 1.66.0 release.

## Set it

```
/goal Playwright 1.66 is matched (goals/playwright-1-66-is-matched.md): Playwright 1.66.0 or later is on npm; every Chromium-relevant item of its release notes and types.d.ts diff is in PARITY as done or out of scope with the reason; the locateOnlyVisible cases are re-recorded with the release and pass; CI installs 1.66 and pixel parity passes; device-mockup was told about any launch-switch change first.
```
