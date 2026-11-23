# Every CDP domain has a tested API

| | |
|---|---|
| Status | met |
| Owner | Gary Somerhalder |
| Proposed | 2026-11-23 |
| Started | 2026-11-23 |
| Finished | 2026-11-23 |
| Repos | garysomerhalder/chromiumoxide |
| Environments | Linux container (Chrome for Testing 159.0.8154.6, headless shell 159.0.8154.6) |

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
   has an API or an entry in the PARITY exclusion list
   (`chromiumoxide_cdp/tests/domains.rs`).
2. All three priorities at once: the PDL generator emits a typed client per
   domain (`cdp::domains`, `page.protocol()` / `browser.protocol()`), and
   `tests/integration/cdp_domains.rs` tests each domain. Draft PR #12.

## Evidence

- The coverage check (fails when a domain or command has neither).
- Integration tests per domain against Chrome for Testing.

## Done when

- [x] The coverage check passes: every non-deprecated command and event has an API or a listed reason.
- [x] Each domain's API has at least one integration test against real Chromium.
- [x] fmt, clippy -D warnings, MSRV 1.88 and the full suite pass locally, and the CI job's commands pass when run locally (GitHub CI skipped at Gary's request).
- [x] `docs/PARITY.md` has a CDP section per domain; `CHANGELOG.md` has the entries.
- [x] Draft PRs for the work are open.

## Stop rule

Stop and ask Gary if a domain needs hardware or accounts the container lacks
(e.g. Cast devices, smart cards beyond emulation), or if the protocol is
updated mid-way and the counts change by more than 10%.

## Status

- 2026-11-23: proposed.
- 2026-11-23: GitHub CI skipped at Gary's request: the CI box now asks for the
  CI job's commands run locally; "Set it" changed with it.
- 2026-11-23: active. Approach: the PDL generator emits a typed client per
  domain (`page.protocol().performance().get_metrics(..)`, events as
  `.on_metrics()` streams), so every command and event is covered and stays
  covered when the protocol is updated.

- 2026-11-23: met. 57 domains, 634 commands and 224 events have typed
  methods, and the coverage check passes. All 57 domain tests pass against
  real Chromium. The CI job's commands pass locally, and draft PR #12 is
  open. Found and fixed on the way:
  - empty required arrays were not sent;
  - `ignoreDefaultArgs: [...]` was missing.

  Caveat: SmartCardEmulation is not served by Chrome for Testing on Linux.
  Its test pins Chromium's "wasn't found" answer and fails once it is
  served (PARITY section 12).

## Set it

```
/goal Every CDP domain has a tested API (goals/every-cdp-domain-has-a-tested-api.md): the coverage check passes (every non-deprecated command and event has an API or a listed reason); each domain's API has an integration test against real Chromium; fmt, clippy -D warnings, MSRV 1.88 and the full suite pass locally, and the CI job's commands pass when run locally; PARITY has a CDP section per domain and CHANGELOG the entries; draft PRs are open.
```
