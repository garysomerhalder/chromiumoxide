# Every CDP domain has a tested API

| | |
|---|---|
| Status | proposed |
| Owner | Gary Somerhalder |
| Proposed | 2026-11-23 |
| Started | — |
| Finished | — |
| Repos | garysomerhalder/chromiumoxide |
| Environments | Linux container (Chrome for Testing 159, headless shell 159); CI ubuntu-latest |

## Outcome

Every non-deprecated Chrome DevTools Protocol domain in the vendored
protocol has a typed, documented, tested high-level API in chromiumoxide,
beyond the parts Playwright uses.

## Why

Gary asked for "all CDP" (2026-11-23). Every command can already be sent,
typed, through `chromiumoxide_cdp`, but only 92 of 596 browser commands and
19 of 82 JavaScript commands have a high-level API (audit in
`docs/BACKLOG-PLATFORMS.md`, M15).

## Scope

In:
- The 57 non-deprecated domains of `v0.0.1751165`, in the priority order of
  `docs/BACKLOG-PLATFORMS.md` (priority 1, then 2, then 3).

Out:
- Deprecated domains (Console, Schema): raw commands only.
- Commands that only make sense inside DevTools' own UI; each is listed with
  the reason in PARITY.

## Rules

- Test-first (`CLAUDE.md`); the device-mockup contract stays green.
- One API per domain, on `Page`, `BrowserContext` or `Browser` as the
  domain's target dictates; events as typed streams.
- When the protocol is updated, re-run the coverage audit in the same PR.

## Work

1. A coverage check in the test suite: every non-deprecated command and event
   has an API or an entry in the PARITY exclusion list.
2. Priority 1 domains.
3. Priority 2 domains.
4. Priority 3 domains.

## Evidence

- The coverage check (fails when a domain or command has neither).
- Integration tests per domain against Chrome for Testing.

## Done when

- [ ] The coverage check passes: every non-deprecated command and event has an API or a listed reason.
- [ ] Each domain's API has at least one integration test against real Chromium.
- [ ] fmt, clippy -D warnings, MSRV 1.88 and the full suite pass, locally and in CI.
- [ ] `docs/PARITY.md` has a CDP section per domain; `CHANGELOG.md` has the entries.
- [ ] Draft PRs for the work are open.

## Stop rule

Stop and ask Gary if a domain needs hardware or accounts the container lacks
(e.g. Cast devices, smart cards beyond emulation), or if the protocol is
updated mid-way and the counts change by more than 10%.

## Status

- 2026-11-23: proposed.

## Set it

```
/goal Every CDP domain has a tested API (goals/every-cdp-domain-has-a-tested-api.md): the coverage check passes (every non-deprecated command and event has an API or a listed reason); each domain's API has an integration test against real Chromium; fmt, clippy -D warnings, MSRV 1.88 and the full suite pass locally and in CI; PARITY has a CDP section per domain and CHANGELOG the entries; draft PRs are open.
```
